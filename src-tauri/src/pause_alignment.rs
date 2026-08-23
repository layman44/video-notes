use crate::{
    ctc_alignment_ffi::{CtcRecognizer, CtcWord},
    punctuation_ffi,
    transcriber::{PauseBoundaryRepair, TranscriptSegment, VadSpeechSegment},
};
use std::{cmp::Ordering as CmpOrdering, path::Path};
use tokio::process::Command;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn hidden_command(program: impl AsRef<Path>) -> Command {
    #[cfg(windows)]
    {
        let mut cmd = Command::new(program.as_ref());
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd
    }
    #[cfg(not(windows))]
    {
        Command::new(program.as_ref())
    }
}

const MIN_VAD_GAP_SECONDS: f64 = 0.20;
const MAX_VAD_GAP_SECONDS: f64 = 3.0;
const WINDOW_BEFORE_SECONDS: f64 = 4.5;
const WINDOW_AFTER_SECONDS: f64 = 4.5;
const EXISTING_BOUNDARY_TOLERANCE: f64 = 0.45;
const MAX_TIME_ERROR_SECONDS: f64 = 0.65;
const MIN_WORD_SIMILARITY: f64 = 0.60;
const MIN_REPAIR_CONFIDENCE: f64 = 0.58;

#[derive(Debug, Clone)]
struct TextSpan {
    start: usize,
    end: usize,
    segment_id: String,
    segment_start: f64,
    segment_end: f64,
    text: String,
}

#[derive(Debug, Clone)]
struct NanoWord {
    norm: String,
    start: usize,
    end: usize,
}

#[derive(Debug, Clone)]
struct PauseCandidate {
    gap_start: f64,
    gap_end: f64,
    gap: f64,
    midpoint: f64,
    approx_offset: usize,
}

#[derive(Debug)]
struct WindowAudio {
    candidate: PauseCandidate,
    window_start: f64,
    samples: Vec<f32>,
    punctuation_hint: String,
}

#[derive(Debug)]
struct CtcScanWindow {
    span: TextSpan,
    window_start: f64,
    samples: Vec<f32>,
    punctuation_hint: String,
}

#[allow(dead_code)]
pub async fn build_selective_pause_repairs(
    ffmpeg: &str,
    video: &str,
    duration: f64,
    segments: &[TranscriptSegment],
    vad_segments: &[VadSpeechSegment],
    dll_path: &Path,
    model_path: &Path,
    tokens_path: &Path,
    punctuation_model_path: &Path,
    threads: usize,
) -> Result<Vec<PauseBoundaryRepair>, String> {
    build_selective_pause_repairs_in_range(
        ffmpeg, video, duration, segments, vad_segments, dll_path, model_path,
        tokens_path, punctuation_model_path, threads, 0.0, duration,
    ).await
}

/// Incremental version used by the streaming Stable Prefix pipeline. `range_start`
/// and `range_end` refer to already-stable acoustic time. Candidate offsets remain
/// global because we still build alignment against the complete stable transcript;
/// only expensive audio windows are filtered to the newly-finalized time range.
pub async fn build_selective_pause_repairs_in_range(
    ffmpeg: &str,
    video: &str,
    duration: f64,
    segments: &[TranscriptSegment],
    vad_segments: &[VadSpeechSegment],
    dll_path: &Path,
    model_path: &Path,
    tokens_path: &Path,
    punctuation_model_path: &Path,
    threads: usize,
    range_start: f64,
    range_end: f64,
) -> Result<Vec<PauseBoundaryRepair>, String> {
    if segments.is_empty() {
        return Ok(Vec::new());
    }

    let (full_text, spans) = build_full_text(segments);
    let all_words = tokenize_words(&full_text);
    if all_words.len() < 4 {
        return Ok(Vec::new());
    }

    let range_start = range_start.clamp(0.0, duration.max(0.0));
    let range_end = range_end.clamp(range_start, duration.max(range_start));
    let candidates = find_selective_candidates(&full_text, &spans, vad_segments)
        .into_iter()
        .filter(|candidate| candidate.midpoint > range_start + 1e-6 && candidate.midpoint <= range_end + 1e-6)
        .collect::<Vec<_>>();
    let scan_spans = eligible_ctc_scan_spans(&full_text, &spans)
        .into_iter()
        .filter(|span| span.segment_end > range_start + 1e-6 && span.segment_end <= range_end + 1e-6)
        .collect::<Vec<_>>();
    if candidates.is_empty() && scan_spans.is_empty() {
        return Ok(Vec::new());
    }

    let mut windows = Vec::new();
    for candidate in candidates {
        let window_start = (candidate.midpoint - WINDOW_BEFORE_SECONDS).max(0.0);
        let window_end = (candidate.midpoint + WINDOW_AFTER_SECONDS).min(duration);
        if window_end - window_start < 3.0 {
            continue;
        }
        let samples = extract_pcm_f32(ffmpeg, video, window_start, window_end - window_start).await?;
        if samples.len() < 16_000 {
            continue;
        }
        windows.push(WindowAudio { candidate, window_start, samples, punctuation_hint: String::new() });
    }

    let mut scan_windows = Vec::new();
    for span in scan_spans {
        let pad = 0.75;
        let window_start = (span.segment_start - pad).max(0.0);
        let window_end = (span.segment_end + pad).min(duration);
        if window_end - window_start < 2.5 {
            continue;
        }
        let samples = extract_pcm_f32(ffmpeg, video, window_start, window_end - window_start).await?;
        if samples.len() < 16_000 {
            continue;
        }
        scan_windows.push(CtcScanWindow {
            span,
            window_start,
            samples,
            punctuation_hint: String::new(),
        });
    }

    if windows.is_empty() && scan_windows.is_empty() {
        return Ok(Vec::new());
    }

    if punctuation_model_path.is_file() {
        let mut hint_inputs = windows
            .iter()
            .map(|window| punctuation_hint_input(&full_text, window.candidate.approx_offset))
            .collect::<Vec<_>>();
        hint_inputs.extend(scan_windows.iter().map(|window| {
            punctuation_hint_input(&full_text, (window.span.start + window.span.end) / 2)
        }));
        if let Ok(hints) = punctuation_ffi::punctuate_batch(dll_path, punctuation_model_path, &hint_inputs) {
            let mut iter = hints.into_iter();
            for window in windows.iter_mut() {
                window.punctuation_hint = iter.next().unwrap_or_default();
            }
            for window in scan_windows.iter_mut() {
                window.punctuation_hint = iter.next().unwrap_or_default();
            }
        }
    }

    let recognizer = CtcRecognizer::new(dll_path, model_path, tokens_path, threads)?;
    let mut repairs = Vec::new();

    // VAD-driven path.
    for window in windows {
        let ctc_words = recognizer.decode_pcm(&window.samples, 16_000)?;
        if ctc_words.len() < 3 {
            continue;
        }
        if let Some(repair) = analyze_pause_window(&full_text, &all_words, &window, &ctc_words) {
            repairs.push(repair);
        }
    }

    // CTC-driven fallback.
    for window in scan_windows {
        let ctc_words = recognizer.decode_pcm(&window.samples, 16_000)?;
        if let Some(repair) = analyze_ctc_scan_window(
            &full_text,
            &all_words,
            vad_segments,
            &window,
            &ctc_words,
        ) {
            repairs.push(repair);
        }
    }

    for repair in &mut repairs {
        if repair.segment_id.is_none() || repair.segment_char_offset.is_none() {
            if let Some((segment_id, local_offset)) = locate_segment_boundary(&spans, repair.boundary_offset) {
                repair.segment_id = Some(segment_id);
                repair.segment_char_offset = Some(local_offset);
            }
        }
        if repair.remove_segment_id.is_none() || repair.remove_segment_char_offset.is_none() {
            if let Some(remove_offset) = repair.remove_punctuation_offset {
                if let Some((segment_id, local_offset)) = locate_segment_boundary(&spans, remove_offset) {
                    repair.remove_segment_id = Some(segment_id);
                    repair.remove_segment_char_offset = Some(local_offset);
                }
            }
        }
    }

    repairs.sort_by(|a, b| a.boundary_offset.cmp(&b.boundary_offset));
    repairs.dedup_by(|a, b| {
        a.boundary_offset.abs_diff(b.boundary_offset) <= 2 || (a.time - b.time).abs() <= 0.20
    });
    Ok(repairs)
}

fn normalize_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn needs_join_space(left: &str, right: &str) -> bool {
    let a = left.chars().rev().find(|c| !c.is_whitespace()).unwrap_or(' ');
    let b = right.chars().find(|c| !c.is_whitespace()).unwrap_or(' ');
    if a.is_whitespace() || b.is_whitespace() {
        return false;
    }
    !matches!(
        b,
        ',' | '.' | ';' | ':' | '!' | '?' | '，' | '。' | '！' | '？' | '；' | '：' | '、' | ')' | ']' | '】' | '》' | '〉'
    ) && !matches!(a, '(' | '[' | '【' | '《' | '〈')
        && a.is_ascii()
        && b.is_ascii()
}

fn build_full_text(segments: &[TranscriptSegment]) -> (String, Vec<TextSpan>) {
    let mut source = segments.to_vec();
    source.sort_by(|a, b| {
        a.start
            .partial_cmp(&b.start)
            .unwrap_or(CmpOrdering::Equal)
            .then_with(|| a.end.partial_cmp(&b.end).unwrap_or(CmpOrdering::Equal))
    });

    let mut full = String::new();
    let mut spans = Vec::new();
    for segment in source {
        let text = normalize_text(&segment.text);
        if text.is_empty() {
            continue;
        }
        if !full.is_empty() && needs_join_space(&full, &text) {
            full.push(' ');
        }
        let start = full.chars().count();
        full.push_str(&text);
        let end = full.chars().count();
        spans.push(TextSpan {
            start,
            end,
            segment_id: segment.id,
            segment_start: segment.start,
            segment_end: segment.end,
            text,
        });
    }
    (full, spans)
}

fn locate_segment_boundary(spans: &[TextSpan], boundary_offset: usize) -> Option<(String, usize)> {
    spans.iter().find_map(|span| {
        if boundary_offset >= span.start && boundary_offset <= span.end {
            Some((span.segment_id.clone(), boundary_offset.saturating_sub(span.start)))
        } else {
            None
        }
    })
}

fn last_visible(text: &str) -> Option<char> {
    text.chars()
        .rev()
        .find(|c| !c.is_whitespace() && !matches!(c, '"' | '\'' | ')' | ']' | '】' | '》'))
}

fn is_strong_sentence_end(c: char) -> bool {
    matches!(c, '.' | '!' | '?' | '。' | '！' | '？')
}

fn existing_sentence_boundary_near(spans: &[TextSpan], time: f64) -> bool {
    spans.iter().any(|span| {
        (span.segment_end - time).abs() <= EXISTING_BOUNDARY_TOLERANCE
            && last_visible(&span.text).is_some_and(is_strong_sentence_end)
    })
}

fn approximate_offset_for_time(spans: &[TextSpan], time: f64) -> Option<usize> {
    if spans.is_empty() {
        return None;
    }
    for span in spans {
        if time >= span.segment_start && time <= span.segment_end {
            let duration = (span.segment_end - span.segment_start).max(0.05);
            let fraction = ((time - span.segment_start) / duration).clamp(0.0, 1.0);
            let length = span.end.saturating_sub(span.start);
            return Some(span.start + ((length as f64 * fraction).round() as usize).min(length));
        }
    }
    None
}

fn strong_punctuation_near(text: &str, offset: usize, radius: usize) -> bool {
    let chars = text.chars().collect::<Vec<_>>();
    let start = offset.saturating_sub(radius).min(chars.len());
    let end = (offset + radius + 1).min(chars.len());
    chars[start..end].iter().copied().any(is_strong_sentence_end)
}

fn find_selective_candidates(
    full_text: &str,
    spans: &[TextSpan],
    vad_segments: &[VadSpeechSegment],
) -> Vec<PauseCandidate> {
    let mut candidates = Vec::new();
    if vad_segments.len() < 2 {
        return candidates;
    }

    for pair in vad_segments.windows(2) {
        let gap_start = pair[0].end;
        let gap_end = pair[1].start;
        let gap = gap_end - gap_start;
        if !(MIN_VAD_GAP_SECONDS..=MAX_VAD_GAP_SECONDS).contains(&gap) {
            continue;
        }
        let midpoint = (gap_start + gap_end) * 0.5;
        if existing_sentence_boundary_near(spans, midpoint) {
            continue;
        }
        let Some(approx_offset) = approximate_offset_for_time(spans, midpoint) else {
            continue;
        };
        if strong_punctuation_near(full_text, approx_offset, 2) {
            continue;
        }
        candidates.push(PauseCandidate {
            gap_start,
            gap_end,
            gap,
            midpoint,
            approx_offset,
        });
    }

    let mut selected = Vec::new();
    for c in candidates {
        if let Some(last) = selected.last_mut() {
            let prev: &PauseCandidate = last;
            if (c.midpoint - prev.midpoint).abs() < 1.20 {
                if c.gap > prev.gap {
                    *last = c;
                }
                continue;
            }
        }
        selected.push(c);
    }

    let total_audio = spans.last().map(|s| s.segment_end).unwrap_or(0.0);
    let budget = ((total_audio / 10.0).ceil() as usize).clamp(12, 90);
    if selected.len() > budget {
        let source = selected;
        let len = source.len();
        let mut covered = Vec::with_capacity(budget);
        for bucket in 0..budget {
            let start = bucket * len / budget;
            let mut end = (bucket + 1) * len / budget;
            if end <= start {
                end = (start + 1).min(len);
            }
            if start >= len {
                break;
            }
            let slice = &source[start..end];
            if let Some(best) = slice.iter().max_by(|a, b| {
                a.gap.partial_cmp(&b.gap).unwrap_or(CmpOrdering::Equal)
            }) {
                covered.push(best.clone());
            }
        }
        covered.sort_by(|a, b| a.midpoint.partial_cmp(&b.midpoint).unwrap_or(CmpOrdering::Equal));
        selected = covered;
    }
    selected
}

fn eligible_ctc_scan_spans(full_text: &str, spans: &[TextSpan]) -> Vec<TextSpan> {
    spans
        .iter()
        .filter(|span| {
            let duration = (span.segment_end - span.segment_start).max(0.0);
            duration >= 2.8
                && ascii_word_count(full_text, span.start, span.end) >= 7
                && span.end > span.start + 12
        })
        .cloned()
        .collect()
}

fn nearest_vad_gap(vad: &[VadSpeechSegment], time: f64) -> Option<(f64, f64)> {
    let mut best: Option<(f64, f64)> = None;
    for pair in vad.windows(2) {
        let gap_start = pair[0].end;
        let gap_end = pair[1].start;
        let gap = (gap_end - gap_start).max(0.0);
        if gap <= 0.0 {
            continue;
        }
        let midpoint = (gap_start + gap_end) * 0.5;
        let error = (midpoint - time).abs();
        if error > 0.75 {
            continue;
        }
        match best {
            Some((_, best_error)) if best_error <= error => {}
            _ => best = Some((gap, error)),
        }
    }
    best
}

fn analyze_ctc_scan_window(
    full_text: &str,
    all_nano: &[NanoWord],
    vad: &[VadSpeechSegment],
    window: &CtcScanWindow,
    ctc: &[CtcWord],
) -> Option<PauseBoundaryRepair> {
    if ctc.len() < 3 {
        return None;
    }

    let mut lo = all_nano
        .iter()
        .position(|word| word.end > window.span.start)
        .unwrap_or(0);
    let mut hi = all_nano
        .iter()
        .rposition(|word| word.start < window.span.end)
        .map(|i| i + 1)
        .unwrap_or(all_nano.len());
    lo = lo.saturating_sub(2);
    hi = (hi + 2).min(all_nano.len());
    if hi <= lo + 3 {
        return None;
    }

    let nano = &all_nano[lo..hi];
    let pairs = align_words(nano, ctc);
    if pairs.len() < 5 {
        return None;
    }

    let mut best: Option<(PauseBoundaryRepair, f64)> = None;

    for adjacent in pairs.windows(2) {
        let (n0, c0, s0) = adjacent[0];
        let (n1, c1, s1) = adjacent[1];
        if n1 != n0 + 1 || c1 <= c0 || s0 < MIN_WORD_SIMILARITY || s1 < MIN_WORD_SIMILARITY {
            continue;
        }
        let global_word = lo + n1;
        if global_word == 0 || global_word >= all_nano.len() {
            continue;
        }
        let left_ctc = &ctc[c0];
        let right_ctc = &ctc[c1];
        let ctc_gap = (right_ctc.start - left_ctc.end).max(0.0);
        let median_gap = local_median_gap(ctc, c0, c1);
        let relative_gap = if median_gap > 0.0 { ctc_gap / median_gap } else { 0.0 };
        let boundary_offset = all_nano[global_word].start;
        let left_norm = &all_nano[global_word - 1].norm;
        let right_norm = &all_nano[global_word].norm;
        let semantic_support = hint_supports_pair(&window.punctuation_hint, left_norm, right_norm);
        let boundary_time = window.window_start + (left_ctc.end + right_ctc.start) * 0.5;
        let vad_support = nearest_vad_gap(vad, boundary_time);
        let remove_punctuation_offset = relocatable_punctuation_offset(full_text, boundary_offset);

        let Some(remove_offset) = remove_punctuation_offset else {
            continue;
        };
        if strong_punctuation_near(full_text, boundary_offset, 2) {
            continue;
        }

        let strong_ctc_gap = ctc_gap >= 0.42 && relative_gap >= 2.0;
        let corroborated_gap = ctc_gap >= 0.28
            && relative_gap >= 2.8
            && (semantic_support || vad_support.is_some());
        let semantic_gap = semantic_support && ctc_gap >= 0.22 && relative_gap >= 2.0;
        if !(strong_ctc_gap || corroborated_gap || semantic_gap) {
            continue;
        }

        let average_sim = (s0 + s1) * 0.5;
        let abs_strength = ((ctc_gap - 0.18) / 0.72).clamp(0.0, 1.0);
        let relative_strength = ((relative_gap - 1.5) / 6.0).clamp(0.0, 1.0);
        let vad_bonus = if vad_support.is_some() { 0.08 } else { 0.0 };
        let semantic_bonus = if semantic_support { 0.10 } else { 0.0 };
        let confidence = (average_sim * 0.42
            + abs_strength * 0.25
            + relative_strength * 0.15
            + vad_bonus
            + semantic_bonus)
            .min(1.0);
        if confidence < 0.58 {
            continue;
        }

        let repair = PauseBoundaryRepair {
            boundary_offset,
            remove_punctuation_offset: Some(remove_offset),
            segment_id: None,
            segment_char_offset: None,
            remove_segment_id: None,
            remove_segment_char_offset: None,
            punctuation_relocation_supported: semantic_support,
            time: boundary_time,
            gap: ctc_gap,
            confidence,
            context: context_around_offset(full_text, boundary_offset, 42),
        };
        match &best {
            Some((_, best_conf)) if *best_conf >= confidence => {}
            _ => {
                best = Some((repair, confidence));
            }
        }
    }

    best.map(|(repair, _)| repair)
}

fn tokenize_words(text: &str) -> Vec<NanoWord> {
    let chars = text.chars().collect::<Vec<_>>();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        while i < chars.len() && !chars[i].is_ascii_alphanumeric() && chars[i] != '\'' {
            i += 1;
        }
        if i >= chars.len() {
            break;
        }
        let start = i;
        let mut raw = String::new();
        while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '\'') {
            raw.push(chars[i].to_ascii_lowercase());
            i += 1;
        }
        let norm = raw.trim_matches('\'').to_string();
        if !norm.is_empty() {
            out.push(NanoWord { norm, start, end: i });
        }
    }
    out
}

fn edit_distance(a: &str, b: &str) -> usize {
    let aa = a.chars().collect::<Vec<_>>();
    let bb = b.chars().collect::<Vec<_>>();
    let mut prev = (0..=bb.len()).collect::<Vec<_>>();
    let mut cur = vec![0usize; bb.len() + 1];
    for (i, ca) in aa.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in bb.iter().enumerate() {
            let sub = prev[j] + usize::from(ca != cb);
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(sub);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[bb.len()]
}

fn similarity(a: &str, b: &str) -> f64 {
    if a == b {
        return 1.0;
    }
    let max_len = a.chars().count().max(b.chars().count()).max(1) as f64;
    1.0 - edit_distance(a, b) as f64 / max_len
}

fn align_words(nano: &[NanoWord], ctc: &[CtcWord]) -> Vec<(usize, usize, f64)> {
    let n = nano.len();
    let m = ctc.len();
    if n == 0 || m == 0 {
        return Vec::new();
    }
    let gap = 0.72f64;
    let mut dp = vec![vec![0.0f64; m + 1]; n + 1];
    let mut bt = vec![vec![0u8; m + 1]; n + 1];
    for i in 1..=n {
        dp[i][0] = i as f64 * gap;
        bt[i][0] = 2;
    }
    for j in 1..=m {
        dp[0][j] = j as f64 * gap;
        bt[0][j] = 3;
    }
    for i in 1..=n {
        for j in 1..=m {
            let sim = similarity(&nano[i - 1].norm, &ctc[j - 1].text);
            let cost_match = 1.0 - sim;
            let m_score = dp[i - 1][j - 1] + cost_match;
            let del_nano = dp[i - 1][j] + gap;
            let ins_ctc = dp[i][j - 1] + gap;
            let mut best_score = m_score;
            let mut best_step = 1u8;
            if del_nano < best_score {
                best_score = del_nano;
                best_step = 2;
            }
            if ins_ctc < best_score {
                best_score = ins_ctc;
                best_step = 3;
            }
            dp[i][j] = best_score;
            bt[i][j] = best_step;
        }
    }
    let mut pairs = Vec::new();
    let mut i = n;
    let mut j = m;
    while i > 0 || j > 0 {
        let step = bt[i][j];
        if step == 1 {
            let sim = similarity(&nano[i - 1].norm, &ctc[j - 1].text);
            pairs.push((i - 1, j - 1, sim));
            i -= 1;
            j -= 1;
        } else if step == 2 {
            i -= 1;
        } else {
            j -= 1;
        }
    }
    pairs.reverse();
    pairs
}

fn punctuation_hint_input(full_text: &str, offset: usize) -> String {
    let chars = full_text.chars().collect::<Vec<_>>();
    let start = offset.saturating_sub(60).min(chars.len());
    let end = (offset + 60).min(chars.len());
    chars[start..end]
        .iter()
        .copied()
        .filter(|c| c.is_ascii_alphanumeric() || c.is_whitespace() || *c == '\'')
        .collect::<String>()
}

#[derive(Debug, Clone)]
struct HintWord {
    norm: String,
    strong_after: bool,
}

fn hint_words(hint: &str) -> Vec<HintWord> {
    let chars = hint.chars().collect::<Vec<_>>();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        while i < chars.len() && !chars[i].is_ascii_alphanumeric() && chars[i] != '\'' {
            i += 1;
        }
        if i >= chars.len() {
            break;
        }
        let mut raw = String::new();
        while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '\'') {
            raw.push(chars[i].to_ascii_lowercase());
            i += 1;
        }
        let norm = raw.trim_matches('\'').to_string();
        if norm.is_empty() {
            continue;
        }
        let mut strong_after = false;
        while i < chars.len() && !chars[i].is_ascii_alphanumeric() {
            if is_strong_sentence_end(chars[i]) {
                strong_after = true;
            }
            i += 1;
        }
        out.push(HintWord { norm, strong_after });
    }
    out
}

fn hint_supports_pair(hint: &str, left: &str, right: &str) -> bool {
    if hint.trim().is_empty() { return false; }
    let words = hint_words(hint);
    for pair in words.windows(2) {
        if !pair[0].strong_after { continue; }
        if similarity(&pair[0].norm, left) >= 0.72 && similarity(&pair[1].norm, right) >= 0.72 {
            return true;
        }
    }
    false
}

fn next_strong_punctuation_after(text: &str, boundary_offset: usize, max_chars: usize) -> Option<usize> {
    let chars = text.chars().collect::<Vec<_>>();
    let end = (boundary_offset + max_chars).min(chars.len());
    for i in boundary_offset..end {
        if is_strong_sentence_end(chars[i]) {
            return Some(i);
        }
    }
    None
}

fn ascii_word_count(text: &str, start: usize, end: usize) -> usize {
    let chars = text.chars().collect::<Vec<_>>();
    let mut count = 0usize;
    let mut in_word = false;
    for ch in chars[start.min(chars.len())..end.min(chars.len())].iter().copied() {
        if ch.is_ascii_alphanumeric() || ch == '\'' {
            if !in_word {
                count += 1;
                in_word = true;
            }
        } else {
            in_word = false;
        }
    }
    count
}

fn has_visible_word_after(text: &str, offset: usize) -> bool {
    text.chars().skip(offset.saturating_add(1)).any(|ch| ch.is_ascii_alphanumeric())
}

fn local_median_gap(ctc: &[CtcWord], c0: usize, c1: usize) -> f64 {
    let lo = c0.saturating_sub(4);
    let hi = (c1 + 5).min(ctc.len());
    let mut gaps = Vec::new();
    for pair in ctc[lo..hi].windows(2) {
        let gap = (pair[1].start - pair[0].end).max(0.0);
        if gap <= 1.2 {
            gaps.push(gap);
        }
    }
    if gaps.is_empty() {
        return 0.06;
    }
    gaps.sort_by(|a, b| a.partial_cmp(b).unwrap_or(CmpOrdering::Equal));
    gaps[gaps.len() / 2].max(0.03)
}

fn relocatable_punctuation_offset(text: &str, boundary_offset: usize) -> Option<usize> {
    let punctuation = next_strong_punctuation_after(text, boundary_offset, 42)?;
    if punctuation <= boundary_offset || !has_visible_word_after(text, punctuation) {
        return None;
    }
    let words = ascii_word_count(text, boundary_offset, punctuation);
    if !(1..=5).contains(&words) {
        return None;
    }
    Some(punctuation)
}

fn context_around_offset(text: &str, offset: usize, radius: usize) -> String {
    let chars = text.chars().collect::<Vec<_>>();
    if chars.is_empty() {
        return String::new();
    }
    let start = offset.saturating_sub(radius).min(chars.len());
    let end = (offset + radius).min(chars.len());
    chars[start..end].iter().copied().collect::<String>()
}

fn analyze_pause_window(
    full_text: &str,
    all_nano: &[NanoWord],
    window: &WindowAudio,
    ctc: &[CtcWord],
) -> Option<PauseBoundaryRepair> {
    let approximate_word = all_nano
        .iter()
        .position(|word| word.end >= window.candidate.approx_offset)
        .unwrap_or_else(|| all_nano.len().saturating_sub(1));
    let lo = approximate_word.saturating_sub(14);
    let hi = (approximate_word + 18).min(all_nano.len());
    if hi <= lo + 3 {
        return None;
    }
    let nano = &all_nano[lo..hi];
    let pairs = align_words(nano, ctc);
    if pairs.len() < 5 {
        return None;
    }

    let gap_start_local = window.candidate.gap_start - window.window_start;
    let gap_end_local = window.candidate.gap_end - window.window_start;
    let gap_mid_local = window.candidate.midpoint - window.window_start;

    let mut best: Option<(usize, f64, f64, bool)> = None;

    for adjacent in pairs.windows(2) {
        let (n0, c0, s0) = adjacent[0];
        let (n1, c1, s1) = adjacent[1];
        if n1 != n0 + 1 || c1 <= c0 {
            continue;
        }
        if s0 < MIN_WORD_SIMILARITY || s1 < MIN_WORD_SIMILARITY {
            continue;
        }
        let left = &ctc[c0];
        let right = &ctc[c1];
        let boundary_time = (left.end + right.start) * 0.5;
        let time_error = (boundary_time - gap_mid_local).abs();
        let global_word = lo + n1;
        if global_word == 0 || global_word >= all_nano.len() {
            continue;
        }
        let boundary_offset = all_nano[global_word].start;
        let left_norm = &all_nano[global_word - 1].norm;
        let right_norm = &all_nano[global_word].norm;
        let ctc_gap = (right.start - left.end).max(0.0);
        let median_gap = local_median_gap(ctc, c0, c1);
        let relative_gap = if median_gap > 0.0 { ctc_gap / median_gap } else { 0.0 };
        let semantic_support = hint_supports_pair(&window.punctuation_hint, left_norm, right_norm);

        if time_error > MAX_TIME_ERROR_SECONDS {
            continue;
        }
        if left.end > gap_end_local + 0.50 || right.start < gap_start_local - 0.50 {
            continue;
        }
        if strong_punctuation_near(full_text, boundary_offset, 2) {
            continue;
        }
        let average_sim = (s0 + s1) * 0.5;
        let gap_strength = ((window.candidate.gap - MIN_VAD_GAP_SECONDS) / 0.70).clamp(0.0, 1.0);
        let time_strength = (1.0 - time_error / MAX_TIME_ERROR_SECONDS).clamp(0.0, 1.0);
        let ctc_bonus = (ctc_gap / 0.45).clamp(0.0, 1.0) * 0.08;
        let acoustically_exceptional = window.candidate.gap >= 0.52
            || (window.candidate.gap >= 0.20 && ctc_gap >= 0.16 && relative_gap >= 2.4);
        if !semantic_support && !acoustically_exceptional {
            continue;
        }
        let semantic_bonus = if semantic_support { 0.12 } else { 0.0 };
        let relative_bonus = ((relative_gap - 1.0) / 4.0).clamp(0.0, 1.0) * 0.10;
        let confidence = (average_sim * 0.40 + gap_strength * 0.20 + time_strength * 0.22 + ctc_bonus + semantic_bonus + relative_bonus).min(1.0);
        if confidence < MIN_REPAIR_CONFIDENCE {
            continue;
        }
        match best {
            Some((_, best_confidence, best_error, _))
                if best_confidence > confidence
                    || ((best_confidence - confidence).abs() < 1e-6 && best_error <= time_error) => {}
            _ => {
                best = Some((boundary_offset, confidence, time_error, semantic_support));
            }
        }
    }

    let (boundary_offset, confidence, _, semantic_support) = best?;
    let snippet_start = boundary_offset.saturating_sub(42);
    let snippet_end = (boundary_offset + 42).min(full_text.chars().count());
    let context = full_text
        .chars()
        .skip(snippet_start)
        .take(snippet_end.saturating_sub(snippet_start))
        .collect::<String>();
    let remove_punctuation_offset = relocatable_punctuation_offset(full_text, boundary_offset);
    Some(PauseBoundaryRepair {
        boundary_offset,
        remove_punctuation_offset,
        segment_id: None,
        segment_char_offset: None,
        remove_segment_id: None,
        remove_segment_char_offset: None,
        punctuation_relocation_supported: semantic_support,
        time: window.candidate.midpoint,
        gap: window.candidate.gap,
        confidence,
        context,
    })
}

async fn extract_pcm_f32(ffmpeg: &str, video: &str, start: f64, duration: f64) -> Result<Vec<f32>, String> {
    let out = hidden_command(ffmpeg)
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-ss")
        .arg(format!("{start:.3}"))
        .arg("-t")
        .arg(format!("{duration:.3}"))
        .arg("-i")
        .arg(video)
        .arg("-vn")
        .arg("-ar")
        .arg("16000")
        .arg("-ac")
        .arg("1")
        .arg("-f")
        .arg("f32le")
        .arg("pipe:1")
        .output()
        .await
        .map_err(|e| format!("无法启动 FFmpeg: {e}"))?;

    if !out.status.success() {
        return Err(format!(
            "FFmpeg 提取选择性 CTC PCM 失败：{}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    if out.stdout.len() % 4 != 0 {
        return Err("FFmpeg f32le 输出长度异常".into());
    }
    let mut samples = Vec::with_capacity(out.stdout.len() / 4);
    for bytes in out.stdout.chunks_exact(4) {
        samples.push(f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]));
    }
    Ok(samples)
}


/// Builds a real English word timeline for the Canonical layer. Unlike the selective
/// pause-repair path, this aligns the complete ASR surface inside every Raw cue and returns
/// RawToken timestamps. Canonical sentence splitting can therefore use punctuation as the
/// linguistic boundary and CTC only as the clock.
pub async fn build_english_alignment_timeline(
    ffmpeg: &str,
    video: &str,
    segments: &[crate::transcript::model::RawSegment],
    dll_path: &Path,
    model_path: &Path,
    tokens_path: &Path,
    threads: usize,
) -> Result<Vec<(String, Vec<crate::transcript::model::RawToken>)>, String> {
    if segments.is_empty() {
        return Ok(Vec::new());
    }

    struct SegmentJob {
        segment_id: String,
        text: String,
        start_ms: u64,
        end_ms: u64,
        window_start: f64,
        nano: Vec<NanoWord>,
        samples: Vec<f32>,
    }

    let mut jobs = Vec::with_capacity(segments.len());
    for seg in segments {
        let nano = tokenize_words(&seg.text);
        if nano.len() < 2 || seg.end_ms <= seg.start_ms {
            jobs.push(SegmentJob {
                segment_id: seg.id.clone(),
                text: seg.text.clone(),
                start_ms: seg.start_ms,
                end_ms: seg.end_ms,
                window_start: 0.0,
                nano: Vec::new(),
                samples: Vec::new(),
            });
            continue;
        }

        let seg_start = seg.start_ms as f64 / 1000.0;
        let seg_end = seg.end_ms as f64 / 1000.0;
        let window_start = (seg_start - 0.25).max(0.0);
        let window_end = seg_end + 0.25;
        let duration = (window_end - window_start).max(0.10);
        let samples = match extract_pcm_f32(ffmpeg, video, window_start, duration).await {
            Ok(v) if !v.is_empty() => v,
            _ => Vec::new(),
        };

        jobs.push(SegmentJob {
            segment_id: seg.id.clone(),
            text: seg.text.clone(),
            start_ms: seg.start_ms,
            end_ms: seg.end_ms,
            window_start,
            nano,
            samples,
        });
    }

    let recognizer = CtcRecognizer::new(dll_path, model_path, tokens_path, threads)?;
    let mut out = Vec::with_capacity(jobs.len());

    for job in jobs {
        if job.samples.is_empty() || job.nano.is_empty() {
            out.push((job.segment_id, Vec::new()));
            continue;
        }

        let ctc = match recognizer.decode_pcm(&job.samples, 16_000) {
            Ok(v) if !v.is_empty() => v,
            _ => {
                out.push((job.segment_id, Vec::new()));
                continue;
            }
        };

        let pairs = align_words(&job.nano, &ctc);
        let accepted = pairs
            .into_iter()
            .filter(|(_, _, sim)| *sim >= 0.50)
            .collect::<Vec<_>>();
        let coverage = accepted.len() as f64 / job.nano.len().max(1) as f64;
        if accepted.len() < 2 || coverage < 0.50 {
            out.push((job.segment_id, Vec::new()));
            continue;
        }

        let chars = job.text.chars().collect::<Vec<_>>();
        let mut raw_tokens = Vec::with_capacity(accepted.len());
        for (nano_index, ctc_index, similarity) in accepted {
            let Some(word) = job.nano.get(nano_index) else { continue };
            let Some(ctc_word) = ctc.get(ctc_index) else { continue };
            if word.start >= word.end || word.end > chars.len() { continue; }
            let surface = chars[word.start..word.end].iter().collect::<String>();
            let start_ms = ((job.window_start + ctc_word.start.max(0.0)) * 1000.0).round() as u64;
            let end_ms = ((job.window_start + ctc_word.end.max(ctc_word.start)) * 1000.0).round() as u64;
            let clamped_start = start_ms.clamp(job.start_ms, job.end_ms);
            let clamped_end = end_ms.clamp(clamped_start, job.end_ms);
            raw_tokens.push(crate::transcript::model::RawToken {
                id: stable_alignment_token_id(&job.segment_id, nano_index, &surface),
                text: surface,
                start_ms: clamped_start,
                end_ms: clamped_end,
                confidence: similarity.clamp(0.0, 1.0) as f32,
            });
        }
        // Preserve ASR lexical order. Clamp tiny CTC timestamp regressions instead of sorting,
        // because Canonical provenance must remain aligned with the original text surface.
        let mut last_start = job.start_ms;
        for token in &mut raw_tokens {
            token.start_ms = token.start_ms.max(last_start);
            token.end_ms = token.end_ms.max(token.start_ms);
            last_start = token.start_ms;
        }
        raw_tokens.dedup_by(|a, b| a.id == b.id);
        out.push((job.segment_id, raw_tokens));
    }
    Ok(out)
}

fn stable_alignment_token_id(segment_id: &str, word_index: usize, text: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in segment_id
        .as_bytes()
        .iter()
        .copied()
        .chain(word_index.to_le_bytes())
        .chain(text.as_bytes().iter().copied())
    {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash & !(1u64 << 63)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(start: f64, end: f64, text: &str) -> TranscriptSegment {
        TranscriptSegment {
            id: format!("test-{start:.1}"),
            start,
            end,
            text: text.into(),
        }
    }

    #[test]
    fn long_pause_inside_sentence_is_candidate() {
        let segments = vec![seg(
            29.0,
            33.7,
            "Yet she refused to buy new ones every morning.",
        )];
        let vad = vec![
            VadSpeechSegment { start: 29.0, end: 31.20 },
            VadSpeechSegment { start: 31.82, end: 33.7 },
        ];
        let (text, spans) = build_full_text(&segments);
        let candidates = find_selective_candidates(&text, &spans, &vad);
        assert_eq!(candidates.len(), 1);
        assert!(candidates[0].gap > 0.6);
    }

    #[test]
    fn short_but_real_pause_is_candidate_for_ctc_review() {
        let segments = vec![seg(
            29.0,
            33.7,
            "Yet she refused to buy new ones every morning. Her breakfast consisted of rice.",
        )];
        let vad = vec![
            VadSpeechSegment { start: 29.0, end: 31.20 },
            VadSpeechSegment { start: 31.48, end: 33.7 },
        ];
        let (text, spans) = build_full_text(&segments);
        let candidates = find_selective_candidates(&text, &spans, &vad);
        assert_eq!(candidates.len(), 1);
        assert!(candidates[0].gap >= 0.20);
    }

    #[test]
    fn relocates_nearby_wrong_period_after_new_boundary() {
        let text = "Yet she refused to buy new ones every morning. Her breakfast consisted of rice.";
        let boundary = text.find("every").unwrap();
        let remove = relocatable_punctuation_offset(text, boundary).expect("expected punctuation relocation");
        assert_eq!(text.chars().nth(remove), Some('.'));
        let before = text.chars().take(remove).collect::<String>();
        assert!(before.ends_with("morning"));
    }

    #[test]
    fn ctc_scan_can_relocate_boundary_without_vad_gap() {
        let segments = vec![seg(
            29.0,
            33.7,
            "Yet she refused to buy new ones every morning. Her breakfast consisted of rice.",
        )];
        let (text, spans) = build_full_text(&segments);
        let words = tokenize_words(&text);
        let span = spans[0].clone();
        let window = CtcScanWindow {
            span,
            window_start: 29.0,
            samples: Vec::new(),
            punctuation_hint: String::new(),
        };
        let raw = [
            ("yet", 0.10, 0.28), ("she", 0.34, 0.48), ("refused", 0.54, 0.86),
            ("to", 0.92, 1.02), ("buy", 1.08, 1.22), ("new", 1.28, 1.42),
            ("ones", 1.48, 1.68),
            ("every", 2.30, 2.50), ("morning", 2.56, 2.88),
            ("her", 2.94, 3.06), ("breakfast", 3.12, 3.44), ("consisted", 3.50, 3.82),
            ("of", 3.88, 3.98), ("rice", 4.04, 4.24),
        ];
        let ctc = raw.iter().map(|(t, s, e)| CtcWord {
            text: (*t).into(), start: *s, end: *e,
        }).collect::<Vec<_>>();
        let repair = analyze_ctc_scan_window(&text, &words, &[], &window, &ctc);
        let repair = repair.expect("CTC scan should relocate the misplaced period");
        assert!(repair.remove_punctuation_offset.is_some());
    }

    #[test]
    fn existing_sentence_boundary_suppresses_candidate() {
        let segments = vec![
            seg(29.0, 31.2, "Yet she refused to buy new ones."),
            seg(31.8, 33.7, "Every morning she counted her coins."),
        ];
        let vad = vec![
            VadSpeechSegment { start: 29.0, end: 31.20 },
            VadSpeechSegment { start: 31.82, end: 33.7 },
        ];
        let (text, spans) = build_full_text(&segments);
        let candidates = find_selective_candidates(&text, &spans, &vad);
        assert!(candidates.is_empty());
    }
}

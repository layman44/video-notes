#![allow(dead_code)]
use crate::transcriber::TranscriptSegment;

#[derive(Debug, Clone, Default)]
pub(crate) struct StitchStats {
    pub trimmed_tokens: usize,
    pub match_tokens: usize,
    pub method: &'static str,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct TailRewriteStats {
    pub removed_previous_tokens: usize,
    pub match_tokens: usize,
    pub method: &'static str,
}

#[derive(Debug, Clone)]
struct TokenRef {
    norm: String,
    segment_index: usize,
    byte_start: usize,
    byte_end: usize,
    ascii_word: bool,
}

fn is_cjk_like(ch: char) -> bool {
    matches!(
        ch as u32,
        0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0x3040..=0x30FF
            | 0x31F0..=0x31FF
            | 0xAC00..=0xD7AF
            | 0x1100..=0x11FF
    )
}

fn tokenize_segment(text: &str, segment_index: usize) -> Vec<TokenRef> {
    let mut tokens = Vec::new();
    let mut ascii_start: Option<usize> = None;
    let mut ascii_end = 0usize;

    let flush_ascii = |tokens: &mut Vec<TokenRef>, start: &mut Option<usize>, end: usize| {
        if let Some(s) = start.take() {
            if end > s {
                let raw = &text[s..end];
                tokens.push(TokenRef {
                    norm: raw.to_ascii_lowercase(),
                    segment_index,
                    byte_start: s,
                    byte_end: end,
                    ascii_word: true,
                });
            }
        }
    };

    for (idx, ch) in text.char_indices() {
        let next = idx + ch.len_utf8();
        if ch.is_ascii_alphanumeric() || ch == '\'' {
            if ascii_start.is_none() {
                ascii_start = Some(idx);
            }
            ascii_end = next;
            continue;
        }

        flush_ascii(&mut tokens, &mut ascii_start, ascii_end);
        if is_cjk_like(ch) {
            tokens.push(TokenRef {
                norm: ch.to_string(),
                segment_index,
                byte_start: idx,
                byte_end: next,
                ascii_word: false,
            });
        } else if ch.is_alphanumeric() {
            tokens.push(TokenRef {
                norm: ch.to_lowercase().collect::<String>(),
                segment_index,
                byte_start: idx,
                byte_end: next,
                ascii_word: false,
            });
        }
    }
    flush_ascii(&mut tokens, &mut ascii_start, ascii_end);
    tokens
}

fn collect_tokens(segments: &[TranscriptSegment]) -> Vec<TokenRef> {
    let mut all = Vec::new();
    for (index, segment) in segments.iter().enumerate() {
        all.extend(tokenize_segment(&segment.text, index));
    }
    all
}

fn min_match_tokens(tokens: &[TokenRef]) -> usize {
    let ascii = tokens.iter().filter(|t| t.ascii_word).count();
    if ascii * 2 >= tokens.len().max(1) {
        // Short English repeats such as "extremely frugal" are common at a
        // 1s overlap boundary. Case and punctuation are already normalized away;
        // allow two exact words only when the phrase carries enough characters to
        // avoid trimming generic pairs such as "a single" or "of the".
        let chars = tokens
            .iter()
            .filter(|t| t.ascii_word)
            .map(|t| t.norm.chars().filter(|c| c.is_ascii_alphanumeric()).count())
            .sum::<usize>();
        if tokens.len() >= 2 && chars >= 10 { 2 } else { 3 }
    } else {
        // CJK tokens are character-like; require a slightly shorter exact run than
        // before so four-character overlap phrases can be removed deterministically.
        4
    }
}

#[derive(Debug, Clone, Copy)]
struct Alignment {
    last_y: usize,
    matched: usize,
    method: &'static str,
}

fn best_contiguous_alignment(x: &[TokenRef], y: &[TokenRef]) -> Option<Alignment> {
    if x.is_empty() || y.is_empty() { return None; }
    let m = x.len();
    let n = y.len();
    let mut prev = vec![0usize; n + 1];
    let mut best: Option<(usize, usize, usize)> = None;

    for i in 1..=m {
        let mut row = vec![0usize; n + 1];
        for j in 1..=n {
            if x[i - 1].norm == y[j - 1].norm {
                row[j] = prev[j - 1] + 1;
                let len = row[j];
                let start_y = j - len;
                let tail_slack = m - i;
                if start_y <= 6 && tail_slack <= 8 {
                    match best {
                        Some((best_len, _, best_end_y))
                            if len < best_len || (len == best_len && j >= best_end_y) => {}
                        _ => best = Some((len, i, j)),
                    }
                }
            }
        }
        prev = row;
    }

    let (len, _end_x, end_y) = best?;
    let matched_slice = &y[end_y - len..end_y];
    if len < min_match_tokens(matched_slice) { return None; }
    Some(Alignment { last_y: end_y - 1, matched: len, method: "contiguous-lcs" })
}

fn lcs_pairs(x: &[TokenRef], y: &[TokenRef]) -> Vec<(usize, usize)> {
    if x.is_empty() || y.is_empty() { return Vec::new(); }
    let m = x.len();
    let n = y.len();
    let mut dp = vec![vec![0u16; n + 1]; m + 1];
    for i in 1..=m {
        for j in 1..=n {
            dp[i][j] = if x[i - 1].norm == y[j - 1].norm {
                dp[i - 1][j - 1] + 1
            } else {
                dp[i - 1][j].max(dp[i][j - 1])
            };
        }
    }
    let mut pairs = Vec::new();
    let (mut i, mut j) = (m, n);
    while i > 0 && j > 0 {
        if x[i - 1].norm == y[j - 1].norm {
            pairs.push((i - 1, j - 1));
            i -= 1;
            j -= 1;
        } else if dp[i - 1][j] >= dp[i][j - 1] {
            i -= 1;
        } else {
            j -= 1;
        }
    }
    pairs.reverse();
    pairs
}

fn best_subsequence_alignment(x: &[TokenRef], y: &[TokenRef]) -> Option<Alignment> {
    if x.is_empty() || y.is_empty() { return None; }
    let m = x.len();
    let pairs = lcs_pairs(x, y);
    if pairs.is_empty() { return None; }

    let mut best: Option<(usize, usize, usize, usize)> = None;
    for start in 0..pairs.len() {
        let (x0, y0) = pairs[start];
        if y0 > 6 { continue; }
        for end in start..pairs.len() {
            let (x1, y1) = pairs[end];
            if m - 1 - x1 > 8 { continue; }
            let matched = end - start + 1;
            let x_span = x1 - x0 + 1;
            let y_span = y1 - y0 + 1;
            let density_x = matched as f64 / x_span as f64;
            let density_y = matched as f64 / y_span as f64;
            if density_x < 0.72 || density_y < 0.72 { continue; }
            let matched_tokens: Vec<TokenRef> = pairs[start..=end]
                .iter().map(|(_, yy)| y[*yy].clone()).collect();
            if matched < min_match_tokens(&matched_tokens) { continue; }
            match best {
                Some((best_matched, _, _, best_y_span))
                    if matched < best_matched || (matched == best_matched && y_span >= best_y_span) => {}
                _ => best = Some((matched, start, end, y_span)),
            }
        }
    }

    let (matched, _start, end, _span) = best?;
    Some(Alignment { last_y: pairs[end].1, matched, method: "subsequence-lcs" })
}

fn trim_leading_noise(text: &str) -> &str {
    text.trim_start_matches(|ch: char| {
        ch.is_whitespace() || matches!(ch, ',' | '.' | '?' | '!' | ':' | ';' | '，' | '。' | '？' | '！' | '：' | '；' | '、')
    })
}

fn trim_overlap_tail_particle(text: &str) -> &str {
    let text = trim_leading_noise(text);
    let mut chars = text.char_indices();
    let Some((_, first)) = chars.next() else { return text };
    if !matches!(first, '吧' | '啊' | '呀' | '呢' | '嘛' | '啦' | '呗' | '哇') { return text; }
    let rest = &text[first.len_utf8()..];
    let rest_trimmed = rest.trim_start();
    let Some(mark) = rest_trimmed.chars().next() else { return text };
    if !matches!(mark, '。' | '？' | '！' | '.' | '?' | '!') { return text; }
    trim_leading_noise(&rest_trimmed[mark.len_utf8()..])
}

fn apply_trim(current: &mut Vec<TranscriptSegment>, token: &TokenRef) -> usize {
    if token.segment_index >= current.len() { return 0; }
    let removed_segments = token.segment_index;
    if removed_segments > 0 { current.drain(0..removed_segments); }
    if current.is_empty() { return removed_segments; }

    let original = current[0].text.clone();
    let byte_end = token.byte_end.min(original.len());
    let remainder = trim_overlap_tail_particle(&original[byte_end..]).to_string();
    if remainder.is_empty() {
        current.remove(0);
        return removed_segments + 1;
    }

    let total_tokens = tokenize_segment(&original, 0).len().max(1);
    let removed_tokens = tokenize_segment(&original[..byte_end], 0).len().min(total_tokens);
    let fraction = (removed_tokens as f64 / total_tokens as f64).clamp(0.0, 0.9);
    let old_start = current[0].start;
    let old_end = current[0].end;
    current[0].start = (old_start + (old_end - old_start) * fraction).min(old_end - 0.05);
    current[0].text = remainder;
    removed_segments
}


#[derive(Debug, Clone, Copy)]
struct TailAlignment {
    first_x: usize,
    first_y: usize,
    matched: usize,
}

/// Find an exact normalized token run that reaches the unstable tail of the
/// previous hypothesis and begins at (or extremely near) the head of the new
/// hypothesis. Unlike `stitch_boundary`, this alignment is used to let the
/// *newer* chunk replace the old provisional tail instead of trimming the new
/// chunk away.
fn best_tail_rewrite_alignment(x: &[TokenRef], y: &[TokenRef]) -> Option<TailAlignment> {
    if x.is_empty() || y.is_empty() { return None; }
    let m = x.len();
    let n = y.len();
    let mut prev = vec![0usize; n + 1];
    let mut best: Option<(usize, usize, usize, usize)> = None; // matched, tail_slack, y_start, x_start

    for i in 1..=m {
        let mut row = vec![0usize; n + 1];
        for j in 1..=n {
            if x[i - 1].norm != y[j - 1].norm { continue; }
            row[j] = prev[j - 1] + 1;
            let matched = row[j];
            let x_start = i - matched;
            let y_start = j - matched;
            let tail_slack = m - i;
            if y_start > 2 || tail_slack > 2 { continue; }
            let matched_slice = &y[y_start..j];
            // Rewriting already-visible text is intentionally more conservative
            // than ordinary duplicate trimming. Two-word English repeats such as
            // "extremely frugal" are safe to trim from the new chunk, but are not
            // strong enough evidence to roll back the previous hypothesis.
            let ascii = matched_slice.iter().filter(|token| token.ascii_word).count();
            let required = if ascii * 2 >= matched_slice.len().max(1) { 3 } else { 4 };
            if matched < required { continue; }

            match best {
                Some((best_matched, best_slack, best_y_start, _))
                    if matched < best_matched
                        || (matched == best_matched && tail_slack > best_slack)
                        || (matched == best_matched && tail_slack == best_slack && y_start >= best_y_start) => {}
                _ => best = Some((matched, tail_slack, y_start, x_start)),
            }
        }
        prev = row;
    }

    let (matched, _tail_slack, first_y, first_x) = best?;
    Some(TailAlignment { first_x, first_y, matched })
}

fn truncate_previous_from_token(previous: &mut Vec<TranscriptSegment>, token: &TokenRef) -> usize {
    if previous.is_empty() || token.segment_index >= previous.len() { return 0; }
    let before = collect_tokens(previous).len();
    previous.truncate(token.segment_index + 1);
    if previous.is_empty() { return before; }

    let last_index = previous.len() - 1;
    let original = previous[last_index].text.clone();
    let cut = token.byte_start.min(original.len());
    let prefix = original[..cut].trim_end().to_string();
    if prefix.is_empty() {
        previous.pop();
    } else {
        let total_tokens = tokenize_segment(&original, 0).len().max(1);
        let kept_tokens = tokenize_segment(&prefix, 0).len().min(total_tokens);
        let fraction = (kept_tokens as f64 / total_tokens as f64).clamp(0.02, 0.98);
        let old_start = previous[last_index].start;
        let old_end = previous[last_index].end;
        previous[last_index].end =
            (old_start + (old_end - old_start) * fraction).max(old_start + 0.05);
        previous[last_index].text = prefix;
    }
    before.saturating_sub(collect_tokens(previous).len())
}

fn trim_current_to_token(current: &mut Vec<TranscriptSegment>, token: &TokenRef) {
    if current.is_empty() || token.segment_index >= current.len() { return; }
    if token.segment_index > 0 { current.drain(0..token.segment_index); }
    if current.is_empty() { return; }
    let original = current[0].text.clone();
    let start = token.byte_start.min(original.len());
    if start == 0 { return; }
    let remainder = trim_leading_noise(&original[start..]).to_string();
    if remainder.is_empty() {
        current.remove(0);
        return;
    }
    let total_tokens = tokenize_segment(&original, 0).len().max(1);
    let removed_tokens = tokenize_segment(&original[..start], 0).len().min(total_tokens);
    let fraction = (removed_tokens as f64 / total_tokens as f64).clamp(0.0, 0.9);
    let old_start = current[0].start;
    let old_end = current[0].end;
    current[0].start = (old_start + (old_end - old_start) * fraction).min(old_end - 0.05);
    current[0].text = remainder;
}

/// True delayed commit / local agreement.
///
/// The previous chunk's tail is still provisional. When the next chunk repeats
/// that tail with additional right context, the newer chunk wins: we roll the
/// old tail back to the beginning of the shared token run and keep the new chunk
/// intact from that run forward. This is the important distinction between
/// "delay committing" and merely "delay displaying" the old text.
pub(crate) fn reconcile_delayed_tail(
    pending: &mut Vec<TranscriptSegment>,
    current: &mut Vec<TranscriptSegment>,
    boundary: f64,
    overlap_seconds: f64,
) -> TailRewriteStats {
    if pending.is_empty() || current.is_empty() || overlap_seconds <= 0.0 {
        return TailRewriteStats::default();
    }

    let window = (overlap_seconds * 4.0).clamp(3.0, 8.0);
    let pending_start = pending.iter().position(|s| s.end >= boundary - window).unwrap_or(pending.len());
    let current_end = current.iter().rposition(|s| s.start <= boundary + window).map(|idx| idx + 1).unwrap_or(0);
    if pending_start >= pending.len() || current_end == 0 { return TailRewriteStats::default(); }

    let mut x = collect_tokens(&pending[pending_start..]);
    let mut y = collect_tokens(&current[..current_end]);
    const MAX_TOKENS: usize = 80;
    if x.len() > MAX_TOKENS { x = x[x.len() - MAX_TOKENS..].to_vec(); }
    if y.len() > MAX_TOKENS { y.truncate(MAX_TOKENS); }
    let Some(alignment) = best_tail_rewrite_alignment(&x, &y) else {
        return TailRewriteStats::default();
    };

    // x's segment indexes are relative to pending[pending_start..]. Rebase the
    // chosen token before mutating the real pending vector.
    let mut x_token = x[alignment.first_x].clone();
    x_token.segment_index += pending_start;
    let y_token = y[alignment.first_y].clone();

    let removed_previous_tokens = truncate_previous_from_token(pending, &x_token);
    if alignment.first_y > 0 {
        trim_current_to_token(current, &y_token);
    }

    TailRewriteStats {
        removed_previous_tokens,
        match_tokens: alignment.matched,
        method: "stable-prefix-rewrite",
    }
}

/// Primary overlap stitcher. Only exact text/token evidence is used.
pub(crate) fn stitch_boundary(
    previous: &[TranscriptSegment],
    current: &mut Vec<TranscriptSegment>,
    boundary: f64,
    overlap_seconds: f64,
) -> StitchStats {
    if previous.is_empty() || current.is_empty() || overlap_seconds <= 0.0 { return StitchStats::default(); }

    let window = (overlap_seconds * 4.0).clamp(3.0, 8.0);
    let prev_start = previous.iter().position(|s| s.end >= boundary - window).unwrap_or(previous.len());
    let curr_end = current.iter().rposition(|s| s.start <= boundary + window).map(|idx| idx + 1).unwrap_or(0);
    if prev_start >= previous.len() || curr_end == 0 { return StitchStats::default(); }

    let mut x = collect_tokens(&previous[prev_start..]);
    let mut y = collect_tokens(&current[..curr_end]);
    const MAX_TOKENS: usize = 96;
    if x.len() > MAX_TOKENS { x = x[x.len() - MAX_TOKENS..].to_vec(); }
    if y.len() > MAX_TOKENS { y.truncate(MAX_TOKENS); }
    if x.is_empty() || y.is_empty() { return StitchStats::default(); }

    let alignment = best_contiguous_alignment(&x, &y).or_else(|| best_subsequence_alignment(&x, &y));
    let Some(alignment) = alignment else { return StitchStats::default(); };
    let token = y[alignment.last_y].clone();
    let before_tokens = collect_tokens(current).len();
    let _ = apply_trim(current, &token);
    let after_tokens = collect_tokens(current).len();

    StitchStats {
        trimmed_tokens: before_tokens.saturating_sub(after_tokens),
        match_tokens: alignment.matched,
        method: alignment.method,
    }
}

/// True when previous and current hypotheses occupy the same acoustic time near
/// the chunk hand-off. This is used to trigger a short boundary re-recognition;
/// text similarity alone never triggers the expensive retry.
pub(crate) fn has_boundary_time_conflict(
    previous: &[TranscriptSegment],
    current: &[TranscriptSegment],
    boundary: f64,
    overlap_seconds: f64,
) -> bool {
    if previous.is_empty() || current.is_empty() { return false; }
    let window = (overlap_seconds * 3.0).clamp(2.0, 6.0);
    let prev = previous.iter().rev().find(|s| s.end >= boundary - window);
    let curr = current.iter().find(|s| s.start <= boundary + window);
    match (prev, curr) {
        (Some(p), Some(c)) => {
            let overlap = p.end.min(c.end) - p.start.max(c.start);
            overlap >= 0.20 || (p.end - c.start) >= 0.35
        }
        _ => false,
    }
}

/// Local-Agreement-inspired second pass. `bridge` is a fresh transcription of a
/// short window centered on the boundary. We never use phonetics/fuzzy spelling.
/// If both the previous tail and current head map by exact tokens to overlapping
/// regions of the bridge hypothesis, the duplicated current prefix is removed.
pub(crate) fn stitch_with_bridge(
    previous: &[TranscriptSegment],
    current: &mut Vec<TranscriptSegment>,
    bridge: &[TranscriptSegment],
) -> StitchStats {
    if previous.is_empty() || current.is_empty() || bridge.is_empty() { return StitchStats::default(); }

    let mut p = collect_tokens(previous);
    let mut c = collect_tokens(current);
    let mut r = collect_tokens(bridge);
    const SIDE_MAX: usize = 64;
    const REF_MAX: usize = 128;
    if p.len() > SIDE_MAX { p = p[p.len() - SIDE_MAX..].to_vec(); }
    if c.len() > SIDE_MAX { c.truncate(SIDE_MAX); }
    if r.len() > REF_MAX { r.truncate(REF_MAX); }
    if p.is_empty() || c.is_empty() || r.is_empty() { return StitchStats::default(); }

    let pp = lcs_pairs(&p, &r);
    let cp = lcs_pairs(&c, &r);
    if pp.is_empty() || cp.is_empty() { return StitchStats::default(); }

    // The previous agreement must reach its tail, and current agreement must begin
    // near its head. This prevents a phrase repeated later in the 12s bridge from
    // being treated as a chunk-boundary duplicate.
    let p_first = pp.first().unwrap();
    let p_last = pp.last().unwrap();
    let c_first = cp.first().unwrap();
    let _c_last = cp.last().unwrap();
    if p.len().saturating_sub(1 + p_last.0) > 8 || c_first.0 > 6 { return StitchStats::default(); }

    let p_tokens: Vec<TokenRef> = pp.iter().map(|(_, ri)| r[*ri].clone()).collect();
    let c_tokens: Vec<TokenRef> = cp.iter().map(|(_, ri)| r[*ri].clone()).collect();
    if pp.len() < min_match_tokens(&p_tokens) || cp.len() < min_match_tokens(&c_tokens) {
        return StitchStats::default();
    }

    let p_ref_start = p_first.1;
    let p_ref_end = p_last.1;
    let c_ref_start = c_first.1;
    let c_ref_end = cp.last().unwrap().1;
    let common_start = p_ref_start.max(c_ref_start);
    let common_end = p_ref_end.min(c_ref_end);
    if common_end < common_start || common_end - common_start + 1 < 3 {
        return StitchStats::default();
    }

    // Trim through the token in current that reaches the previous hypothesis end;
    // allow one extra reference token for a sentence-final particle such as “吧”.
    let trim_ref_end = p_ref_end.saturating_add(1);
    let Some((trim_current_index, _)) = cp.iter().rev().find(|(_, ri)| *ri <= trim_ref_end).copied() else {
        return StitchStats::default();
    };
    let token = c[trim_current_index].clone();
    let before_tokens = collect_tokens(current).len();
    let _ = apply_trim(current, &token);
    let after_tokens = collect_tokens(current).len();
    let trimmed = before_tokens.saturating_sub(after_tokens);
    if trimmed == 0 { return StitchStats::default(); }

    StitchStats {
        trimmed_tokens: trimmed,
        match_tokens: (common_end - common_start + 1).max(pp.len().min(cp.len())),
        method: "bridge-agreement",
    }
}


#[derive(Debug, Clone, Copy)]
struct ExactAnchor {
    side_start: usize,
    side_end: usize,
    bridge_start: usize,
    bridge_end: usize,
    matched: usize,
}

fn exact_contiguous_anchors(side: &[TokenRef], bridge: &[TokenRef]) -> Vec<ExactAnchor> {
    let mut out = Vec::new();
    if side.is_empty() || bridge.is_empty() { return out; }
    for i in 0..side.len() {
        for j in 0..bridge.len() {
            if side[i].norm != bridge[j].norm { continue; }
            if i > 0 && j > 0 && side[i - 1].norm == bridge[j - 1].norm {
                continue; // not the start of a maximal run
            }
            let mut len = 0usize;
            while i + len < side.len()
                && j + len < bridge.len()
                && side[i + len].norm == bridge[j + len].norm
            {
                len += 1;
            }
            if len == 0 { continue; }
            out.push(ExactAnchor {
                side_start: i,
                side_end: i + len - 1,
                bridge_start: j,
                bridge_end: j + len - 1,
                matched: len,
            });
        }
    }
    out
}

/// 0.7 authoritative boundary bridge.
///
/// The bridge transcription is centered on the chunk boundary, so it has better
/// left/right context than either main window edge. We use *exact* token anchors
/// on both sides and replace only the unstable text between those anchors with the
/// bridge text. There is no phonetic, edit-distance, or semantic guessing.
///
/// Example:
/// previous: "...可能我见了黄河才会倒。"
/// current : "死心吧，可能我偏要一条路走到黑吧..."
/// bridge  : "...可能我见了黄河才会死心吧，可能我偏要一条路走到黑吧..."
/// becomes : "...可能我见了黄河才会死心吧，" + "可能我偏要一条路走到黑吧..."
pub(crate) fn rewrite_with_authoritative_bridge(
    previous: &mut Vec<TranscriptSegment>,
    current: &mut Vec<TranscriptSegment>,
    bridge: &[TranscriptSegment],
) -> StitchStats {
    if previous.is_empty() || current.is_empty() || bridge.is_empty() {
        return StitchStats::default();
    }

    let prev_index = previous.len() - 1;
    let prev_text = previous[prev_index].text.clone();
    let curr_text = current[0].text.clone();
    let bridge_text = bridge
        .iter()
        .map(|s| s.text.trim())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if prev_text.trim().is_empty() || curr_text.trim().is_empty() || bridge_text.trim().is_empty() {
        return StitchStats::default();
    }

    let mut p = tokenize_segment(&prev_text, 0);
    let mut c = tokenize_segment(&curr_text, 0);
    let mut r = tokenize_segment(&bridge_text, 0);
    const SIDE_MAX: usize = 96;
    const REF_MAX: usize = 192;
    if p.len() > SIDE_MAX { p = p[p.len() - SIDE_MAX..].to_vec(); }
    if c.len() > SIDE_MAX { c.truncate(SIDE_MAX); }
    if r.len() > REF_MAX { r.truncate(REF_MAX); }
    if p.is_empty() || c.is_empty() || r.is_empty() { return StitchStats::default(); }

    let lefts = exact_contiguous_anchors(&p, &r)
        .into_iter()
        .filter(|a| p.len().saturating_sub(1 + a.side_end) <= 8)
        .filter(|a| a.matched >= min_match_tokens(&p[a.side_start..=a.side_end]))
        .collect::<Vec<_>>();
    let mut rights = Vec::new();
    for anchor in exact_contiguous_anchors(&c, &r) {
        // The maximal exact run may begin with a few unstable boundary tokens
        // (e.g. current starts with “死心吧，可能我偏要…”). Generate exact
        // suffix anchors as well, so the bridge can own those first tokens and
        // close the previous sentence naturally. Still exact matching only.
        let max_shift = anchor.matched.saturating_sub(3).min(6);
        for shift in 0..=max_shift {
            let adjusted = ExactAnchor {
                side_start: anchor.side_start + shift,
                side_end: anchor.side_end,
                bridge_start: anchor.bridge_start + shift,
                bridge_end: anchor.bridge_end,
                matched: anchor.matched - shift,
            };
            if adjusted.side_start > 6 { continue; }
            let normal_min = min_match_tokens(&c[adjusted.side_start..=adjusted.side_end]);
            // In authoritative bridge mode a very short CJK right cue can contain one
            // unstable leading character (e.g. "足的一天" vs bridge "的一天"). A 3-char
            // exact suffix is acceptable only because the left anchor is also exact and
            // the bridge comes from re-listening to the boundary audio.
            let authoritative_min = if c.len() <= 6 && adjusted.side_start <= 1 { normal_min.min(3) } else { normal_min };
            if adjusted.matched < authoritative_min { continue; }
            rights.push(adjusted);
        }
    }
    if lefts.is_empty() || rights.is_empty() { return StitchStats::default(); }

    let mut best: Option<(ExactAnchor, ExactAnchor, usize)> = None;
    for left in &lefts {
        for right in &rights {
            // We only rewrite when the bridge contains a real middle region between
            // the left and right exact anchors. Overlap-only cases stay with LCS.
            if left.bridge_end >= right.bridge_start { continue; }
            let bridge_gap = right.bridge_start - left.bridge_end - 1;
            if bridge_gap > 48 { continue; }
            // Prefer a bridge middle that ends at a punctuation boundary. This
            // keeps the repaired sentence together instead of moving arbitrary
            // extra words from the current segment into the previous segment.
            let mid_start = r[left.bridge_end].byte_end.min(bridge_text.len());
            let mid_end = r[right.bridge_start].byte_start.min(bridge_text.len());
            let middle_preview = if mid_end >= mid_start {
                bridge_text[mid_start..mid_end].trim()
            } else {
                ""
            };
            let punctuation_bonus = match middle_preview.chars().last() {
                Some('。' | '？' | '！' | '.' | '?' | '!') => 10,
                Some('，' | '；' | '：' | ',' | ';' | ':') => 8,
                _ => 0,
            };
            let score = left.matched + right.matched + punctuation_bonus;
            match best {
                Some((_, _, best_score)) if score <= best_score => {}
                _ => best = Some((*left, *right, score)),
            }
        }
    }
    let Some((left, right, _)) = best else { return StitchStats::default(); };

    let prev_cut = p[left.side_end].byte_end.min(prev_text.len());
    let bridge_mid_start = r[left.bridge_end].byte_end.min(bridge_text.len());
    let bridge_mid_end = r[right.bridge_start].byte_start.min(bridge_text.len());
    let curr_keep = c[right.side_start].byte_start.min(curr_text.len());
    if bridge_mid_end < bridge_mid_start { return StitchStats::default(); }

    let middle = bridge_text[bridge_mid_start..bridge_mid_end].trim();

    let mut new_prev = prev_text[..prev_cut].trim_end().to_string();
    if !new_prev.is_empty() {
        let last = new_prev.chars().last().unwrap();
        let first = middle.chars().next();
        if last.is_ascii_alphanumeric() && first.map(|c| c.is_ascii_alphanumeric()).unwrap_or(false) {
            new_prev.push(' ');
        }
    }
    new_prev.push_str(middle);
    new_prev = new_prev.trim().to_string();

    let new_curr = trim_leading_noise(&curr_text[curr_keep..]).to_string();
    if new_prev.is_empty() || new_curr.is_empty() { return StitchStats::default(); }

    let before = tokenize_segment(&prev_text, 0).len() + tokenize_segment(&curr_text, 0).len();
    previous[prev_index].text = new_prev;
    current[0].text = new_curr;
    let after = tokenize_segment(&previous[prev_index].text, 0).len() + tokenize_segment(&current[0].text, 0).len();

    StitchStats {
        trimmed_tokens: before.saturating_sub(after),
        match_tokens: left.matched + right.matched,
        method: "authoritative-bridge",
    }
}

pub(crate) fn split_stable_tail(
    mut current: Vec<TranscriptSegment>,
    nominal_end: f64,
    holdback_seconds: f64,
    is_last: bool,
) -> (Vec<TranscriptSegment>, Vec<TranscriptSegment>) {
    if is_last || current.is_empty() || holdback_seconds <= 0.0 { return (current, Vec::new()); }

    let holdback = holdback_seconds.clamp(1.0, 8.0);
    let threshold = nominal_end - holdback;
    let time_split = current
        .iter()
        .position(|segment| segment.end > threshold)
        .unwrap_or(current.len());

    // A GGUF SRT cue can cover most (or all) of a 15-second inference window.
    // Holding whole cues therefore creates large latency and, more importantly,
    // prevents us from keeping only the actually unstable sentence tail. Keep a
    // small token tail even when the runtime emitted one giant cue.
    let all_tokens = collect_tokens(&current);
    if all_tokens.is_empty() {
        let split = time_split.min(current.len().saturating_sub(1));
        let pending = current.split_off(split);
        return (current, pending);
    }
    let ascii = all_tokens.iter().filter(|token| token.ascii_word).count();
    let desired_tail_tokens = if ascii * 2 >= all_tokens.len() { 10usize } else { 16usize };
    let token_cut = all_tokens.len().saturating_sub(desired_tail_tokens.min(all_tokens.len()));
    let token = all_tokens[token_cut].clone();

    // Whichever rule asks us to hold *more* context wins.
    let split_segment = time_split.min(token.segment_index).min(current.len() - 1);
    if split_segment < token.segment_index {
        let pending = current.split_off(split_segment);
        return (current, pending);
    }

    // Split inside a giant cue at a token boundary so the stable prefix can be
    // committed while roughly the last sentence / last few seconds stay mutable.
    let original = current[split_segment].clone();
    let cut = token.byte_start.min(original.text.len());
    if cut == 0 {
        let pending = current.split_off(split_segment);
        return (current, pending);
    }

    let stable_text = original.text[..cut].trim_end().to_string();
    let pending_text = trim_leading_noise(&original.text[cut..]).to_string();
    if stable_text.is_empty() || pending_text.is_empty() {
        let pending = current.split_off(split_segment);
        return (current, pending);
    }

    let total_tokens = tokenize_segment(&original.text, 0).len().max(1);
    let stable_tokens = tokenize_segment(&stable_text, 0).len().min(total_tokens);
    let fraction = (stable_tokens as f64 / total_tokens as f64).clamp(0.02, 0.98);
    let split_time = (original.start + (original.end - original.start) * fraction)
        .clamp(original.start + 0.05, original.end - 0.05);

    let stable_segment = TranscriptSegment {
        id: format!("{}-stable", original.id),
        start: original.start,
        end: split_time,
        text: stable_text,
    };
    let pending_segment = TranscriptSegment {
        id: format!("{}-pending", original.id),
        start: split_time,
        end: original.end,
        text: pending_text,
    };

    let trailing = if split_segment + 1 < current.len() {
        current.split_off(split_segment + 1)
    } else {
        Vec::new()
    };
    current[split_segment] = stable_segment;
    let mut pending = vec![pending_segment];
    pending.extend(trailing);
    (current, pending)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(start: f64, end: f64, text: &str) -> TranscriptSegment {
        TranscriptSegment { id: format!("{start}-{end}"), start, end, text: text.to_string() }
    }

    #[test]
    fn trims_chinese_boundary_overlap() {
        let previous = vec![seg(27.0, 30.8, "所以这个问题需要进一步分析接下来我们来看第二种情况")];
        let mut current = vec![seg(29.4, 33.0, "接下来我们来看第二种情况首先需要确认数据")];
        let stats = stitch_boundary(&previous, &mut current, 30.0, 1.0);
        assert!(stats.trimmed_tokens >= 5);
        assert!(current[0].text.contains("首先需要确认数据"));
    }

    #[test]
    fn trims_english_boundary_overlap() {
        let previous = vec![seg(27.0, 30.6, "today we are going to discuss this important problem")];
        let mut current = vec![seg(29.3, 33.0, "discuss this important problem and then look at the data")];
        let stats = stitch_boundary(&previous, &mut current, 30.0, 1.0);
        assert!(stats.trimmed_tokens >= 3);
        assert!(current[0].text.starts_with("and then"));
    }

    #[test]
    fn bridge_resolves_real_huanghe_weihe_case_without_phonetics() {
        let previous = vec![
            seg(112.0, 119.0, "可能我见了"),
            seg(119.0, 120.98, "黄河才会死心。"),
        ];
        let mut current = vec![seg(119.63, 130.76, "为何才会死心吧？可能我偏要一条路走到黑吧。")];
        let bridge = vec![seg(114.0, 126.0, "可能我见了黄河才会死心吧。可能我偏要一条路走到黑吧。")];
        let stats = stitch_with_bridge(&previous, &mut current, &bridge);
        assert_eq!(stats.method, "bridge-agreement");
        assert!(stats.trimmed_tokens >= 5);
        assert_eq!(current[0].text, "可能我偏要一条路走到黑吧。");
    }

    #[test]
    fn no_phonetic_guess_when_bridge_does_not_agree() {
        let previous = vec![seg(119.0, 120.98, "黄河才会死心。")];
        let mut current = vec![seg(119.63, 130.76, "为何才会死心吧？后面的新内容。")];
        let bridge = vec![seg(114.0, 126.0, "这里是完全不同的内容。")];
        let stats = stitch_with_bridge(&previous, &mut current, &bridge);
        assert_eq!(stats.trimmed_tokens, 0);
        assert!(current[0].text.starts_with("为何"));
    }
    #[test]
    fn authoritative_bridge_repairs_split_sentence_without_phonetics() {
        let mut previous = vec![seg(98.77, 120.41, "直至把电跑光，最终计算车辆的纯电续航达成率。晚上可能我中了南墙才会回头吧，可能我见了黄河才会倒。")] ;
        let mut current = vec![seg(121.045, 130.76, "死心吧，可能我偏要一条路走到黑吧，可能我还没遇见那个他吧。")] ;
        let bridge = vec![seg(115.0, 125.0, "晚上可能我中了南墙才会回头吧，可能我见了黄河才会死心吧，可能我偏要一条路走到黑吧。")] ;
        let stats = rewrite_with_authoritative_bridge(&mut previous, &mut current, &bridge);
        assert_eq!(stats.method, "authoritative-bridge");
        assert!(previous[0].text.ends_with("可能我见了黄河才会死心吧，"));
        assert!(current[0].text.starts_with("可能我偏要一条路走到黑吧"));
        assert!(!previous[0].text.contains("才会倒"));
    }

    #[test]
    fn authoritative_bridge_can_remove_one_unstable_cjk_leading_char() {
        let mut previous = vec![seg(55.0, 60.0, "很多上班族")];
        let mut current = vec![seg(60.0, 61.0, "足的一天。")];
        let bridge = vec![seg(56.0, 62.0, "很多上班族的一天。")];
        let stats = rewrite_with_authoritative_bridge(&mut previous, &mut current, &bridge);
        assert_eq!(stats.method, "authoritative-bridge");
        assert_eq!(previous[0].text, "很多上班族");
        assert_eq!(current[0].text, "的一天。");
    }

    #[test]
    fn trims_short_english_overlap_ignoring_case_and_punctuation() {
        let previous = vec![seg(7.0, 10.2, "She was extremely frugal.")];
        let mut current = vec![seg(9.4, 14.0, "Extremely frugal. Missus Stingy lived alone.")];
        let stats = stitch_boundary(&previous, &mut current, 10.0, 1.0);
        assert_eq!(stats.match_tokens, 2);
        assert!(stats.trimmed_tokens >= 2);
        assert_eq!(current[0].text, "Missus Stingy lived alone.");
    }

    #[test]
    fn delayed_commit_lets_new_chunk_replace_old_tail() {
        let mut pending = vec![seg(
            20.0,
            29.0,
            "Her clothes were worn out, torn and covered with patches. Yet she refused.",
        )];
        let mut current = vec![seg(
            28.0,
            38.0,
            "Yet she refused to buy new ones. Every morning she counted her coins.",
        )];
        let stats = reconcile_delayed_tail(&mut pending, &mut current, 29.0, 1.0);
        assert!(stats.match_tokens >= 3);
        assert!(stats.removed_previous_tokens >= 3);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].text, "Her clothes were worn out, torn and covered with patches.");
        assert!(current[0].text.starts_with("Yet she refused to buy new ones."));
    }

    #[test]
    fn delayed_commit_does_not_rollback_on_only_two_english_words() {
        let mut pending = vec![seg(7.0, 10.2, "She was extremely frugal.")];
        let mut current = vec![seg(9.4, 14.0, "Extremely frugal. Missus Stingy lived alone.")];
        let stats = reconcile_delayed_tail(&mut pending, &mut current, 10.0, 1.0);
        assert_eq!(stats.match_tokens, 0);
        assert_eq!(pending[0].text, "She was extremely frugal.");
    }

    #[test]
    fn stable_prefix_splits_inside_one_giant_runtime_cue() {
        let current = vec![seg(
            14.0,
            29.0,
            "She was extremely frugal. Her clothes were worn out, torn and covered with patches. Yet she refused.",
        )];
        let (stable, pending) = split_stable_tail(current, 29.0, 2.0, false);
        assert!(!stable.is_empty());
        assert!(!pending.is_empty());
        let stable_text = stable.iter().map(|s| s.text.as_str()).collect::<Vec<_>>().join(" ");
        let pending_text = pending.iter().map(|s| s.text.as_str()).collect::<Vec<_>>().join(" ");
        assert!(stable_text.contains("She was extremely frugal."));
        assert!(pending_text.contains("Yet she refused"));
    }

}

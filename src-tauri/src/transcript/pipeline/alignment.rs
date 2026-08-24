use std::ops::Range;

use crate::transcript::model::{LanguageProfile, RawSegment, RawToken};
use crate::transcript::pipeline::{BoundaryEvidence, BoundaryEvidenceKind};

/// Builds sentence-boundary evidence from language punctuation plus the best timing source
/// currently available. If a word/token timeline exists, sentence candidates are anchored to
/// adjacent token timestamps. Otherwise we use a nearby acoustic pause when available and only
/// fall back to a low-confidence monotonic estimate for presentation-friendly Canonical timing.
pub fn derive_sentence_boundary_evidence(
    segments: &[RawSegment],
    profile: LanguageProfile,
    existing: &[BoundaryEvidence],
) -> Vec<BoundaryEvidence> {
    let mut out = existing.to_vec();

    for seg in segments {
        let normalized = normalize_text(&seg.text);
        let char_len = normalized.chars().count();
        if char_len < 2 {
            continue;
        }
        let candidates = sentence_boundary_candidates(&normalized, profile);
        if candidates.is_empty() {
            continue;
        }

        let token_spans = align_token_char_spans(&normalized, &seg.tokens);
        for offset in candidates {
            if offset == 0 || offset >= char_len {
                continue;
            }
            if out.iter().any(|e| {
                e.segment_id == seg.id
                    && matches!(e.kind, BoundaryEvidenceKind::AlignmentBoundary | BoundaryEvidenceKind::StrongPunctuation)
                    && e.char_offset.abs_diff(offset) <= 1
            }) {
                continue;
            }

            if let Some((time_ms, confidence)) = aligned_time_for_offset(&seg.tokens, token_spans.as_deref(), offset) {
                out.push(BoundaryEvidence {
                    segment_id: seg.id.clone(),
                    char_offset: offset,
                    time_ms,
                    gap_ms: adjacent_gap_for_offset(&seg.tokens, token_spans.as_deref(), offset),
                    confidence,
                    kind: BoundaryEvidenceKind::AlignmentBoundary,
                });
                continue;
            }

            if let Some(pause) = nearest_acoustic_pause(existing, &seg.id, offset) {
                out.push(BoundaryEvidence {
                    segment_id: seg.id.clone(),
                    char_offset: offset,
                    time_ms: pause.time_ms,
                    gap_ms: pause.gap_ms,
                    confidence: (pause.confidence * 0.85 + 0.12).min(0.92),
                    kind: BoundaryEvidenceKind::StrongPunctuation,
                });
                continue;
            }

            // No word-level aligner for this language yet. Preserve the textual sentence structure,
            // but make the lower confidence explicit in TransformationLog. This timing is only a
            // temporary monotonic estimate and can later be replaced by a CJK alignment timeline.
            let fraction = offset as f64 / char_len.max(1) as f64;
            let duration = seg.end_ms.saturating_sub(seg.start_ms);
            let time_ms = seg.start_ms + (duration as f64 * fraction).round() as u64;
            out.push(BoundaryEvidence {
                segment_id: seg.id.clone(),
                char_offset: offset,
                time_ms: time_ms.clamp(seg.start_ms, seg.end_ms),
                gap_ms: 0,
                confidence: 0.52,
                kind: BoundaryEvidenceKind::StrongPunctuation,
            });
        }
    }

    out.sort_by(|a, b| {
        a.segment_id
            .cmp(&b.segment_id)
            .then_with(|| a.char_offset.cmp(&b.char_offset))
            .then_with(|| a.time_ms.cmp(&b.time_ms))
    });
    out
}

fn normalize_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn sentence_boundary_candidates(text: &str, profile: LanguageProfile) -> Vec<usize> {
    let chars = text.chars().collect::<Vec<_>>();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        let ch = chars[i];
        if !matches!(ch, '.' | '!' | '?' | '。' | '！' | '？') {
            i += 1;
            continue;
        }

        // Consume a punctuation cluster so "..." / "?!" creates at most one candidate.
        let mut end = i + 1;
        while end < chars.len() && matches!(chars[end], '.' | '!' | '?' | '。' | '！' | '？') {
            end += 1;
        }
        // Include closing quotes/brackets in the left sentence.
        while end < chars.len() && matches!(chars[end], '"' | '\'' | '”' | '’' | ')' | ']' | '】' | '》') {
            end += 1;
        }

        if end < chars.len() && is_real_sentence_terminal(&chars, i, end, profile) {
            out.push(end);
        }
        i = end;
    }
    out
}

fn is_real_sentence_terminal(chars: &[char], punct_start: usize, punct_end: usize, profile: LanguageProfile) -> bool {
    let mark = chars[punct_start];
    let prev_word = previous_word(chars, punct_start).to_ascii_lowercase();

    // Numeric decimal / version: 3.5, 130.1, 2.0.1.
    // Do not inspect the whole surrounding "word" here: in CJK text,
    // `is_alphanumeric()` also treats Chinese characters as alphanumeric, so
    // `那电池度数为130.1度电` would otherwise yield prev_word=`那电池度数为130`
    // and next_word=`1度电`, causing the decimal point to be mistaken for a
    // sentence terminator.  A decimal/version dot is identified by its
    // immediately adjacent characters instead.
    if mark == '.' {
        let prev_char = punct_start
            .checked_sub(1)
            .and_then(|idx| chars.get(idx))
            .copied();
        let next_char = chars.get(punct_end).copied();
        if prev_char.is_some_and(|c| c.is_ascii_digit())
            && next_char.is_some_and(|c| c.is_ascii_digit())
        {
            return false;
        }
    }
    // Common Chinese ASR decimal artifact: "579 点。8" should be repaired by ITN, not split.
    if mark == '。' {
        let prev_visible = chars[..punct_start].iter().rev().find(|c| !c.is_whitespace()).copied();
        let next_visible = chars[punct_end..].iter().find(|c| !c.is_whitespace()).copied();
        if prev_visible == Some('点') && next_visible.is_some_and(is_numeric_char) {
            return false;
        }
    }

    if matches!(profile, LanguageProfile::En | LanguageProfile::Mixed | LanguageProfile::Auto) && mark == '.' {
        const ABBREVIATIONS: &[&str] = &[
            "mr", "mrs", "ms", "dr", "prof", "sr", "jr", "st", "vs", "etc", "fig", "no", "dept",
            "inc", "ltd", "jan", "feb", "mar", "apr", "jun", "jul", "aug", "sep", "sept", "oct", "nov", "dec",
        ];
        if ABBREVIATIONS.contains(&prev_word.as_str()) {
            return false;
        }
        // Initials and acronym fragments: "J. K. Rowling", "U.S. market".
        if prev_word.chars().count() == 1 && prev_word.chars().all(|c| c.is_ascii_alphabetic()) {
            return false;
        }
    }

    true
}

fn previous_word(chars: &[char], before: usize) -> String {
    let mut end = before;
    while end > 0 && (chars[end - 1].is_whitespace() || matches!(chars[end - 1], '"' | '\'' | '”' | '’' | ')' | ']' | '】')) {
        end -= 1;
    }
    let mut start = end;
    while start > 0 && (chars[start - 1].is_alphanumeric() || matches!(chars[start - 1], '\'' | '_' | '-')) {
        start -= 1;
    }
    chars[start..end].iter().collect()
}

fn is_numeric_char(ch: char) -> bool {
    ch.is_ascii_digit() || matches!(ch, '零' | '〇' | '一' | '二' | '两' | '三' | '四' | '五' | '六' | '七' | '八' | '九' | '幺')
}

/// Character ranges, not byte ranges. RawToken text is searched sequentially against the
/// whitespace-normalized ASR surface. Exact case is preferred, then ASCII case-insensitive.
fn align_token_char_spans(text: &str, tokens: &[RawToken]) -> Option<Vec<Range<usize>>> {
    if tokens.is_empty() {
        return None;
    }
    let hay = text.chars().collect::<Vec<_>>();
    let mut cursor = 0usize;
    let mut spans = Vec::with_capacity(tokens.len());
    for token in tokens {
        let needle = token.text.trim().chars().collect::<Vec<_>>();
        if needle.is_empty() {
            spans.push(cursor..cursor);
            continue;
        }
        let pos = find_char_slice(&hay, cursor, &needle)
            .or_else(|| find_char_slice_ascii_case_insensitive(&hay, cursor, &needle))?;
        spans.push(pos..pos + needle.len());
        cursor = pos + needle.len();
    }
    Some(spans)
}

fn find_char_slice(hay: &[char], start: usize, needle: &[char]) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    (start..=hay.len().saturating_sub(needle.len())).find(|&i| &hay[i..i + needle.len()] == needle)
}

fn find_char_slice_ascii_case_insensitive(hay: &[char], start: usize, needle: &[char]) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    (start..=hay.len().saturating_sub(needle.len())).find(|&i| {
        hay[i..i + needle.len()]
            .iter()
            .zip(needle.iter())
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
    })
}

fn aligned_time_for_offset(tokens: &[RawToken], spans: Option<&[Range<usize>]>, offset: usize) -> Option<(u64, f32)> {
    let spans = spans?;
    if tokens.len() != spans.len() || tokens.len() < 2 {
        return None;
    }
    for i in 0..tokens.len() - 1 {
        if offset >= spans[i].end && offset <= spans[i + 1].start {
            let left = &tokens[i];
            let right = &tokens[i + 1];
            let char_gap = spans[i + 1].start.saturating_sub(spans[i].end);
            let char_pos = offset.saturating_sub(spans[i].end);
            let fraction = if char_gap > 0 { char_pos as f64 / char_gap as f64 } else { 0.5 };
            let acoustic_gap = right.start_ms.saturating_sub(left.end_ms);
            let time = left.end_ms + (acoustic_gap as f64 * fraction.clamp(0.0, 1.0)).round() as u64;
            let coverage_penalty = if char_gap > 40 { 0.15 } else if char_gap > 20 { 0.08 } else { 0.0 };
            let confidence = (left.confidence.min(right.confidence) - coverage_penalty).clamp(0.52, 0.99);
            return Some((time, confidence));
        }
    }
    None
}

fn adjacent_gap_for_offset(tokens: &[RawToken], spans: Option<&[Range<usize>]>, offset: usize) -> u64 {
    let Some(spans) = spans else { return 0 };
    if tokens.len() != spans.len() || tokens.len() < 2 {
        return 0;
    }
    for i in 0..tokens.len() - 1 {
        if offset >= spans[i].end && offset <= spans[i + 1].start {
            return tokens[i + 1].start_ms.saturating_sub(tokens[i].end_ms);
        }
    }
    0
}

fn nearest_acoustic_pause<'a>(existing: &'a [BoundaryEvidence], segment_id: &str, offset: usize) -> Option<&'a BoundaryEvidence> {
    existing
        .iter()
        .filter(|e| e.segment_id == segment_id && e.kind == BoundaryEvidenceKind::AcousticPause)
        .filter(|e| e.char_offset.abs_diff(offset) <= 14)
        .min_by_key(|e| e.char_offset.abs_diff(offset))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mrs_is_not_sentence_boundary() {
        let offsets = sentence_boundary_candidates(
            "There lived a woman known as Mrs. Stingy. It was quiet.",
            LanguageProfile::En,
        );
        assert_eq!(offsets.len(), 1);
    }

    #[test]
    fn decimal_artifact_is_not_sentence_boundary() {
        let offsets = sentence_boundary_candidates("续航579点。8公里。下一句。", LanguageProfile::Zh);
        assert_eq!(offsets.len(), 1);
    }

    #[test]
    fn ascii_decimal_in_cjk_text_is_not_sentence_boundary() {
        let offsets = sentence_boundary_candidates(
            "那电池度数为130.1度电。城市CLTC折现率为100.2%。下一句。",
            LanguageProfile::Zh,
        );
        assert_eq!(offsets.len(), 2);
    }

    #[test]
    fn multiple_ascii_decimals_are_not_split() {
        let offsets = sentence_boundary_candidates(
            "高速行驶400.3公里以后，折现率为59.6%，电耗在21.8度电到24度电之间波动。下一句。",
            LanguageProfile::Zh,
        );
        assert_eq!(offsets.len(), 1);
    }
}

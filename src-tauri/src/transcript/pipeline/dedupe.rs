use crate::transcript::model::{CanonicalSegment, LanguageProfile};
use crate::transcript::pipeline::edit::{merged_source_ids, refresh_segment_text};
use crate::transcript::transform::{TransformLog, TransformOperation, TransformStage};

/// Canonical dedupe intentionally handles only high-confidence mechanical duplication:
/// 1) overlap between adjacent chunks; 2) decode loops repeated >= 4 times in a very short window.
/// Ordinary rhetorical repetition remains untouched in Standard; no hidden cleanup view rewrites it.
pub fn run_conservative_dedupe(
    segments: &mut Vec<CanonicalSegment>,
    profile: LanguageProfile,
    log: &mut TransformLog,
) {
    dedupe_chunk_overlap(segments, profile, log);
    for seg in segments.iter_mut() {
        collapse_obvious_token_loops(seg, profile, log);
    }
    segments.retain(|s| !s.text.trim().is_empty());
}

fn dedupe_chunk_overlap(segments: &mut [CanonicalSegment], profile: LanguageProfile, log: &mut TransformLog) {
    for i in 1..segments.len() {
        let (left, right) = segments.split_at_mut(i);
        let prev = &left[i - 1];
        let next = &mut right[0];
        if prev.tokens.is_empty() || next.tokens.is_empty() { continue; }

        let temporal_overlap = next.start_ms <= prev.end_ms.saturating_add(300);
        if !temporal_overlap { continue; }

        let max = prev.tokens.len().min(next.tokens.len()).min(16);
        let mut best = 0usize;
        for n in 1..=max {
            let a = &prev.tokens[prev.tokens.len() - n..];
            let b = &next.tokens[..n];
            if token_slices_equivalent(a, b) && overlap_is_substantial(a) { best = n; }
        }
        if best == 0 { continue; }

        let removed = next.tokens[..best].to_vec();
        let before = next.text.clone();
        let source_ids = merged_source_ids(&removed);
        next.tokens.drain(..best);
        refresh_segment_text(next, profile);
        log.record(
            TransformStage::Dedupe,
            TransformOperation::RemoveDuplicate,
            source_ids,
            before,
            next.text.clone(),
            "adjacent_chunk_overlap",
            0.99,
        );
    }
}

fn collapse_obvious_token_loops(seg: &mut CanonicalSegment, profile: LanguageProfile, log: &mut TransformLog) {
    if seg.tokens.len() < 4 { return; }
    let mut i = 0usize;
    while i < seg.tokens.len() {
        let key = normalize(&seg.tokens[i].text);
        if key.is_empty() { i += 1; continue; }
        let mut j = i + 1;
        while j < seg.tokens.len() && normalize(&seg.tokens[j].text) == key {
            j += 1;
        }
        let count = j - i;
        let duration = seg.tokens[j - 1].end_ms.saturating_sub(seg.tokens[i].start_ms);
        if count >= 4 && duration <= 1_500 {
            let removed = seg.tokens[i + 1..j].to_vec();
            let before = seg.text.clone();
            let source_ids = merged_source_ids(&removed);
            let merged = removed.iter().fold(seg.tokens[i].provenance.clone(), |p, t| p.merge(&t.provenance));
            seg.tokens[i].provenance = merged;
            seg.tokens.drain(i + 1..j);
            refresh_segment_text(seg, profile);
            log.record(
                TransformStage::Dedupe,
                TransformOperation::RemoveDuplicate,
                source_ids,
                before,
                seg.text.clone(),
                "high_repetition_decode_loop",
                0.98,
            );
            i += 1;
        } else {
            i = j;
        }
    }
}

fn token_slices_equivalent(a: &[crate::transcript::model::CanonicalToken], b: &[crate::transcript::model::CanonicalToken]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| normalize(&x.text) == normalize(&y.text))
}

fn overlap_is_substantial(tokens: &[crate::transcript::model::CanonicalToken]) -> bool {
    if tokens.len() >= 2 { return true; }
    tokens.first().map(|t| normalize(&t.text).chars().count() >= 4).unwrap_or(false)
}

fn normalize(text: &str) -> String {
    text.trim_matches(|c: char| c.is_whitespace() || c.is_ascii_punctuation() || matches!(c, '，' | '。' | '！' | '？' | '、'))
        .to_lowercase()
}

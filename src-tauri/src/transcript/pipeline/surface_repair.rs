use crate::transcript::model::{LanguageProfile, RawSegment};
use crate::transcript::pipeline::{BoundaryEvidence, BoundaryEvidenceKind, SurfaceRepairEvidence};
use crate::transcript::surface::{analyze_decoder_surface, lexical_units, punctuation_only_projection};
use crate::transcript::transform::{TransformLog, TransformOperation, TransformStage};

const MIN_FALLBACK_PAUSE_CONFIDENCE: f32 = 0.66;
const MIN_FALLBACK_PAUSE_GAP_MS: u64 = 320;

/// Repairs only presentation structure on the Canonical working copy. Raw ASR remains immutable.
/// Retry evidence may contribute punctuation only when its lexical units exactly match the current
/// working text. If retry punctuation is unavailable, a severely collapsed long segment may use
/// strong acoustic pauses as a conservative sentence-boundary fallback.
pub fn apply_surface_repairs(
    mut segments: Vec<RawSegment>,
    profile: LanguageProfile,
    boundary_evidence: &[BoundaryEvidence],
    retry_repairs: &[SurfaceRepairEvidence],
    log: &mut TransformLog,
) -> Vec<RawSegment> {
    for seg in &mut segments {
        let duration_ms = seg.end_ms.saturating_sub(seg.start_ms);
        let mut health = analyze_decoder_surface(&seg.text, duration_ms);
        let has_retry = retry_repairs.iter().any(|repair| {
            repair.target_segment_ids.len() == 1 && repair.target_segment_ids[0] == seg.id
        });
        if !health.punctuation_degenerated && !has_retry {
            continue;
        }

        if let Some(repair) = retry_repairs.iter().find(|repair| {
            repair.target_segment_ids.len() == 1 && repair.target_segment_ids[0] == seg.id
        }) {
            if let Some(projected) = punctuation_only_projection(&seg.text, &repair.observed_text) {
                if projected != seg.text && has_more_sentence_structure(&projected, &seg.text) {
                    let before = seg.text.clone();
                    seg.text = projected;
                    log.record(
                        TransformStage::Boundary,
                        TransformOperation::NormalizePunctuation,
                        seg.tokens.iter().map(|token| token.id).collect(),
                        before,
                        seg.text.clone(),
                        &repair.rule_id,
                        repair.confidence,
                    );
                    health = analyze_decoder_surface(&seg.text, duration_ms);
                }
            }
        }

        if health.punctuation_degenerated {
            restore_from_strong_pauses(seg, profile, boundary_evidence, log);
        }
    }
    segments
}

fn has_more_sentence_structure(after: &str, before: &str) -> bool {
    strong_marks(after) > strong_marks(before)
}

fn strong_marks(text: &str) -> usize {
    text.chars().filter(|c| matches!(c, '.' | '?' | '!' | '。' | '？' | '！')).count()
}

fn restore_from_strong_pauses(
    seg: &mut RawSegment,
    profile: LanguageProfile,
    evidence: &[BoundaryEvidence],
    log: &mut TransformLog,
) {
    let normalized = normalize_spaces(&seg.text);
    let char_len = normalized.chars().count();
    if char_len < 8 {
        return;
    }

    let mut candidates = evidence
        .iter()
        .filter(|item| {
            item.segment_id == seg.id
                && item.kind == BoundaryEvidenceKind::AcousticPause
                && item.confidence >= MIN_FALLBACK_PAUSE_CONFIDENCE
                && item.gap_ms >= MIN_FALLBACK_PAUSE_GAP_MS
                && item.char_offset > 0
                && item.char_offset < char_len
        })
        .filter(|item| pause_has_enough_lexical_context(&normalized, item.char_offset))
        .cloned()
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return;
    }

    // Prefer stronger/longer pauses, then keep boundaries separated enough to avoid subtitle-like
    // over-segmentation. Offsets are later applied in descending order.
    candidates.sort_by(|a, b| {
        b.confidence
            .total_cmp(&a.confidence)
            .then_with(|| b.gap_ms.cmp(&a.gap_ms))
            .then_with(|| a.char_offset.cmp(&b.char_offset))
    });
    let mut selected = Vec::<BoundaryEvidence>::new();
    for candidate in candidates {
        if selected.iter().any(|chosen| chosen.char_offset.abs_diff(candidate.char_offset) < 16) {
            continue;
        }
        selected.push(candidate);
        if selected.len() >= 6 {
            break;
        }
    }
    selected.sort_by_key(|item| std::cmp::Reverse(item.char_offset));

    let before = seg.text.clone();
    let mut chars = normalized.chars().collect::<Vec<_>>();
    let mut min_confidence = 1.0_f32;
    for item in &selected {
        min_confidence = min_confidence.min(item.confidence);
        let target_offset = item.char_offset.min(chars.len());
        insert_sentence_mark(&mut chars, target_offset, profile);
    }
    seg.text = normalize_spaces(&chars.into_iter().collect::<String>());
    if seg.text != before {
        log.record(
            TransformStage::Boundary,
            TransformOperation::NormalizePunctuation,
            seg.tokens.iter().map(|token| token.id).collect(),
            before,
            seg.text.clone(),
            "decoder_surface_collapse_acoustic_boundary_fallback",
            min_confidence.max(MIN_FALLBACK_PAUSE_CONFIDENCE),
        );
    }
}

fn pause_has_enough_lexical_context(text: &str, char_offset: usize) -> bool {
    let Some(byte_offset) = char_to_byte_offset(text, char_offset) else { return false };
    let left = lexical_units(&text[..byte_offset]).len();
    let right = lexical_units(&text[byte_offset..]).len();
    left >= 4 && right >= 3
}

fn insert_sentence_mark(chars: &mut Vec<char>, offset: usize, profile: LanguageProfile) {
    if chars.is_empty() || offset == 0 {
        return;
    }
    let mut left = offset.min(chars.len());
    while left > 0 && chars[left - 1].is_whitespace() {
        left -= 1;
    }
    if left == 0 {
        return;
    }
    let prev = chars[left - 1];
    if matches!(prev, '.' | '?' | '!' | '。' | '？' | '！') {
        return;
    }
    let mark = if profile.prefers_cjk_spacing() { '。' } else { '.' };
    if matches!(prev, ',' | ';' | ':' | '，' | '；' | '：' | '、') {
        chars[left - 1] = mark;
    } else {
        chars.insert(left, mark);
    }
}

fn char_to_byte_offset(text: &str, char_offset: usize) -> Option<usize> {
    if char_offset == text.chars().count() {
        Some(text.len())
    } else {
        text.char_indices().nth(char_offset).map(|(idx, _)| idx)
    }
}

fn normalize_spaces(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::model::RawSegment;

    #[test]
    fn retry_projection_imports_only_punctuation() {
        let segments = vec![RawSegment {
            id: "s".into(),
            start_ms: 0,
            end_ms: 20_000,
            text: "HOW'S THE GAME PRETTY GOOD RIGHT I DON'T THINK IT'S FUN".into(),
            tokens: vec![],
        }];
        let mut log = TransformLog::new("j");
        let out = apply_surface_repairs(
            segments,
            LanguageProfile::En,
            &[],
            &[SurfaceRepairEvidence {
                target_segment_ids: vec!["s".into()],
                observed_text: "How's the game? Pretty good, right. I don't think it's fun.".into(),
                confidence: 0.91,
                rule_id: "decoder_surface_retry_punctuation_projection".into(),
            }],
            &mut log,
        );
        assert_eq!(out[0].text, "HOW'S THE GAME? PRETTY GOOD, RIGHT. I DON'T THINK IT'S FUN.");
    }
}

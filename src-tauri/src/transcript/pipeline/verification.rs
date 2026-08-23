use crate::transcript::model::RawSegment;
use crate::transcript::transform::{TransformLog, TransformOperation, TransformStage};

#[derive(Debug, Clone, PartialEq)]
pub struct VerificationRewriteEvidence {
    /// Contiguous Raw segment ids that were re-listened to and jointly verified.
    pub target_segment_ids: Vec<String>,
    /// Replacement surface for the complete target span. Empty means the verifier confirmed that
    /// the suspicious span contains no independently spoken content.
    pub replacement_text: String,
    pub confidence: f32,
    pub rule_id: String,
}

pub fn apply_verification_rewrites(
    mut segments: Vec<RawSegment>,
    rewrites: &[VerificationRewriteEvidence],
    log: &mut TransformLog,
) -> Vec<RawSegment> {
    for rewrite in rewrites {
        if rewrite.target_segment_ids.is_empty() { continue; }
        let mut indices = rewrite.target_segment_ids.iter()
            .filter_map(|id| segments.iter().position(|s| &s.id == id))
            .collect::<Vec<_>>();
        if indices.len() != rewrite.target_segment_ids.len() { continue; }
        indices.sort_unstable();
        if indices.windows(2).any(|w| w[1] != w[0] + 1) { continue; }

        let first = indices[0];
        let last = *indices.last().unwrap_or(&first);
        let before = segments[first..=last]
            .iter().map(|s| s.text.trim()).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ");
        let source_ids = segments[first..=last]
            .iter().flat_map(|s| s.tokens.iter().map(|t| t.id)).collect::<Vec<_>>();

        if rewrite.replacement_text.trim().is_empty() {
            // Never leave the Canonical working set structurally invalid: removal is allowed only
            // when at least one other segment remains.
            if segments.len() <= indices.len() { continue; }
            for index in indices.into_iter().rev() { segments.remove(index); }
            log.record(
                TransformStage::Verification,
                TransformOperation::VerificationCorrection,
                source_ids,
                before,
                "",
                rewrite.rule_id.clone(),
                rewrite.confidence,
            );
            continue;
        }

        let start_ms = segments[first].start_ms;
        let end_ms = segments[last].end_ms;
        segments[first].text = rewrite.replacement_text.trim().to_string();
        segments[first].start_ms = start_ms;
        segments[first].end_ms = end_ms;
        // The surface changed after acoustic re-verification, so the previous lexical alignment is
        // no longer authoritative. Preserve segment timing and require a future aligner to rebuild
        // word/token provenance if needed.
        segments[first].tokens.clear();
        for index in (first + 1..=last).rev() { segments.remove(index); }
        log.record(
            TransformStage::Verification,
            TransformOperation::VerificationCorrection,
            source_ids,
            before,
            segments[first].text.clone(),
            rewrite.rule_id.clone(),
            rewrite.confidence,
        );
    }
    segments
}

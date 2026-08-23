use crate::transcript::model::RawSegment;
use crate::transcript::pipeline::CrossBoundaryRewriteEvidence;
use crate::transcript::transform::{TransformLog, TransformOperation, TransformStage};

/// Applies authoritative bridge text only to the Canonical working copy. Raw files are never
/// rewritten. The producer of this evidence must have re-listened to the boundary audio (or used
/// an equivalent acoustic authority); ordinary text heuristics must not populate this structure.
pub fn apply_cross_boundary_rewrites(
    mut segments: Vec<RawSegment>,
    rewrites: &[CrossBoundaryRewriteEvidence],
    log: &mut TransformLog,
) -> Vec<RawSegment> {
    for rewrite in rewrites {
        let left_index = segments.iter().position(|s| s.id == rewrite.left_segment_id);
        let right_index = segments.iter().position(|s| s.id == rewrite.right_segment_id);
        let (Some(li), Some(ri)) = (left_index, right_index) else { continue };
        if li == ri { continue; }

        if let Some(text) = rewrite.left_text.as_ref().filter(|s| !s.trim().is_empty()) {
            let before = segments[li].text.clone();
            if before != *text {
                // A bridge rewrite invalidates any old lexical alignment for the changed surface;
                // retain timing at segment level and let a future aligner regenerate token detail.
                segments[li].text = text.trim().to_string();
                segments[li].tokens.clear();
                log.record(
                    TransformStage::Boundary,
                    if rewrite.drop_right {
                        TransformOperation::RepairBoundaryFragment
                    } else {
                        TransformOperation::CrossBoundaryRewrite
                    },
                    Vec::new(),
                    before,
                    segments[li].text.clone(),
                    if rewrite.drop_right {
                        "authoritative_bridge_boundary_fragment_left_rewrite"
                    } else {
                        "authoritative_bridge_left_rewrite"
                    },
                    rewrite.confidence,
                );
            }
        }
        if let Some(text) = rewrite.right_text.as_ref().filter(|s| !s.trim().is_empty()) {
            let before = segments[ri].text.clone();
            if before != *text {
                segments[ri].text = text.trim().to_string();
                segments[ri].tokens.clear();
                log.record(
                    TransformStage::Boundary,
                    TransformOperation::CrossBoundaryRewrite,
                    Vec::new(),
                    before,
                    segments[ri].text.clone(),
                    "authoritative_bridge_right_rewrite",
                    rewrite.confidence,
                );
            }
        }

        if rewrite.drop_right {
            // The right segment is removed only after an authoritative bridge has repaired the
            // lexical continuation. This is never triggered by script/language heuristics alone.
            let removed = segments.remove(ri);
            log.record(
                TransformStage::Boundary,
                TransformOperation::RepairBoundaryFragment,
                removed.tokens.iter().map(|t| t.id).collect(),
                removed.text,
                "",
                "authoritative_bridge_drop_verified_boundary_fragment",
                rewrite.confidence,
            );
        }
    }
    segments
}

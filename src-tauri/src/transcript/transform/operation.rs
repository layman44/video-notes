use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TransformStage {
    Integrity,
    Verification,
    Boundary,
    SemanticBoundary,
    Dedupe,
    Itn,
    Entity,
    Typography,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TransformOperation {
    TagAnomaly,
    VerificationCorrection,
    DropHallucination,
    MergeBoundary,
    MergeSemanticBoundary,
    RelocateBoundary,
    SplitBoundary,
    CrossBoundaryRewrite,
    RepairBoundaryFragment,
    RemoveDuplicate,
    MergeDecimal,
    NormalizeNumber,
    NormalizePercentage,
    NormalizeUnit,
    NormalizeEntity,
    StitchAcronym,
    NormalizePunctuation,
    NormalizeSpacing,
    NormalizeCasing,
}

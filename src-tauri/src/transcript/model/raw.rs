use serde::{Deserialize, Serialize};
use super::token::{TimeSpan, TokenId};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AsrMetadata {
    /// Legacy compatibility field. New code should treat CURRENT_PIPELINE_VERSION as canonical-pipeline metadata,
    /// not as an ASR property. It is intentionally retained so existing callers do not break.
    #[serde(default)]
    pub pipeline_version: String,
    pub asr_backend: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asr_model_version: Option<String>,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_audio_hash: Option<String>,
    /// Stable identity of the immutable Raw ASR revision. Derived from Raw content, not wall-clock time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_revision_id: Option<String>,
    /// Stable non-cryptographic content fingerprint used to tie Canonical output to this Raw revision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_content_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawToken {
    pub id: TokenId,
    pub text: String,
    pub start_ms: u64,
    pub end_ms: u64,
    #[serde(default = "default_confidence")]
    pub confidence: f32,
}

fn default_confidence() -> f32 { 1.0 }

impl RawToken {
    pub fn span(&self) -> TimeSpan { TimeSpan::new(self.start_ms, self.end_ms) }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawSegment {
    pub id: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    #[serde(default)]
    pub tokens: Vec<RawToken>,
}

impl RawSegment {
    pub fn span(&self) -> TimeSpan { TimeSpan::new(self.start_ms, self.end_ms) }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawTranscript {
    pub job_id: String,
    pub metadata: AsrMetadata,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    pub segments: Vec<RawSegment>,
}

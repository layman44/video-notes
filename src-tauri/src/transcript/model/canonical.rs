use serde::{Deserialize, Serialize};
use super::provenance::Provenance;
use super::raw::AsrMetadata;
use super::token::{TimeSpan, TokenId};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanonicalToken {
    /// Canonical token id. Raw token ids are preserved in `provenance`.
    pub id: TokenId,
    pub text: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub provenance: Provenance,
}

impl CanonicalToken {
    pub fn span(&self) -> TimeSpan {
        TimeSpan::new(self.start_ms, self.end_ms)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanonicalSegment {
    pub id: String,
    pub start_ms: u64,
    pub end_ms: u64,
    /// Projection of `tokens`. Pipeline stages must keep both in sync.
    pub text: String,
    #[serde(default)]
    pub tokens: Vec<CanonicalToken>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub translated_text: Option<String>,
}

impl CanonicalSegment {
    pub fn span(&self) -> TimeSpan {
        TimeSpan::new(self.start_ms, self.end_ms)
    }

    pub fn refresh_bounds_from_tokens(&mut self) {
        if let (Some(first), Some(last)) = (self.tokens.first(), self.tokens.last()) {
            self.start_ms = first.start_ms;
            self.end_ms = last.end_ms.max(first.start_ms);
        }
    }

    pub fn source_token_ids(&self) -> Vec<TokenId> {
        let mut ids: Vec<TokenId> = self
            .tokens
            .iter()
            .flat_map(|t| t.provenance.source_token_ids.iter().copied())
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanonicalTranscript {
    pub job_id: String,
    /// Kept for API compatibility. Pipeline version is persisted separately by storage/version.
    pub metadata: AsrMetadata,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    pub segments: Vec<CanonicalSegment>,
}

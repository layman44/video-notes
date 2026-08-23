use serde::{Deserialize, Serialize};
use crate::transcript::model::TokenId;
use super::operation::{TransformOperation, TransformStage};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransformRecord {
    pub stage: TransformStage,
    pub operation: TransformOperation,
    pub source_token_ids: Vec<TokenId>,
    pub before_text: String,
    pub after_text: String,
    pub rule_id: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransformLog {
    pub job_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_revision_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_content_hash: Option<String>,
    pub records: Vec<TransformRecord>,
}

impl TransformLog {
    pub fn new(job_id: impl Into<String>) -> Self {
        Self {
            job_id: job_id.into(),
            raw_revision_id: None,
            raw_content_hash: None,
            records: Vec::new(),
        }
    }

    pub fn record(
        &mut self,
        stage: TransformStage,
        operation: TransformOperation,
        source_token_ids: Vec<TokenId>,
        before_text: impl Into<String>,
        after_text: impl Into<String>,
        rule_id: impl Into<String>,
        confidence: f32,
    ) {
        self.records.push(TransformRecord {
            stage,
            operation,
            source_token_ids,
            before_text: before_text.into(),
            after_text: after_text.into(),
            rule_id: rule_id.into(),
            confidence,
        });
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

use serde::{Deserialize, Serialize};
use super::token::{TimeSpan, TokenId};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Provenance {
    pub source_token_ids: Vec<TokenId>,
    pub source_span: TimeSpan,
}

impl Provenance {
    pub fn single(token_id: TokenId, span: TimeSpan) -> Self {
        Self {
            source_token_ids: vec![token_id],
            source_span: span,
        }
    }

    pub fn multiple(token_ids: Vec<TokenId>, span: TimeSpan) -> Self {
        Self {
            source_token_ids: token_ids,
            source_span: span,
        }
    }

    pub fn merge(&self, other: &Provenance) -> Self {
        let mut ids = self.source_token_ids.clone();
        for id in &other.source_token_ids {
            if !ids.contains(id) {
                ids.push(*id);
            }
        }
        ids.sort_unstable();
        Self {
            source_token_ids: ids,
            source_span: self.source_span.merge(&other.source_span),
        }
    }
}

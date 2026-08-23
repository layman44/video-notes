use serde::{Deserialize, Serialize};
use crate::transcript::model::RawTranscript;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewSegment {
    pub id: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub translated_text: Option<String>,
}

pub fn render_raw_view(raw: &RawTranscript) -> Vec<ViewSegment> {
    raw.segments
        .iter()
        .map(|s| ViewSegment {
            id: s.id.clone(),
            start_ms: s.start_ms,
            end_ms: s.end_ms,
            text: s.text.clone(),
            translated_text: None,
        })
        .collect()
}

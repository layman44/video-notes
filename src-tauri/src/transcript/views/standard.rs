use crate::transcript::model::CanonicalTranscript;
use super::raw::ViewSegment;

pub fn render_standard_view(canonical: &CanonicalTranscript) -> Vec<ViewSegment> {
    canonical
        .segments
        .iter()
        .map(|s| ViewSegment {
            id: s.id.clone(),
            start_ms: s.start_ms,
            end_ms: s.end_ms,
            text: s.text.clone(),
            translated_text: s.translated_text.clone(),
        })
        .collect()
}

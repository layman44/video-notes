use crate::transcript::model::{CanonicalTranscript, LanguageProfile};

pub fn render_note_input_view(canonical: &CanonicalTranscript) -> String {
    let profile = LanguageProfile::from_language_tag(canonical.language.as_deref());
    let mut paragraphs: Vec<String> = Vec::new();
    let mut current: Vec<String> = Vec::new();
    let mut last_end: Option<u64> = None;

    for seg in &canonical.segments {
        let text = seg.text.trim();
        if text.is_empty() { continue; }
        let gap = last_end.map(|e| seg.start_ms.saturating_sub(e)).unwrap_or(0);
        let curr_len: usize = current.iter().map(|s| s.chars().count()).sum();
        if !current.is_empty() && (gap > 2_500 || curr_len >= 220) {
            paragraphs.push(join_sentences(&current, profile));
            current.clear();
        }
        current.push(text.to_string());
        last_end = Some(seg.end_ms);
    }
    if !current.is_empty() { paragraphs.push(join_sentences(&current, profile)); }
    paragraphs.join("\n\n")
}

fn join_sentences(parts: &[String], profile: LanguageProfile) -> String {
    if profile.prefers_cjk_spacing() {
        parts.join("")
    } else {
        parts.join(" ")
    }
}

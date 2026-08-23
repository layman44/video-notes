use std::collections::HashMap;

use crate::transcript::model::{LanguageProfile, RawSegment};
use crate::transcript::pipeline::PunctuationRepairEvidence;
use crate::transcript::transform::{TransformLog, TransformOperation, TransformStage};

const MIN_CONFIDENCE: f32 = 0.58;

#[derive(Debug, Clone)]
enum Edit {
    Remove { offset: usize },
    Boundary { offset: usize, confidence: f32 },
}

/// Applies only punctuation movement proven by the CTC/punctuation repair path. A plain
/// acoustic pause never enters this function, so `Please | have some` remains one sentence
/// unless an actual strong mark is relocated to that position.
pub fn apply_punctuation_repairs(
    mut segments: Vec<RawSegment>,
    profile: LanguageProfile,
    repairs: &[PunctuationRepairEvidence],
    log: &mut TransformLog,
) -> Vec<RawSegment> {
    let mut edits: HashMap<String, Vec<Edit>> = HashMap::new();
    for r in repairs.iter().filter(|r| r.confidence >= MIN_CONFIDENCE) {
        edits.entry(r.segment_id.clone()).or_default().push(Edit::Boundary {
            offset: r.char_offset,
            confidence: r.confidence,
        });
        if let (Some(id), Some(offset)) = (&r.remove_segment_id, r.remove_char_offset) {
            edits.entry(id.clone()).or_default().push(Edit::Remove { offset });
        }
    }

    for seg in &mut segments {
        let Some(mut seg_edits) = edits.remove(&seg.id) else { continue };
        let before = seg.text.clone();
        let mut chars: Vec<char> = seg.text.chars().collect();
        // Descending offsets keep all offsets valid against the original surface. At equal offsets,
        // removal happens first so relocation can install the authoritative mark afterwards.
        seg_edits.sort_by(|a, b| edit_offset(b).cmp(&edit_offset(a)).then_with(|| edit_rank(a).cmp(&edit_rank(b))));
        let mut max_conf = 0.0f32;
        for edit in seg_edits {
            match edit {
                Edit::Remove { offset } => {
                    if offset < chars.len() && is_sentence_punctuation(chars[offset]) {
                        chars.remove(offset);
                    }
                }
                Edit::Boundary { offset, confidence } => {
                    max_conf = max_conf.max(confidence);
                    let len = chars.len();
                    apply_boundary_mark(&mut chars, offset.min(len), profile);
                }
            }
        }
        seg.text = chars.into_iter().collect::<String>();
        seg.text = normalize_spaces(&seg.text, profile);
        if seg.text != before {
            let source_ids = seg.tokens.iter().map(|t| t.id).collect::<Vec<_>>();
            log.record(
                TransformStage::Boundary,
                TransformOperation::NormalizePunctuation,
                source_ids,
                before,
                seg.text.clone(),
                "ctc_punctuation_relocation",
                max_conf.max(MIN_CONFIDENCE),
            );
        }
    }
    segments
}

fn edit_offset(e: &Edit) -> usize {
    match e { Edit::Remove { offset } | Edit::Boundary { offset, .. } => *offset }
}
fn edit_rank(e: &Edit) -> u8 { match e { Edit::Remove { .. } => 0, Edit::Boundary { .. } => 1 } }

fn apply_boundary_mark(chars: &mut Vec<char>, offset: usize, profile: LanguageProfile) {
    if chars.is_empty() { return; }
    let mut left = offset;
    while left > 0 && chars[left - 1].is_whitespace() { left -= 1; }
    if left == 0 { return; }
    let prev_idx = left - 1;
    let prev = chars[prev_idx];
    if is_strong(prev) { return; }
    let mark = if profile.prefers_cjk_spacing() { '。' } else { '.' };
    if is_weak(prev) {
        chars[prev_idx] = mark;
        return;
    }
    chars.insert(left, mark);
    if !profile.prefers_cjk_spacing() {
        let after = left + 1;
        if after < chars.len() && !chars[after].is_whitespace() && !is_closing(chars[after]) {
            chars.insert(after, ' ');
        }
    }
}

fn normalize_spaces(text: &str, profile: LanguageProfile) -> String {
    if profile.prefers_cjk_spacing() {
        text.trim().to_string()
    } else {
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    }
}
fn is_strong(c: char) -> bool { matches!(c, '.' | '!' | '?' | '。' | '！' | '？') }
fn is_weak(c: char) -> bool { matches!(c, ',' | ';' | ':' | '，' | '；' | '：' | '、') }
fn is_sentence_punctuation(c: char) -> bool { is_strong(c) || is_weak(c) }
fn is_closing(c: char) -> bool { matches!(c, ')' | ']' | '}' | '】' | '》' | '〉' | '”' | '’' | '"' | '\'') }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::model::RawSegment;

    #[test]
    fn relocation_moves_existing_strong_mark_to_authoritative_boundary() {
        let text = "Yet she refused to buy new ones every morning although it was late.";
        let boundary = text[..text.find("although").unwrap()].chars().count();
        let remove = text[..text.rfind('.').unwrap()].chars().count();
        let seg = RawSegment {
            id: "s".into(), start_ms: 0, end_ms: 2_000, text: text.into(), tokens: vec![],
        };
        let out = apply_punctuation_repairs(
            vec![seg],
            LanguageProfile::En,
            &[PunctuationRepairEvidence {
                segment_id: "s".into(),
                char_offset: boundary,
                remove_segment_id: Some("s".into()),
                remove_char_offset: Some(remove),
                time_ms: 900,
                confidence: 0.9,
            }],
            &mut TransformLog::new("j"),
        );
        assert_eq!(
            out[0].text,
            "Yet she refused to buy new ones every morning. although it was late"
        );
    }
}

use crate::transcript::model::{LanguageProfile, RawSegment, RawTranscript};
use crate::transcript::transform::{TransformLog, TransformOperation, TransformStage};

/// Integrity is deliberately conservative: it tags suspicious content but does not delete real speech
/// such as fillers ("嗯", "uh") or short cross-lingual utterances.
pub fn run_integrity_guard(
    transcript: &RawTranscript,
    profile: LanguageProfile,
    log: &mut TransformLog,
) -> Vec<RawSegment> {
    let mut out = Vec::with_capacity(transcript.segments.len());

    for seg in &transcript.segments {
        let text = seg.text.trim();
        if text.is_empty() {
            continue;
        }

        let avg_conf = average_confidence(seg);
        let source_ids = seg.tokens.iter().map(|t| t.id).collect();

        if is_probable_decode_loop(text) && avg_conf.map_or(true, |c| c < 0.55) {
            log.record(
                TransformStage::Integrity,
                TransformOperation::TagAnomaly,
                source_ids,
                text,
                text,
                "probable_decode_loop",
                0.90,
            );
        } else if is_short_script_outlier(text, profile) && avg_conf.is_some_and(|c| c < 0.45) {
            log.record(
                TransformStage::Integrity,
                TransformOperation::TagAnomaly,
                source_ids,
                text,
                text,
                "low_confidence_script_outlier",
                0.75,
            );
        } else if is_filler_only(text) {
            // Fillers are real speech. Keep them in Canonical/Standard; downstream notes may summarize them away without rewriting the transcript.
            log.record(
                TransformStage::Integrity,
                TransformOperation::TagAnomaly,
                source_ids,
                text,
                text,
                "filler_only_segment_preserved",
                0.60,
            );
        }

        out.push(seg.clone());
    }

    out
}

fn average_confidence(seg: &RawSegment) -> Option<f32> {
    if seg.tokens.is_empty() { return None; }
    Some(seg.tokens.iter().map(|t| t.confidence).sum::<f32>() / seg.tokens.len() as f32)
}

fn is_probable_decode_loop(text: &str) -> bool {
    let chars: Vec<char> = text.chars().filter(|c| !c.is_whitespace() && !c.is_ascii_punctuation()).collect();
    if chars.len() < 8 { return false; }
    for n in 1..=4.min(chars.len() / 4) {
        let pattern = &chars[..n];
        let repeats = chars.chunks(n).take_while(|chunk| *chunk == pattern).count();
        if repeats >= 4 && repeats * n >= chars.len().saturating_sub(n) { return true; }
    }
    false
}

fn is_short_script_outlier(text: &str, profile: LanguageProfile) -> bool {
    let cjk = text.chars().filter(|&c| crate::transcript::pipeline::edit::is_cjk(c)).count();
    let latin = text.chars().filter(|c| c.is_ascii_alphabetic()).count();
    match profile {
        LanguageProfile::En => cjk > 0 && latin == 0 && cjk <= 6,
        LanguageProfile::Zh => latin > 0 && cjk == 0 && latin <= 3,
        _ => false,
    }
}

fn is_filler_only(text: &str) -> bool {
    let compact: String = text.chars().filter(|c| !c.is_whitespace() && !is_punct(*c)).collect();
    matches!(compact.as_str(),
        "啊" | "呃" | "额" | "嗯" | "哦" | "噢" | "喔" | "哎" | "欸" | "唉" | "呀" | "哇" | "哈" | "嘿" |
        "uh" | "um" | "erm" | "hmm" | "hm")
}

fn is_punct(ch: char) -> bool {
    ch.is_ascii_punctuation() || matches!(ch, '，' | '。' | '！' | '？' | '：' | '；' | '、' | '…' | '—')
}

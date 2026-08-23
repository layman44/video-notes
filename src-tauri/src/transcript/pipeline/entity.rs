use regex::Regex;
use std::sync::LazyLock;

use crate::transcript::model::{CanonicalSegment, CanonicalToken, LanguageProfile};
use crate::transcript::pipeline::edit::{apply_text_replacements, merged_source_ids, refresh_segment_text, synthetic_token_id, TextReplacement};
use crate::transcript::transform::{TransformLog, TransformOperation, TransformStage};

// Conservative generic candidate: 3-6 separately spoken uppercase Latin letters.
// Two-letter sequences are intentionally not auto-joined because "A B test" and "A/B test" are ambiguous.
static SPACED_ACRONYM_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:[A-Z]\s+){2,5}[A-Z]\b").unwrap()
});

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlossaryEntry {
    pub canonical: String,
    pub aliases: Vec<String>,
}

pub fn run_entity_resolver(
    segments: &mut [CanonicalSegment],
    profile: LanguageProfile,
    log: &mut TransformLog,
) {
    for seg in segments.iter_mut() {
        stitch_precise_acronym_tokens(seg, profile, log);

        // Fallback for segment-level ASR without precise raw tokens.
        if !seg.tokens.iter().any(|t| !t.provenance.source_token_ids.is_empty()) {
            let replacements: Vec<_> = SPACED_ACRONYM_RE
                .find_iter(&seg.text)
                .map(|m| TextReplacement {
                    range: m.range(),
                    replacement: m.as_str().split_whitespace().collect::<Vec<_>>().join(""),
                    operation: TransformOperation::StitchAcronym,
                    rule_id: "generic_spaced_acronym_3plus",
                    confidence: 0.95,
                })
                .collect();
            apply_text_replacements(seg, profile, TransformStage::Entity, &replacements, log);
        }
    }
}

/// Optional external glossary. Core pipeline never hard-codes domain names such as CLTC, drug names or library names.
pub fn apply_glossary(
    segments: &mut [CanonicalSegment],
    entries: &[GlossaryEntry],
    profile: LanguageProfile,
    log: &mut TransformLog,
) {
    for seg in segments.iter_mut() {
        let mut replacements = Vec::new();
        for entry in entries {
            for alias in &entry.aliases {
                if alias.is_empty() || alias == &entry.canonical { continue; }
                for (start, _) in seg.text.match_indices(alias) {
                    replacements.push(TextReplacement {
                        range: start..start + alias.len(),
                        replacement: entry.canonical.clone(),
                        operation: TransformOperation::NormalizeEntity,
                        rule_id: "external_glossary_alias",
                        confidence: 1.0,
                    });
                }
            }
        }
        // Keep non-overlapping matches only, preferring earlier/longer entries.
        replacements.sort_by_key(|r| (r.range.start, std::cmp::Reverse(r.range.end - r.range.start)));
        let mut accepted = Vec::new();
        let mut last_end = 0usize;
        for r in replacements {
            if r.range.start >= last_end {
                last_end = r.range.end;
                accepted.push(r);
            }
        }
        apply_text_replacements(seg, profile, TransformStage::Entity, &accepted, log);
    }
}

fn stitch_precise_acronym_tokens(seg: &mut CanonicalSegment, profile: LanguageProfile, log: &mut TransformLog) {
    let mut i = 0usize;
    while i < seg.tokens.len() {
        if !is_single_upper_letter(&seg.tokens[i].text) { i += 1; continue; }
        let mut j = i + 1;
        while j < seg.tokens.len()
            && is_single_upper_letter(&seg.tokens[j].text)
            && seg.tokens[j].start_ms.saturating_sub(seg.tokens[j - 1].end_ms) <= 500
            && j - i < 6
        {
            j += 1;
        }
        let count = j - i;
        if count >= 3 {
            let source_ids = merged_source_ids(&seg.tokens[i..j]);
            let before = seg.tokens[i..j].iter().map(|t| t.text.trim()).collect::<Vec<_>>().join(" ");
            let joined = seg.tokens[i..j].iter().map(|t| t.text.trim()).collect::<Vec<_>>().join("");
            let provenance = seg.tokens[i..j].iter().map(|t| t.provenance.clone()).reduce(|a, b| a.merge(&b)).unwrap();
            let token = CanonicalToken {
                id: synthetic_token_id(&seg.id, (i, j, &joined, "acronym")),
                text: joined.clone(),
                start_ms: seg.tokens[i].start_ms,
                end_ms: seg.tokens[j - 1].end_ms,
                provenance,
            };
            seg.tokens.splice(i..j, [token]);
            refresh_segment_text(seg, profile);
            log.record(
                TransformStage::Entity,
                TransformOperation::StitchAcronym,
                source_ids,
                before,
                joined,
                "generic_spoken_acronym_3plus",
                0.98,
            );
            i += 1;
        } else {
            i = j;
        }
    }
}

fn is_single_upper_letter(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed.len() == 1 && trimmed.as_bytes()[0].is_ascii_uppercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::model::{Provenance, TimeSpan};

    #[test]
    fn generic_acronym_does_not_invent_domain_correction() {
        let mut seg = CanonicalSegment {
            id: "s".into(), start_ms: 0, end_ms: 400, text: String::new(), translated_text: None,
            tokens: ["C", "O", "T", "C"].into_iter().enumerate().map(|(i, t)| CanonicalToken {
                id: i as u64 + 10, text: t.into(), start_ms: i as u64 * 100, end_ms: i as u64 * 100 + 80,
                provenance: Provenance::single(i as u64 + 1, TimeSpan::new(i as u64 * 100, i as u64 * 100 + 80)),
            }).collect(),
        };
        refresh_segment_text(&mut seg, LanguageProfile::En);
        let mut log = TransformLog::new("t");
        run_entity_resolver(std::slice::from_mut(&mut seg), LanguageProfile::En, &mut log);
        assert_eq!(seg.text, "COTC");
        assert_ne!(seg.text, "CLTC");
    }
}

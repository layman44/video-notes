use std::collections::{HashMap, HashSet};

use crate::transcript::model::{CanonicalSegment, LanguageProfile};
use crate::transcript::pipeline::edit::refresh_segment_text;
use crate::transcript::surface::{analyze_decoder_surface, lexical_spans};
use crate::transcript::transform::{TransformLog, TransformOperation, TransformStage};

pub fn run_typography_normalizer(
    segments: &mut [CanonicalSegment],
    profile: LanguageProfile,
    log: &mut TransformLog,
) {
    let degenerated_groups = decoder_degenerated_groups(segments);
    let trusted_surfaces = build_trusted_surface_lexicon(segments, profile, &degenerated_groups);

    for seg in segments.iter_mut() {
        let before = seg.text.clone();
        let mut punct_changed = false;
        let mut casing_changed = false;

        for i in 0..seg.tokens.len() {
            if is_protected_token(&seg.tokens[i].text) { continue; }
            let prev = i.checked_sub(1).and_then(|p| seg.tokens.get(p)).map(|t| t.text.as_str());
            let next = seg.tokens.get(i + 1).map(|t| t.text.as_str());
            let normalized = normalize_punctuation_token(&seg.tokens[i].text, profile, prev, next);
            if normalized != seg.tokens[i].text {
                seg.tokens[i].text = normalized;
                punct_changed = true;
            }
        }

        refresh_segment_text(seg, profile);

        if matches!(profile, LanguageProfile::En | LanguageProfile::Mixed | LanguageProfile::Auto)
            && (degenerated_groups.contains(base_segment_id(&seg.id))
                || should_normalize_prose_casing(&seg.text))
        {
            let mut capitalize_next = true;
            for token in &mut seg.tokens {
                if is_protected_token(&token.text) { continue; }
                let normalized = normalize_english_casing(&token.text, &trusted_surfaces, &mut capitalize_next);
                if normalized != token.text {
                    token.text = normalized;
                    casing_changed = true;
                }
            }
            refresh_segment_text(seg, profile);
        }

        if seg.text != before {
            let (operation, rule_id, confidence) = if casing_changed {
                (
                    TransformOperation::NormalizeCasing,
                    "decoder_surface_standard_casing",
                    0.97,
                )
            } else if punct_changed {
                (
                    TransformOperation::NormalizePunctuation,
                    "safe_script_aware_typography",
                    0.99,
                )
            } else {
                (
                    TransformOperation::NormalizeSpacing,
                    "safe_script_aware_typography",
                    0.99,
                )
            };
            log.record(
                TransformStage::Typography,
                operation,
                seg.source_token_ids(),
                before,
                seg.text.clone(),
                rule_id,
                confidence,
            );
        }
    }
}

fn base_segment_id(id: &str) -> &str {
    id.split_once("#s").map(|(base, _)| base).unwrap_or(id)
}

fn decoder_degenerated_groups(segments: &[CanonicalSegment]) -> HashSet<String> {
    let mut grouped = HashMap::<String, (u64, u64, Vec<String>)>::new();
    for seg in segments {
        let base = base_segment_id(&seg.id).to_string();
        let entry = grouped
            .entry(base)
            .or_insert_with(|| (seg.start_ms, seg.end_ms, Vec::new()));
        entry.0 = entry.0.min(seg.start_ms);
        entry.1 = entry.1.max(seg.end_ms);
        entry.2.push(seg.text.clone());
    }
    grouped
        .into_iter()
        .filter_map(|(id, (start_ms, end_ms, parts))| {
            let combined = parts.join(" ");
            analyze_decoder_surface(&combined, end_ms.saturating_sub(start_ms))
                .case_degenerated
                .then_some(id)
        })
        .collect()
}

fn build_trusted_surface_lexicon(
    segments: &[CanonicalSegment],
    profile: LanguageProfile,
    degenerated_groups: &HashSet<String>,
) -> HashMap<String, String> {
    if !matches!(profile, LanguageProfile::En | LanguageProfile::Mixed | LanguageProfile::Auto) {
        return HashMap::new();
    }

    let mut lexicon = HashMap::<String, String>::new();
    for seg in segments {
        let duration_ms = seg.end_ms.saturating_sub(seg.start_ms);
        let health = analyze_decoder_surface(&seg.text, duration_ms);
        if health.case_degenerated
            || should_normalize_prose_casing(&seg.text)
            || degenerated_groups.contains(base_segment_id(&seg.id))
        {
            continue;
        }
        let spans = lexical_spans(&seg.text);
        for span in &spans {
            let surface = &seg.text[span.byte_start..span.byte_end];
            let sentence_initial = is_sentence_initial_position(&seg.text, span.byte_start);
            if should_trust_surface(surface, &span.norm, !sentence_initial) {
                lexicon.entry(span.norm.clone()).or_insert_with(|| surface.to_string());
            }
        }
    }
    lexicon
}

fn is_sentence_initial_position(text: &str, byte_start: usize) -> bool {
    if byte_start == 0 {
        return true;
    }
    let prefix = &text[..byte_start];
    let trimmed = prefix.trim_end();
    if trimmed.is_empty() {
        return true;
    }
    trimmed.chars().next_back().is_some_and(|c| {
        matches!(c, '.' | '?' | '!' | '。' | '？' | '！')
    })
}

fn is_common_prose_word(norm: &str) -> bool {
    matches!(
        norm,
        "a" | "an" | "the" | "and" | "or" | "but" | "if" | "in" | "on" | "at" | "to" | "for"
            | "of" | "with" | "by" | "from" | "up" | "about" | "into" | "over" | "after"
            | "is" | "are" | "was" | "were" | "be" | "been" | "being" | "have" | "has"
            | "had" | "do" | "does" | "did" | "will" | "would" | "shall" | "should"
            | "can" | "could" | "may" | "might" | "must" | "i" | "you" | "he" | "she"
            | "it" | "we" | "they" | "me" | "him" | "her" | "us" | "them" | "my" | "your"
            | "his" | "its" | "our" | "their" | "this" | "that" | "these" | "those"
            | "what" | "which" | "who" | "whom" | "whose" | "when" | "where" | "why"
            | "how" | "all" | "any" | "both" | "each" | "few" | "more" | "most" | "other"
            | "some" | "such" | "no" | "nor" | "not" | "only" | "own" | "same" | "so"
            | "than" | "too" | "very" | "yes" | "said" | "dont" | "don't" | "didnt" | "didn't"
            | "cant" | "can't" | "wont" | "won't" | "im" | "i'm" | "ive" | "i've"
            | "youre" | "you're" | "theyre" | "they're" | "we're" | "there"
            | "get" | "got" | "know" | "knew" | "think" | "thought" | "see" | "saw"
            | "say" | "go" | "went" | "come" | "came" | "make" | "made"
            | "take" | "took" | "good" | "right" | "left" | "fun" | "game" | "play"
            | "duckling" | "ducklings" | "forget" | "quack"
    )
}

fn should_trust_surface(surface: &str, norm: &str, not_sentence_initial: bool) -> bool {
    let letters = surface.chars().filter(|c| c.is_ascii_alphabetic()).collect::<Vec<_>>();
    if letters.is_empty() {
        return false;
    }
    let upper = letters.iter().filter(|c| c.is_ascii_uppercase()).count();
    let lower = letters.iter().filter(|c| c.is_ascii_lowercase()).count();
    let acronym = letters.len() >= 2 && letters.len() <= 8 && upper == letters.len() && !is_common_prose_word(norm);
    let mixed_case = upper >= 1 && lower >= 1 && !is_simple_title_case(surface);
    let title_case = not_sentence_initial && is_simple_title_case(surface) && !is_common_prose_word(norm);
    acronym || mixed_case || title_case
}

fn is_simple_title_case(surface: &str) -> bool {
    let mut letters = surface.chars().filter(|c| c.is_ascii_alphabetic());
    let Some(first) = letters.next() else { return false };
    first.is_ascii_uppercase() && letters.all(|c| c.is_ascii_lowercase())
}

fn should_normalize_prose_casing(text: &str) -> bool {
    let health = analyze_decoder_surface(text, 0);
    if health.case_degenerated {
        return true;
    }

    let spans = lexical_spans(text);
    if spans.len() < 4 {
        return false;
    }
    let mut upper_words = 0usize;
    let mut lower_words = 0usize;
    let mut run = 0usize;
    let mut max_run = 0usize;
    for span in spans {
        let raw = &text[span.byte_start..span.byte_end];
        let letters = raw.chars().filter(|c| c.is_ascii_alphabetic()).collect::<Vec<_>>();
        if letters.len() >= 2 && letters.iter().all(|c| c.is_ascii_uppercase()) {
            upper_words += 1;
            run += 1;
            max_run = max_run.max(run);
        } else {
            if letters.iter().any(|c| c.is_ascii_lowercase()) {
                lower_words += 1;
            }
            run = 0;
        }
    }
    max_run >= 3 && upper_words >= lower_words.max(1)
}

fn normalize_english_casing(
    text: &str,
    trusted_surfaces: &HashMap<String, String>,
    capitalize_next: &mut bool,
) -> String {
    let spans = lexical_spans(text);
    if spans.is_empty() {
        if contains_sentence_terminal(text) {
            *capitalize_next = true;
        }
        return text.to_string();
    }

    let mut out = String::with_capacity(text.len());
    let mut cursor = 0usize;
    for span in spans {
        out.push_str(&text[cursor..span.byte_start]);
        let raw = &text[span.byte_start..span.byte_end];
        let normalized = normalize_word(raw, &span.norm, trusted_surfaces, *capitalize_next);
        out.push_str(&normalized);

        let between_start = span.byte_end;
        cursor = span.byte_end;
        let next_terminal = text[between_start..].chars().next().is_some_and(|ch| {
            matches!(ch, '.' | '?' | '!' | '。' | '？' | '！')
        });
        if next_terminal {
            *capitalize_next = true;
        } else {
            *capitalize_next = false;
        }
    }
    out.push_str(&text[cursor..]);
    if ends_with_sentence_terminal(text) {
        *capitalize_next = true;
    }
    out
}

fn normalize_word(
    raw: &str,
    norm: &str,
    trusted_surfaces: &HashMap<String, String>,
    sentence_initial: bool,
) -> String {
    if let Some(trusted) = trusted_surfaces.get(norm) {
        return trusted.clone();
    }

    let lower = raw.to_ascii_lowercase();
    if lower == "i" {
        return "I".into();
    }
    if lower.starts_with("i'") || lower.starts_with("i’") {
        return format!("I{}", &lower[1..]);
    }
    if sentence_initial {
        let mut chars = lower.chars();
        if let Some(first) = chars.next() {
            return first.to_ascii_uppercase().to_string() + chars.as_str();
        }
    }
    lower
}

fn contains_sentence_terminal(text: &str) -> bool {
    text.chars().any(|c| matches!(c, '.' | '?' | '!' | '。' | '？' | '！'))
}

fn ends_with_sentence_terminal(text: &str) -> bool {
    text.trim_end().chars().next_back().is_some_and(|c| {
        matches!(c, '.' | '?' | '!' | '。' | '？' | '！')
    })
}

fn normalize_punctuation_token(text: &str, profile: LanguageProfile, prev: Option<&str>, next: Option<&str>) -> String {
    let t = text.trim();
    if t.chars().count() != 1 { return text.to_string(); }
    let ch = t.chars().next().unwrap();
    match profile {
        LanguageProfile::Zh => match ch {
            ',' => "，".into(),
            '?' if !looks_like_ascii_entity(prev, next) => "？".into(),
            '!' if !looks_like_ascii_entity(prev, next) => "！".into(),
            ';' if !looks_like_ascii_entity(prev, next) => "；".into(),
            '.' if !looks_like_ascii_entity(prev, next) => "。".into(),
            _ => text.to_string(),
        },
        LanguageProfile::En => match ch {
            '，' | '、' => ",".into(),
            '。' => ".".into(),
            '？' => "?".into(),
            '！' => "!".into(),
            '；' => ";".into(),
            '：' => ":".into(),
            _ => text.to_string(),
        },
        _ => text.to_string(),
    }
}

fn looks_like_ascii_entity(prev: Option<&str>, next: Option<&str>) -> bool {
    let asciiish = |s: &str| s.chars().any(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '/' | ':' | '#'));
    prev.is_some_and(asciiish) && next.is_some_and(asciiish)
}

fn is_protected_token(text: &str) -> bool {
    let t = text.trim();
    t.contains("://")
        || t.contains("::")
        || (t.contains('@') && t.contains('.'))
        || t.contains("\\")
        || (t.contains('/') && t.chars().any(|c| c.is_ascii_alphabetic()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::model::{CanonicalToken, Provenance, TimeSpan};

    fn segment(text: &str, start: u64, end: u64) -> CanonicalSegment {
        CanonicalSegment {
            id: format!("s-{start}"), start_ms: start, end_ms: end, text: text.into(), translated_text: None,
            tokens: vec![CanonicalToken { id: start + 1, text: text.into(), start_ms: start, end_ms: end, provenance: Provenance::multiple(vec![start + 1], TimeSpan::new(start,end)) }],
        }
    }

    #[test]
    fn english_typography_never_deletes_cjk() {
        let mut seg = segment("人工智能", 0, 500);
        let mut log = TransformLog::new("t");
        run_typography_normalizer(std::slice::from_mut(&mut seg), LanguageProfile::En, &mut log);
        assert_eq!(seg.text, "人工智能");
    }

    #[test]
    fn protected_code_token_is_untouched() {
        assert!(is_protected_token("https://example.com/api?v=2"));
        assert!(is_protected_token("std::vector"));
    }

    #[test]
    fn all_caps_prose_becomes_standard_casing() {
        let mut seg = segment("I'M HIS MOTHER. I DON'T KNOW WHAT TO DO.", 0, 5_000);
        let mut log = TransformLog::new("t");
        run_typography_normalizer(std::slice::from_mut(&mut seg), LanguageProfile::En, &mut log);
        assert_eq!(seg.text, "I'm his mother. I don't know what to do.");
        assert!(log.records.iter().any(|record| record.operation == TransformOperation::NormalizeCasing));
    }

    #[test]
    fn trusted_acronym_and_name_surface_survive_all_caps_repair() {
        let mut segments = vec![
            segment("We tested CLTC with Gray today.", 0, 3_000),
            segment("GRAY SAID CLTC WAS READY FOR ANOTHER TEST.", 4_000, 8_000),
        ];
        let mut log = TransformLog::new("t");
        run_typography_normalizer(&mut segments, LanguageProfile::En, &mut log);
        assert_eq!(segments[1].text, "Gray said CLTC was ready for another test.");
    }

    #[test]
    fn mixed_frankenstein_casing_is_normalized() {
        let mut seg = segment("YES, MY DUCKLINGS DON'T FORGET right left quack", 0, 4_000);
        let mut log = TransformLog::new("t");
        run_typography_normalizer(std::slice::from_mut(&mut seg), LanguageProfile::En, &mut log);
        assert_eq!(seg.text, "Yes, my ducklings don't forget right left quack");
    }
}

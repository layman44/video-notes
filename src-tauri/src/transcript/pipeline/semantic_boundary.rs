use crate::transcript::model::{CanonicalSegment, CanonicalToken, LanguageProfile};
use crate::transcript::pipeline::edit::{refresh_segment_text, synthetic_token_id};
use crate::transcript::transform::{TransformLog, TransformOperation, TransformStage};

const MAX_SEMANTIC_SEGMENT_WORDS: usize = 70;
const MAX_SEMANTIC_SEGMENT_CHARS: usize = 420;

/// Final conservative sentence-boundary pass for Standard/Canonical text.
///
/// The acoustic/strong-punctuation resolver intentionally runs earlier. This pass only repairs
/// boundaries that are structurally very likely to split one English semantic unit. It never
/// changes Raw and it does not try to rewrite arbitrary prose.
pub fn run_final_semantic_boundary_review(
    segments: &mut Vec<CanonicalSegment>,
    profile: LanguageProfile,
    log: &mut TransformLog,
) {
    if !matches!(profile, LanguageProfile::En | LanguageProfile::Mixed | LanguageProfile::Auto)
        || segments.len() < 2
    {
        return;
    }

    // A small fixed-point loop is enough for chains such as:
    // "the last time." | "She had laughed ..." | "Or shared ..."
    for _ in 0..4 {
        let mut changed = false;
        let mut i = 0usize;
        while i + 1 < segments.len() {
            if try_relocate_complement_boundary(segments, i, profile, log) {
                changed = true;
                i = i.saturating_sub(1);
                continue;
            }
            if should_merge_boundary(&segments[i], &segments[i + 1]) {
                merge_pair(segments, i, profile, log);
                changed = true;
                i = i.saturating_sub(1);
                continue;
            }
            i += 1;
        }
        if !changed {
            break;
        }
    }
}

fn should_merge_boundary(left: &CanonicalSegment, right: &CanonicalSegment) -> bool {
    let combined_chars = left.text.chars().count() + right.text.chars().count();
    let combined_words = english_words(&left.text).len() + english_words(&right.text).len();
    if combined_chars > MAX_SEMANTIC_SEGMENT_CHARS || combined_words > MAX_SEMANTIC_SEGMENT_WORDS {
        return false;
    }

    let left_text = left.text.trim();
    let right_text = right.text.trim();
    if left_text.is_empty() || right_text.is_empty() {
        return false;
    }

    let left_open = left_requires_continuation(left_text);
    if ends_question_or_exclamation(right_text) && !left_open {
        return false;
    }
    let right_fragment = right_is_continuation_fragment(right_text);
    let left_fragment = left_is_nominal_fragment(left_text);
    let relative_subject_fragment = left_is_relative_subject_fragment(left_text);

    // Do not erase a confident question/exclamation boundary unless the left side itself is an
    // obvious unfinished construction.
    if ends_question_or_exclamation(left_text) && !left_open {
        return false;
    }

    left_open
        || right_fragment
        || ((left_fragment || relative_subject_fragment) && right_starts_predicate_continuation(right_text))
}

fn left_requires_continuation(text: &str) -> bool {
    let words = english_words(text);
    if words.is_empty() {
        return false;
    }
    let last = words.last().map(String::as_str).unwrap_or_default();
    if matches!(
        last,
        "and" | "or" | "but" | "because" | "although" | "though" | "while" | "when"
            | "if" | "unless" | "until" | "since" | "as" | "than" | "to" | "of" | "for"
            | "with" | "without" | "from" | "into" | "onto" | "by" | "at" | "in" | "on"
            | "the" | "a" | "an" | "become" | "became" | "becomes" | "seem" | "seemed"
            | "seems" | "remain" | "remained" | "remains"
    ) {
        return true;
    }

    if starts_with_phrase(&words, &["instead", "of"]) && !has_main_clause_after_comma(text) {
        return true;
    }
    if starts_forward_subordinate_fragment(text) {
        return true;
    }
    if has_dangling_trailing_subordinate(text) {
        return true;
    }

    ends_with_phrase(&words, &["the", "last", "time"])
        || ends_with_phrase(&words, &["the", "first", "time"])
        || ends_with_phrase(&words, &["the", "only", "thing"])
        || ends_with_phrase(&words, &["one", "thing"])
        || ends_with_phrase(&words, &["such", "as"])
        || ends_with_phrase(&words, &["in", "order", "to"])
}

fn right_is_continuation_fragment(text: &str) -> bool {
    let words = english_words(text);
    let Some(first) = words.first().map(String::as_str) else { return false };

    if matches!(first, "while" | "when" | "as") {
        // These temporal clauses commonly trail the previous main clause. Other subordinators
        // such as "Because ..." are handled as forward-attaching fragments when they become the
        // left segment, avoiding "A. Because X." being incorrectly glued to A.
        return !has_main_clause_after_comma(text);
    }

    if matches!(first, "and" | "or" | "but" | "nor" | "yet") {
        if starts_with_phrase(&words, &["yet", "despite"]) {
            return false;
        }
        // Preserve deliberate new sentences with an explicit subject after the coordinator.
        return !words.get(1).is_some_and(|word| is_explicit_subject(word));
    }

    false
}

fn right_starts_predicate_continuation(text: &str) -> bool {
    let words = english_words(text);
    let Some(first) = words.first().map(String::as_str) else { return false };
    matches!(
        first,
        "always" | "never" | "often" | "usually" | "still" | "also" | "then"
            | "am" | "is" | "are" | "was" | "were" | "be" | "been" | "being"
    ) || (first.ends_with("ed") && first.len() > 3)
}

fn left_is_nominal_fragment(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let words = english_words(&lower);
    if words.len() < 3 || words.len() > 16 {
        return false;
    }

    let has_aux = words.iter().any(|word| matches!(
        word.as_str(),
        "am" | "is" | "are" | "was" | "were" | "be" | "been" | "being" | "have" | "has"
            | "had" | "do" | "does" | "did" | "can" | "could" | "will" | "would" | "shall"
            | "should" | "may" | "might" | "must"
    ));
    let has_obvious_past = words.iter().any(|word| word.ends_with("ed"));
    if has_aux || has_obvious_past {
        return false;
    }

    // Introductory prepositional/adverbial phrase followed by a noun phrase, but no predicate.
    lower.contains(',')
        && matches!(
            words.first().map(String::as_str),
            Some("from" | "in" | "on" | "at" | "after" | "before" | "during" | "for" | "with")
        )
        || starts_with_phrase(&words, &["just", "then"])
}

fn left_is_relative_subject_fragment(text: &str) -> bool {
    let words = english_words(text);
    if words.len() < 4 || words.len() > 24 {
        return false;
    }
    let starts_nominal = matches!(
        words.first().map(String::as_str),
        Some("the" | "a" | "an" | "this" | "that" | "these" | "those" | "my" | "your" | "his" | "her" | "our" | "their")
    );
    let has_relative = words.iter().any(|word| matches!(word.as_str(), "who" | "which" | "that"));
    starts_nominal && has_relative
}

fn starts_forward_subordinate_fragment(text: &str) -> bool {
    if has_main_clause_after_comma(text) {
        return false;
    }
    let words = english_words(text);
    matches!(
        words.first().map(String::as_str),
        Some("because" | "although" | "though" | "unless" | "until" | "if" | "despite")
    ) || starts_with_phrase(&words, &["yet", "despite"])
}

fn has_dangling_trailing_subordinate(text: &str) -> bool {
    let Some((_, tail)) = text.split_once(',') else { return false };
    if tail.contains(',') {
        return false;
    }
    let words = english_words(tail);
    matches!(
        words.first().map(String::as_str),
        Some("as" | "when" | "while" | "because" | "although" | "though" | "if" | "unless" | "until")
    )
}

/// Repairs a particularly damaging false stop:
/// "... had become." | "Completely useless as she ..."
/// -> "... had become completely useless." | "As she ..."
///
/// This is grammatical-class based rather than phrase-specific: a linking verb on the left may
/// absorb a short complement prefix from the right up to a subordinate/coordinating clause marker.
fn try_relocate_complement_boundary(
    segments: &mut Vec<CanonicalSegment>,
    index: usize,
    profile: LanguageProfile,
    log: &mut TransformLog,
) -> bool {
    let left_words = english_words(&segments[index].text);
    let Some(last) = left_words.last().map(String::as_str) else { return false };
    if !matches!(last, "become" | "became" | "becomes" | "seem" | "seemed" | "seems" | "remain" | "remained" | "remains" | "feel" | "felt" | "look" | "looked" | "sound" | "sounded") {
        return false;
    }

    let marker = find_clause_marker_token_index(&segments[index + 1].tokens);
    let Some(marker_token_index) = marker else { return false };
    if marker_token_index == 0 {
        return false;
    }

    let prefix_word_count = segments[index + 1].tokens[..marker_token_index]
        .iter()
        .flat_map(|t| english_words(&t.text))
        .count();
    if prefix_word_count == 0 || prefix_word_count > 6 {
        return false;
    }

    let before = format!("{} | {}", segments[index].text, segments[index + 1].text);
    let mut moved = segments[index + 1].tokens.drain(..marker_token_index).collect::<Vec<_>>();
    if moved.is_empty() {
        return false;
    }
    lowercase_continuation_initial(&mut moved);

    remove_false_terminal_period(&mut segments[index]);
    segments[index].tokens.append(&mut moved);
    refresh_segment_text(&mut segments[index], profile);
    append_period_if_missing(&mut segments[index]);
    refresh_segment_text(&mut segments[index], profile);
    capitalize_first_ascii_word(&mut segments[index + 1]);
    refresh_segment_text(&mut segments[index + 1], profile);

    if segments[index + 1].text.trim().is_empty() {
        let right = segments.remove(index + 1);
        segments[index].tokens.extend(right.tokens);
        refresh_segment_text(&mut segments[index], profile);
    }

    let after = if index + 1 < segments.len() {
        format!("{} | {}", segments[index].text, segments[index + 1].text)
    } else {
        segments[index].text.clone()
    };
    let source_ids = segments[index].source_token_ids();
    log.record(
        TransformStage::SemanticBoundary,
        TransformOperation::RelocateBoundary,
        source_ids,
        before,
        after,
        "english_linking_complement_boundary_relocation",
        0.92,
    );
    true
}

fn merge_pair(
    segments: &mut Vec<CanonicalSegment>,
    index: usize,
    profile: LanguageProfile,
    log: &mut TransformLog,
) {
    let mut right = segments.remove(index + 1);
    let before = format!("{} | {}", segments[index].text, right.text);
    let mut source_ids = segments[index].source_token_ids();
    source_ids.extend(right.source_token_ids());
    source_ids.sort_unstable();
    source_ids.dedup();

    if needs_clause_join_comma(&segments[index].text) {
        replace_false_terminal_period_with_comma(&mut segments[index]);
    } else {
        remove_false_terminal_period(&mut segments[index]);
    }
    lowercase_continuation_initial(&mut right.tokens);
    segments[index].tokens.extend(right.tokens);
    segments[index].translated_text = None;
    refresh_segment_text(&mut segments[index], profile);

    log.record(
        TransformStage::SemanticBoundary,
        TransformOperation::MergeSemanticBoundary,
        source_ids,
        before,
        segments[index].text.clone(),
        "english_incomplete_semantic_unit_merge",
        0.90,
    );
}

fn needs_clause_join_comma(text: &str) -> bool {
    let words = english_words(text);
    (starts_with_phrase(&words, &["instead", "of"]) && !has_main_clause_after_comma(text))
        || starts_forward_subordinate_fragment(text)
        || has_dangling_trailing_subordinate(text)
}

fn replace_false_terminal_period_with_comma(segment: &mut CanonicalSegment) {
    let Some(last_index) = segment.tokens.len().checked_sub(1) else { return };
    let trimmed = segment.tokens[last_index].text.trim_end().to_string();
    if trimmed == "." {
        segment.tokens[last_index].text = ",".to_string();
        return;
    }
    if trimmed.ends_with('.') && !trimmed.ends_with("...") {
        let mut text = segment.tokens[last_index].text.clone();
        while text.ends_with('.') {
            text.pop();
        }
        text.push(',');
        segment.tokens[last_index].text = text;
    }
}

fn remove_false_terminal_period(segment: &mut CanonicalSegment) {
    let Some(last_index) = segment.tokens.len().checked_sub(1) else { return };
    let trimmed = segment.tokens[last_index].text.trim_end().to_string();
    if trimmed == "." {
        segment.tokens.pop();
        return;
    }
    if trimmed.ends_with('.') && !trimmed.ends_with("...") {
        let mut text = segment.tokens[last_index].text.clone();
        while text.ends_with('.') {
            text.pop();
        }
        segment.tokens[last_index].text = text.trim_end().to_string();
        if segment.tokens[last_index].text.is_empty() {
            segment.tokens.pop();
        }
    }
}

fn append_period_if_missing(segment: &mut CanonicalSegment) {
    let already_terminal = segment.tokens.iter().rev().find_map(|token| {
        token.text.chars().rev().find(|c| !c.is_whitespace())
    }).is_some_and(|c| matches!(c, '.' | '?' | '!'));
    if already_terminal {
        return;
    }
    let time = segment.tokens.last().map(|t| t.end_ms).unwrap_or(segment.end_ms);
    let span = segment.span();
    segment.tokens.push(CanonicalToken {
        id: synthetic_token_id(&segment.id, ("semantic-period", time, segment.tokens.len())),
        text: ".".to_string(),
        start_ms: time,
        end_ms: time,
        provenance: crate::transcript::model::Provenance::multiple(Vec::new(), span),
    });
}

fn lowercase_continuation_initial(tokens: &mut [CanonicalToken]) {
    for token in tokens {
        let Some((pos, ch)) = token.text.char_indices().find_map(|(idx, ch)| ch.is_ascii_alphabetic().then_some((idx, ch))) else { continue };
        let word = token.text[pos..]
            .split(|c: char| !c.is_ascii_alphabetic())
            .next()
            .unwrap_or_default();
        let lower = word.to_ascii_lowercase();
        let should_lower = matches!(
            lower.as_str(),
            "she" | "he" | "it" | "we" | "they" | "you" | "this" | "that" | "these" | "those"
                | "and" | "or" | "but" | "because" | "although" | "though" | "while" | "when" | "as"
                | "always" | "never" | "often" | "usually" | "still" | "also" | "then"
                | "completely" | "totally" | "very" | "extremely" | "entirely" | "absolutely" | "deeply" | "highly"
        ) || (lower.ends_with("ed") && lower.len() > 3);
        if should_lower && ch.is_ascii_uppercase() {
            let replacement = ch.to_ascii_lowercase().to_string();
            token.text.replace_range(pos..pos + ch.len_utf8(), &replacement);
        }
        break;
    }
}

fn capitalize_first_ascii_word(segment: &mut CanonicalSegment) {
    for token in &mut segment.tokens {
        let Some(pos) = token.text.char_indices().find_map(|(idx, ch)| ch.is_ascii_alphabetic().then_some(idx)) else { continue };
        let ch = token.text[pos..].chars().next().unwrap();
        if ch.is_ascii_lowercase() {
            let upper = ch.to_ascii_uppercase().to_string();
            token.text.replace_range(pos..pos + ch.len_utf8(), &upper);
        }
        break;
    }
}

fn find_clause_marker_token_index(tokens: &[CanonicalToken]) -> Option<usize> {
    for (index, token) in tokens.iter().enumerate().skip(1) {
        let words = english_words(&token.text);
        let Some(first) = words.first().map(String::as_str) else { continue };
        if matches!(first, "as" | "when" | "while" | "because" | "although" | "though" | "but" | "and" | "so" | "yet") {
            return Some(index);
        }
    }
    None
}

fn has_main_clause_after_comma(text: &str) -> bool {
    let Some((_, after)) = text.split_once(',') else { return false };
    let words = english_words(after);
    if words.len() < 2 {
        return false;
    }
    if words.first().is_some_and(|word| is_explicit_subject(word)) {
        return true;
    }
    if matches!(words.first().map(String::as_str), Some("mr" | "mrs" | "ms" | "miss" | "dr" | "professor"))
        && words.len() >= 3
    {
        return true;
    }

    let raw_first = after.trim_start()
        .split(|c: char| !c.is_ascii_alphabetic())
        .find(|word| !word.is_empty())
        .unwrap_or_default();
    let lower_first = raw_first.to_ascii_lowercase();
    let subordinate = matches!(
        lower_first.as_str(),
        "as" | "when" | "while" | "because" | "although" | "though" | "if" | "unless" | "until" | "despite"
    );
    raw_first.chars().next().is_some_and(|ch| ch.is_ascii_uppercase()) && !subordinate
}

fn is_explicit_subject(word: &str) -> bool {
    matches!(word, "i" | "you" | "he" | "she" | "it" | "we" | "they" | "this" | "that" | "these" | "those" | "there" | "no")
}

fn ends_question_or_exclamation(text: &str) -> bool {
    text.trim_end().chars().rev().find(|c| !matches!(c, '"' | '\'' | '”' | '’' | ')' | ']'))
        .is_some_and(|c| matches!(c, '?' | '!'))
}

fn starts_with_phrase(words: &[String], phrase: &[&str]) -> bool {
    words.len() >= phrase.len()
        && words[..phrase.len()]
            .iter()
            .map(String::as_str)
            .eq(phrase.iter().copied())
}

fn ends_with_phrase(words: &[String], phrase: &[&str]) -> bool {
    words.len() >= phrase.len()
        && words[words.len() - phrase.len()..]
            .iter()
            .map(String::as_str)
            .eq(phrase.iter().copied())
}

fn english_words(text: &str) -> Vec<String> {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '\'' || c == '’'))
        .map(|word| word.trim_matches(|c: char| c == '\'' || c == '’').to_ascii_lowercase())
        .filter(|word| !word.is_empty() && word.chars().any(|c| c.is_ascii_alphabetic()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::model::{Provenance, TimeSpan};

    fn token(id: u64, text: &str, start: u64, end: u64) -> CanonicalToken {
        CanonicalToken {
            id,
            text: text.to_string(),
            start_ms: start,
            end_ms: end,
            provenance: Provenance::single(id, TimeSpan::new(start, end)),
        }
    }

    fn segment(id: &str, start: u64, words: &[&str]) -> CanonicalSegment {
        let mut tokens = Vec::new();
        for (i, word) in words.iter().enumerate() {
            tokens.push(token((start + i as u64 + 1) * 17, word, start + i as u64 * 100, start + (i as u64 + 1) * 100));
        }
        let mut seg = CanonicalSegment {
            id: id.to_string(),
            start_ms: start,
            end_ms: start + words.len() as u64 * 100,
            text: String::new(),
            tokens,
            translated_text: None,
        };
        refresh_segment_text(&mut seg, LanguageProfile::En);
        seg
    }

    #[test]
    fn relocates_linking_complement_before_subordinate_clause() {
        let mut segments = vec![
            segment("a", 0, &["She", "finally", "understood", "all", "the", "money", "had", "become", "."]),
            segment("b", 900, &["Completely", "useless", "as", "she", "wept", ",", "she", "felt", "something", "."]),
        ];
        let mut log = TransformLog::new("t");
        run_final_semantic_boundary_review(&mut segments, LanguageProfile::En, &mut log);
        assert_eq!(segments.len(), 2);
        assert!(segments[0].text.to_ascii_lowercase().contains("had become completely useless."));
        assert!(segments[1].text.to_ascii_lowercase().starts_with("as she wept"));
    }

    #[test]
    fn merges_last_time_and_coordinated_continuation_chain() {
        let mut segments = vec![
            segment("a", 0, &["She", "couldn't", "remember", "the", "last", "time", "."]),
            segment("b", 700, &["She", "had", "laughed", "with", "someone", "."]),
            segment("c", 1300, &["Or", "shared", "a", "meal", "with", "a", "friend", "."]),
        ];
        let mut log = TransformLog::new("t");
        run_final_semantic_boundary_review(&mut segments, LanguageProfile::En, &mut log);
        assert_eq!(segments.len(), 1);
        let text = segments[0].text.to_ascii_lowercase();
        assert!(text.contains("the last time she had laughed with someone or shared a meal with a friend"));
    }

    #[test]
    fn merges_nominal_subject_with_predicate_and_object_continuation() {
        let mut segments = vec![
            segment("a", 0, &["From", "that", "day", "on", ",", "hungry", "stray", "cats", "and", "dogs", "."]),
            segment("b", 1100, &["Always", "found", "a", "bowl", "of", "food", "."]),
            segment("c", 1800, &["And", "clean", "water", "waiting", "outside", "her", "shop", "."]),
        ];
        let mut log = TransformLog::new("t");
        run_final_semantic_boundary_review(&mut segments, LanguageProfile::En, &mut log);
        assert_eq!(segments.len(), 1);
        let text = segments[0].text.to_ascii_lowercase();
        assert!(text.contains("cats and dogs always found a bowl of food and clean water"));
    }

    #[test]
    fn merges_dangling_subordinate_with_following_main_clause() {
        let mut segments = vec![
            segment("a", 0, &["One", "evening", ",", "as", "the", "sun", "began", "to", "set", "."]),
            segment("b", 1000, &["Mrs.", "Stingy", "sat", "alone", "in", "her", "shop", "."]),
        ];
        let mut log = TransformLog::new("t");
        run_final_semantic_boundary_review(&mut segments, LanguageProfile::En, &mut log);
        assert_eq!(segments.len(), 1);
        let text = segments[0].text.to_ascii_lowercase();
        assert!(text.contains("as the sun began to set,") && text.contains("stingy sat alone"));
    }

    #[test]
    fn merges_adverbial_nominal_fragment_with_bare_predicate() {
        let mut segments = vec![
            segment("a", 0, &["Just", "then", ",", "dark", "clouds", "."]),
            segment("b", 600, &["Covered", "the", "sky", "."]),
        ];
        let mut log = TransformLog::new("t");
        run_final_semantic_boundary_review(&mut segments, LanguageProfile::En, &mut log);
        assert_eq!(segments.len(), 1);
        assert!(segments[0].text.to_ascii_lowercase().contains("dark clouds covered the sky"));
    }


    #[test]
    fn keeps_complete_subordinate_sentence_with_main_clause() {
        let mut segments = vec![
            segment("a", 0, &["Although", "she", "had", "money", ",", "she", "refused", "to", "spend", "it", "."]),
            segment("b", 1100, &["She", "went", "home", "."]),
        ];
        let mut log = TransformLog::new("t");
        run_final_semantic_boundary_review(&mut segments, LanguageProfile::En, &mut log);
        assert_eq!(segments.len(), 2);
    }
}

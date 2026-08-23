use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LexicalSpan {
    pub norm: String,
    pub byte_start: usize,
    pub byte_end: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DecoderSurfaceHealth {
    pub word_count: usize,
    pub ascii_letter_count: usize,
    pub uppercase_ratio: f32,
    pub strong_punctuation_count: usize,
    pub repetition_ratio: f32,
    pub case_degenerated: bool,
    pub punctuation_degenerated: bool,
    pub severe: bool,
}

pub(crate) fn lexical_spans(text: &str) -> Vec<LexicalSpan> {
    let mut spans = Vec::new();
    let mut start: Option<usize> = None;
    let mut end = 0usize;

    let flush = |spans: &mut Vec<LexicalSpan>, start: &mut Option<usize>, end: usize| {
        if let Some(s) = start.take() {
            if end > s {
                let raw = &text[s..end];
                spans.push(LexicalSpan {
                    norm: raw.to_ascii_lowercase(),
                    byte_start: s,
                    byte_end: end,
                });
            }
        }
    };

    for (idx, ch) in text.char_indices() {
        let next = idx + ch.len_utf8();
        if ch.is_ascii_alphanumeric() || matches!(ch, '\'' | '’' | '-') {
            if start.is_none() {
                start = Some(idx);
            }
            end = next;
        } else {
            flush(&mut spans, &mut start, end);
        }
    }
    flush(&mut spans, &mut start, end);
    spans
}

pub(crate) fn lexical_units(text: &str) -> Vec<String> {
    lexical_spans(text).into_iter().map(|span| span.norm).collect()
}

pub(crate) fn lexical_surface_equivalent(a: &str, b: &str) -> bool {
    let aa = lexical_units(a);
    let bb = lexical_units(b);
    !aa.is_empty() && aa == bb
}

pub(crate) fn punctuation_only_projection(source: &str, observed: &str) -> Option<String> {
    let source_spans = lexical_spans(source);
    let observed_spans = lexical_spans(observed);
    if source_spans.is_empty() || source_spans.len() != observed_spans.len() {
        return None;
    }
    if source_spans
        .iter()
        .zip(observed_spans.iter())
        .any(|(a, b)| a.norm != b.norm)
    {
        return None;
    }

    let mut out = String::with_capacity(source.len().max(observed.len()));
    let mut observed_cursor = 0usize;
    for (source_span, observed_span) in source_spans.iter().zip(observed_spans.iter()) {
        if observed_span.byte_start > observed_cursor {
            out.push_str(&observed[observed_cursor..observed_span.byte_start]);
        }
        out.push_str(&source[source_span.byte_start..source_span.byte_end]);
        observed_cursor = observed_span.byte_end;
    }
    if observed_cursor < observed.len() {
        out.push_str(&observed[observed_cursor..]);
    }
    Some(normalize_spaces(&out))
}

pub fn analyze_decoder_surface(text: &str, duration_ms: u64) -> DecoderSurfaceHealth {
    let words = lexical_units(text);
    let ascii_letters = text.chars().filter(|c| c.is_ascii_alphabetic()).collect::<Vec<_>>();
    let uppercase = ascii_letters.iter().filter(|c| c.is_ascii_uppercase()).count();
    let uppercase_ratio = if ascii_letters.is_empty() {
        0.0
    } else {
        uppercase as f32 / ascii_letters.len() as f32
    };
    let strong_punctuation_count = text
        .chars()
        .filter(|c| matches!(c, '.' | '?' | '!' | '。' | '？' | '！'))
        .count();
    let repetition_ratio = repeated_ngram_ratio(&words);

    let case_degenerated = words.len() >= 4
        && ascii_letters.len() >= 12
        && uppercase_ratio >= 0.90;
    let punctuation_degenerated = duration_ms >= 8_000
        && words.len() >= 10
        && strong_punctuation_count == 0;
    let severe = duration_ms >= 10_000
        && words.len() >= 10
        && punctuation_degenerated
        && (case_degenerated || repetition_ratio >= 0.24);

    DecoderSurfaceHealth {
        word_count: words.len(),
        ascii_letter_count: ascii_letters.len(),
        uppercase_ratio,
        strong_punctuation_count,
        repetition_ratio,
        case_degenerated,
        punctuation_degenerated,
        severe,
    }
}

fn repeated_ngram_ratio(words: &[String]) -> f32 {
    if words.len() < 6 {
        return 0.0;
    }
    let mut best = 0.0_f32;
    for n in 2..=4usize {
        if words.len() < n * 2 {
            continue;
        }
        let mut counts: HashMap<String, usize> = HashMap::new();
        for window in words.windows(n) {
            let key = window.join("\u{1f}");
            *counts.entry(key).or_default() += 1;
        }
        for count in counts.values().copied().filter(|count| *count >= 2) {
            let covered = (count * n).min(words.len());
            best = best.max(covered as f32 / words.len() as f32);
        }
    }
    best
}

fn normalize_spaces(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut previous_space = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            if !previous_space && !out.is_empty() {
                out.push(' ');
            }
            previous_space = true;
        } else {
            out.push(ch);
            previous_space = false;
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_long_all_caps_punctuation_collapse() {
        let health = analyze_decoder_surface(
            "HOW'S THE GAME PRETTY GOOD RIGHT I DON'T THINK IT'S FUN DO YOU THINK SO I'M NOT INTO IT EITHER I THINK WE SHOULD PLAY BRIDGE I THINK WE SHOULD PLAY BRIDGE TOO",
            20_800,
        );
        assert!(health.case_degenerated);
        assert!(health.punctuation_degenerated);
        assert!(health.severe);
    }

    #[test]
    fn short_shout_is_not_severe_decoder_degeneration() {
        let health = analyze_decoder_surface("GET OVER HERE, YOUNG LADY!", 1_950);
        assert!(health.case_degenerated);
        assert!(!health.punctuation_degenerated);
        assert!(!health.severe);
    }

    #[test]
    fn punctuation_projection_never_changes_lexical_surface() {
        let source = "HOW'S THE GAME PRETTY GOOD RIGHT I DON'T THINK IT'S FUN";
        let observed = "How's the game? Pretty good, right. I don't think it's fun.";
        let projected = punctuation_only_projection(source, observed).unwrap();
        assert_eq!(projected, "HOW'S THE GAME? PRETTY GOOD, RIGHT. I DON'T THINK IT'S FUN.");
    }
}

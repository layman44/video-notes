use std::hash::{Hash, Hasher};
use std::ops::Range;

use crate::transcript::model::{CanonicalSegment, CanonicalToken, LanguageProfile, Provenance, TimeSpan, TokenId};
use crate::transcript::transform::{TransformLog, TransformOperation, TransformStage};

#[derive(Debug, Clone)]
pub struct TextReplacement {
    pub range: Range<usize>,
    pub replacement: String,
    pub operation: TransformOperation,
    pub rule_id: &'static str,
    pub confidence: f32,
}

pub fn synthetic_token_id(segment_id: &str, salt: impl Hash) -> TokenId {
    let mut h = Fnv1a64::default();
    segment_id.hash(&mut h);
    salt.hash(&mut h);
    (1u64 << 63) | (h.finish() & !(1u64 << 63))
}

#[derive(Default)]
struct Fnv1a64(u64);

impl Hasher for Fnv1a64 {
    fn finish(&self) -> u64 {
        if self.0 == 0 { 0xcbf29ce484222325 } else { self.0 }
    }

    fn write(&mut self, bytes: &[u8]) {
        let mut hash = if self.0 == 0 { 0xcbf29ce484222325 } else { self.0 };
        for byte in bytes {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        self.0 = hash;
    }
}

pub fn canonical_token_from_raw(
    segment_id: &str,
    raw_id: TokenId,
    text: String,
    start_ms: u64,
    end_ms: u64,
) -> CanonicalToken {
    CanonicalToken {
        id: synthetic_token_id(segment_id, (raw_id, start_ms, end_ms, &text)),
        text,
        start_ms,
        end_ms,
        provenance: Provenance::single(raw_id, TimeSpan::new(start_ms, end_ms)),
    }
}

pub fn synthetic_segment_token(segment_id: &str, text: String, start_ms: u64, end_ms: u64) -> CanonicalToken {
    CanonicalToken {
        id: synthetic_token_id(segment_id, (start_ms, end_ms, &text)),
        text,
        start_ms,
        end_ms,
        provenance: Provenance::multiple(Vec::new(), TimeSpan::new(start_ms, end_ms)),
    }
}

pub fn render_tokens(tokens: &[CanonicalToken], profile: LanguageProfile) -> String {
    let mut out = String::new();
    for token in tokens {
        let piece = token.text.trim();
        if piece.is_empty() { continue; }
        if out.is_empty() {
            out.push_str(piece);
            continue;
        }
        let prev = out.chars().last().unwrap_or(' ');
        let curr = piece.chars().next().unwrap_or(' ');
        if should_insert_space(prev, curr, profile) {
            out.push(' ');
        }
        out.push_str(piece);
    }
    out.trim().to_string()
}

fn should_insert_space(prev: char, curr: char, profile: LanguageProfile) -> bool {
    if is_closing_punctuation(curr) || is_opening_punctuation(prev) { return false; }
    if prev.is_whitespace() || curr.is_whitespace() { return false; }

    let prev_cjk = is_cjk(prev);
    let curr_cjk = is_cjk(curr);
    let prev_ascii_word = prev.is_ascii_alphanumeric() || matches!(prev, '_' | '#' | '+');
    let curr_ascii_word = curr.is_ascii_alphanumeric() || matches!(curr, '_' | '#' | '+');

    if prev_cjk && curr_cjk { return false; }
    if (prev_cjk && curr_ascii_word) || (prev_ascii_word && curr_cjk) { return true; }
    if prev_ascii_word && curr_ascii_word { return true; }

    matches!(profile, LanguageProfile::En | LanguageProfile::Mixed | LanguageProfile::Auto)
        && !is_punctuation(prev)
        && !is_punctuation(curr)
}

pub fn refresh_segment_text(segment: &mut CanonicalSegment, profile: LanguageProfile) {
    segment.refresh_bounds_from_tokens();
    segment.text = render_tokens(&segment.tokens, profile);
}

pub fn apply_text_replacements(
    segment: &mut CanonicalSegment,
    profile: LanguageProfile,
    stage: TransformStage,
    replacements: &[TextReplacement],
    log: &mut TransformLog,
) {
    if replacements.is_empty() { return; }

    let mut reps = replacements.to_vec();
    reps.sort_by(|a, b| b.range.start.cmp(&a.range.start));

    for rep in reps {
        if rep.range.start >= rep.range.end || rep.range.end > segment.text.len() { continue; }
        if !segment.text.is_char_boundary(rep.range.start) || !segment.text.is_char_boundary(rep.range.end) { continue; }

        let before = segment.text[rep.range.clone()].to_string();
        if before == rep.replacement { continue; }

        let precise = segment.tokens.iter().any(|t| !t.provenance.source_token_ids.is_empty());
        if !precise {
            segment.text.replace_range(rep.range.clone(), &rep.replacement);
            if segment.tokens.is_empty() {
                segment.tokens.push(synthetic_segment_token(
                    &segment.id,
                    segment.text.clone(),
                    segment.start_ms,
                    segment.end_ms,
                ));
            } else {
                let span = segment.span();
                let provenance = segment.tokens.iter().fold(None, |acc: Option<Provenance>, t| {
                    Some(match acc { Some(p) => p.merge(&t.provenance), None => t.provenance.clone() })
                }).unwrap_or_else(|| Provenance::multiple(Vec::new(), span));
                segment.tokens = vec![CanonicalToken {
                    id: synthetic_token_id(&segment.id, (&segment.text, segment.start_ms, segment.end_ms)),
                    text: segment.text.clone(),
                    start_ms: segment.start_ms,
                    end_ms: segment.end_ms,
                    provenance,
                }];
            }
            log.record(stage, rep.operation, Vec::new(), before, rep.replacement, rep.rule_id, rep.confidence);
            continue;
        }

        let (rendered, spans) = render_tokens_with_spans(&segment.tokens, profile);
        if rendered != segment.text {
            // Restore the invariant before editing rather than applying offsets to a stale projection.
            segment.text = rendered.clone();
            if rep.range.end > rendered.len() { continue; }
        }

        let affected: Vec<usize> = spans
            .iter()
            .enumerate()
            .filter_map(|(i, r)| (r.end > rep.range.start && r.start < rep.range.end).then_some(i))
            .collect();
        if affected.is_empty() { continue; }

        let first = *affected.first().unwrap();
        let last = *affected.last().unwrap();
        let source_ids = merged_source_ids(&segment.tokens[first..=last]);
        let merged_provenance = segment.tokens[first..=last]
            .iter()
            .map(|t| t.provenance.clone())
            .reduce(|a, b| a.merge(&b))
            .unwrap();

        // A match contained inside one token can be edited in-place without destroying surrounding text.
        if first == last && rep.range.start >= spans[first].start && rep.range.end <= spans[first].end {
            let local_start = rep.range.start - spans[first].start;
            let local_end = rep.range.end - spans[first].start;
            if segment.tokens[first].text.is_char_boundary(local_start) && segment.tokens[first].text.is_char_boundary(local_end) {
                segment.tokens[first].text.replace_range(local_start..local_end, &rep.replacement);
                refresh_segment_text(segment, profile);
                log.record(stage, rep.operation, source_ids, before, rep.replacement, rep.rule_id, rep.confidence);
                continue;
            }
        }

        let first_span = &spans[first];
        let last_span = &spans[last];
        let prefix_len = rep.range.start.saturating_sub(first_span.start).min(segment.tokens[first].text.len());
        let suffix_start = rep.range.end.saturating_sub(last_span.start).min(segment.tokens[last].text.len());

        let prefix = if prefix_len > 0 && segment.tokens[first].text.is_char_boundary(prefix_len) {
            Some(segment.tokens[first].text[..prefix_len].to_string())
        } else { None };
        let suffix = if suffix_start < segment.tokens[last].text.len() && segment.tokens[last].text.is_char_boundary(suffix_start) {
            Some(segment.tokens[last].text[suffix_start..].to_string())
        } else { None };

        let start_ms = segment.tokens[first].start_ms;
        let end_ms = segment.tokens[last].end_ms;
        let mut replacement_tokens = Vec::new();
        if let Some(prefix_text) = prefix.filter(|s| !s.is_empty()) {
            let mut t = segment.tokens[first].clone();
            t.text = prefix_text;
            replacement_tokens.push(t);
        }
        if !rep.replacement.is_empty() {
            replacement_tokens.push(CanonicalToken {
                id: synthetic_token_id(&segment.id, (rep.range.start, rep.range.end, &rep.replacement, rep.rule_id)),
                text: rep.replacement.clone(),
                start_ms,
                end_ms,
                provenance: merged_provenance,
            });
        }
        if let Some(suffix_text) = suffix.filter(|s| !s.is_empty()) {
            let mut t = segment.tokens[last].clone();
            t.id = synthetic_token_id(&segment.id, (last, &suffix_text, rep.rule_id));
            t.text = suffix_text;
            replacement_tokens.push(t);
        }

        segment.tokens.splice(first..=last, replacement_tokens);
        refresh_segment_text(segment, profile);
        log.record(stage, rep.operation, source_ids, before, rep.replacement, rep.rule_id, rep.confidence);
    }
}

pub fn render_tokens_with_spans(tokens: &[CanonicalToken], profile: LanguageProfile) -> (String, Vec<Range<usize>>) {
    let mut out = String::new();
    let mut spans = Vec::with_capacity(tokens.len());
    for token in tokens {
        let piece = token.text.trim();
        if piece.is_empty() {
            spans.push(out.len()..out.len());
            continue;
        }
        if !out.is_empty() {
            let prev = out.chars().last().unwrap_or(' ');
            let curr = piece.chars().next().unwrap_or(' ');
            if should_insert_space(prev, curr, profile) { out.push(' '); }
        }
        let start = out.len();
        out.push_str(piece);
        spans.push(start..out.len());
    }
    (out.trim().to_string(), spans)
}

pub fn merged_source_ids(tokens: &[CanonicalToken]) -> Vec<TokenId> {
    let mut ids: Vec<_> = tokens.iter().flat_map(|t| t.provenance.source_token_ids.iter().copied()).collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

pub fn is_cjk(ch: char) -> bool {
    matches!(ch as u32,
        0x3400..=0x4DBF | 0x4E00..=0x9FFF |
        0x3040..=0x30FF | 0x31F0..=0x31FF |
        0x1100..=0x11FF | 0xAC00..=0xD7AF)
}

pub fn is_punctuation(ch: char) -> bool {
    ch.is_ascii_punctuation() || matches!(ch,
        '，' | '。' | '！' | '？' | '：' | '；' | '、' | '“' | '”' | '‘' | '’' |
        '（' | '）' | '《' | '》' | '【' | '】' | '—' | '…')
}

fn is_closing_punctuation(ch: char) -> bool {
    is_punctuation(ch) && !matches!(ch, '(' | '[' | '{' | '“' | '‘' | '（' | '【' | '《')
}

fn is_opening_punctuation(ch: char) -> bool {
    matches!(ch, '(' | '[' | '{' | '“' | '‘' | '（' | '【' | '《')
}

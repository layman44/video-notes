use std::ops::Range;

use crate::transcript::model::{CanonicalSegment, CanonicalToken, LanguageProfile, Provenance, RawSegment, RawToken, TimeSpan};
use crate::transcript::pipeline::edit::{canonical_token_from_raw, refresh_segment_text, synthetic_segment_token, synthetic_token_id};
use crate::transcript::pipeline::{BoundaryEvidence, BoundaryEvidenceKind};
use crate::transcript::transform::{TransformLog, TransformOperation, TransformStage};

pub fn resolve_boundaries(
    segments: Vec<RawSegment>,
    profile: LanguageProfile,
    evidence: &[BoundaryEvidence],
    log: &mut TransformLog,
) -> Vec<CanonicalSegment> {
    let mut ordered = segments;
    ordered.sort_by_key(|seg| (seg.start_ms, seg.end_ms));

    let mut split = Vec::new();
    for seg in ordered {
        let local = evidence
            .iter()
            .filter(|e| e.segment_id == seg.id)
            .cloned()
            .collect::<Vec<_>>();
        split.extend(split_raw_segment(seg, profile, &local, log));
    }
    merge_adjacent_segments(split, profile, evidence, log)
}

/// Canonical sentence splitting is text-led. Only StrongPunctuation or AlignmentBoundary
/// evidence can split a sentence. AcousticPause is corroborating evidence and is intentionally
/// ignored here; SubtitleRenderer may use it later without changing linguistic sentence structure.
fn split_raw_segment(
    seg: RawSegment,
    profile: LanguageProfile,
    evidence: &[BoundaryEvidence],
    log: &mut TransformLog,
) -> Vec<CanonicalSegment> {
    let raw_text = normalize_surface(&seg.text);
    if raw_text.is_empty() {
        return Vec::new();
    }
    let char_len = raw_text.chars().count();

    let mut cuts = evidence
        .iter()
        .filter(|e| {
            matches!(e.kind, BoundaryEvidenceKind::StrongPunctuation | BoundaryEvidenceKind::AlignmentBoundary)
                && e.confidence >= 0.50
                && e.char_offset > 0
                && e.char_offset < char_len
                && e.time_ms > seg.start_ms
                && e.time_ms < seg.end_ms
        })
        .cloned()
        .collect::<Vec<_>>();
    cuts.sort_by_key(|e| (e.char_offset, e.time_ms));
    cuts.dedup_by(|a, b| a.char_offset.abs_diff(b.char_offset) <= 1 || a.time_ms.abs_diff(b.time_ms) <= 80);

    if cuts.is_empty() {
        return vec![canonical_from_whole_segment(seg, profile, &raw_text)];
    }

    let mut offsets = Vec::with_capacity(cuts.len() + 2);
    offsets.push(0usize);
    offsets.extend(cuts.iter().map(|e| e.char_offset));
    offsets.push(char_len);

    let mut times = Vec::with_capacity(cuts.len() + 2);
    times.push(seg.start_ms);
    times.extend(cuts.iter().map(|e| e.time_ms));
    times.push(seg.end_ms);

    let raw_tokens = {
        let mut t = seg.tokens.clone();
        t.sort_by_key(|v| (v.start_ms, v.end_ms));
        t
    };
    let token_spans = if raw_tokens.is_empty() {
        None
    } else {
        align_raw_token_spans(&raw_text, &raw_tokens)
    };

    let mut result = Vec::new();
    for i in 0..offsets.len() - 1 {
        let a = offsets[i];
        let b = offsets[i + 1];
        if a >= b { continue; }
        let Some(byte_a) = char_to_byte_offset(&raw_text, a) else { continue };
        let Some(byte_b) = char_to_byte_offset(&raw_text, b) else { continue };
        if byte_a >= byte_b { continue; }
        let part_text = raw_text[byte_a..byte_b].trim();
        if part_text.is_empty() { continue; }

        let nominal_start = times[i];
        let nominal_end = times[i + 1].max(nominal_start);
        let id = format!("{}#s{}", seg.id, i + 1);

        let token_slice = token_spans.as_ref().map(|spans| {
            raw_tokens
                .iter()
                .zip(spans.iter())
                .filter(|(_, span)| span.end > byte_a && span.start < byte_b)
                .map(|(token, _)| token.clone())
                .collect::<Vec<_>>()
        }).unwrap_or_default();

        let tokens = if token_slice.is_empty() {
            vec![synthetic_segment_token(&id, part_text.to_string(), nominal_start, nominal_end)]
        } else {
            canonical_tokens_preserving_text(&id, part_text, &token_slice, nominal_start, nominal_end)
        };
        let mut canonical = CanonicalSegment {
            id,
            start_ms: nominal_start,
            end_ms: nominal_end,
            text: String::new(),
            tokens,
            translated_text: None,
        };
        refresh_segment_text(&mut canonical, profile);
        // Synthetic-only segments use evidence time. Token-aligned segments use their real token bounds.
        if canonical.source_token_ids().is_empty() {
            canonical.start_ms = nominal_start;
            canonical.end_ms = nominal_end;
        }
        result.push(canonical);
    }

    if result.len() < 2 {
        return vec![canonical_from_whole_segment(seg, profile, &raw_text)];
    }

    let source_ids = raw_tokens.iter().map(|t| t.id).collect::<Vec<_>>();
    let after = result.iter().map(|s| s.text.as_str()).collect::<Vec<_>>().join(" | ");
    let min_confidence = cuts.iter().map(|e| e.confidence).fold(1.0_f32, f32::min);
    let has_alignment = cuts.iter().any(|e| e.kind == BoundaryEvidenceKind::AlignmentBoundary);
    log.record(
        TransformStage::Boundary,
        TransformOperation::SplitBoundary,
        source_ids,
        raw_text,
        after,
        if has_alignment { "sentence_split_alignment_timeline" } else { "sentence_split_strong_punctuation_estimated_time" },
        min_confidence,
    );
    result
}

fn canonical_from_whole_segment(seg: RawSegment, profile: LanguageProfile, raw_text: &str) -> CanonicalSegment {
    let mut raw_tokens = seg.tokens.clone();
    raw_tokens.sort_by_key(|t| (t.start_ms, t.end_ms));
    let tokens = if raw_tokens.is_empty() {
        vec![synthetic_segment_token(&seg.id, raw_text.to_string(), seg.start_ms, seg.end_ms)]
    } else {
        canonical_tokens_preserving_text(&seg.id, raw_text, &raw_tokens, seg.start_ms, seg.end_ms)
    };
    let mut canonical = CanonicalSegment {
        id: seg.id,
        start_ms: seg.start_ms,
        end_ms: seg.end_ms,
        text: String::new(),
        tokens,
        translated_text: None,
    };
    refresh_segment_text(&mut canonical, profile);
    if canonical.source_token_ids().is_empty() {
        canonical.start_ms = seg.start_ms;
        canonical.end_ms = seg.end_ms;
    }
    canonical
}

fn merge_adjacent_segments(
    segments: Vec<CanonicalSegment>,
    profile: LanguageProfile,
    evidence: &[BoundaryEvidence],
    log: &mut TransformLog,
) -> Vec<CanonicalSegment> {
    let mut out: Vec<CanonicalSegment> = Vec::with_capacity(segments.len());
    for seg in segments {
        if seg.text.trim().is_empty() { continue; }
        if let Some(prev) = out.last_mut() {
            let gap = seg.start_ms.saturating_sub(prev.end_ms);
            let no_strong_terminal = !ends_strong(prev.text.trim());
            let combined_len = prev.text.chars().count() + seg.text.chars().count();
            let max_len = if profile.prefers_cjk_spacing() { 90 } else { 180 };
            let threshold = match profile {
                LanguageProfile::En => 420,
                LanguageProfile::Zh | LanguageProfile::Ja | LanguageProfile::Ko => 520,
                _ => 450,
            };

            let hard_sentence_boundary = evidence.iter().any(|e| {
                matches!(e.kind, BoundaryEvidenceKind::StrongPunctuation | BoundaryEvidenceKind::AlignmentBoundary)
                    && e.confidence >= 0.50
                    && (e.time_ms.abs_diff(prev.end_ms) <= 120 || e.time_ms.abs_diff(seg.start_ms) <= 120)
            });

            // Overlap stays separate for Dedupe. Acoustic pauses alone do NOT lock a Canonical
            // sentence boundary; this is what prevents "Please | have some" and comma pauses
            // such as "worn out, | torn" from becoming false sentences.
            if !hard_sentence_boundary
                && seg.start_ms >= prev.end_ms
                && gap <= threshold
                && no_strong_terminal
                && combined_len <= max_len
            {
                let before = format!("{} | {}", prev.text, seg.text);
                let mut source_ids = prev.source_token_ids();
                source_ids.extend(seg.source_token_ids());
                source_ids.sort_unstable();
                source_ids.dedup();
                prev.tokens.extend(seg.tokens);
                refresh_segment_text(prev, profile);
                if prev.source_token_ids().is_empty() {
                    prev.end_ms = seg.end_ms.max(prev.end_ms);
                }
                log.record(
                    TransformStage::Boundary,
                    TransformOperation::MergeBoundary,
                    source_ids,
                    before,
                    prev.text.clone(),
                    "short_gap_without_sentence_terminal",
                    0.88,
                );
                continue;
            }
        }
        out.push(seg);
    }
    out
}

fn normalize_surface(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn char_to_byte_offset(text: &str, char_offset: usize) -> Option<usize> {
    if char_offset == text.chars().count() {
        return Some(text.len());
    }
    text.char_indices().nth(char_offset).map(|(idx, _)| idx)
}

/// Reconciles non-acoustic text (typically punctuation) from the ASR segment with aligned raw tokens.
/// When exact alignment fails, it safely falls back to one canonical token carrying all source ids.
fn canonical_tokens_preserving_text(
    segment_id: &str,
    raw_text: &str,
    raw_tokens: &[RawToken],
    segment_start_ms: u64,
    segment_end_ms: u64,
) -> Vec<CanonicalToken> {
    let Some(spans) = align_raw_token_spans(raw_text, raw_tokens) else {
        let mut ids: Vec<_> = raw_tokens.iter().map(|t| t.id).collect();
        ids.sort_unstable();
        ids.dedup();
        return vec![CanonicalToken {
            id: synthetic_token_id(segment_id, (segment_start_ms, segment_end_ms, raw_text, "unaligned")),
            text: raw_text.trim().to_string(),
            start_ms: segment_start_ms,
            end_ms: segment_end_ms,
            provenance: Provenance::multiple(ids, TimeSpan::new(segment_start_ms, segment_end_ms)),
        }];
    };

    let mut out = Vec::new();
    let mut cursor = 0usize;
    for (i, token) in raw_tokens.iter().enumerate() {
        let span = &spans[i];
        if span.start > cursor {
            push_synthetic_gap(
                &mut out,
                segment_id,
                &raw_text[cursor..span.start],
                if i == 0 { segment_start_ms } else { raw_tokens[i - 1].end_ms },
                token.start_ms,
                cursor,
            );
        }
        out.push(canonical_token_from_raw(segment_id, token.id, token.text.trim().to_string(), token.start_ms, token.end_ms));
        cursor = span.end;
    }
    if cursor < raw_text.len() {
        push_synthetic_gap(
            &mut out,
            segment_id,
            &raw_text[cursor..],
            raw_tokens.last().map(|t| t.end_ms).unwrap_or(segment_start_ms),
            segment_end_ms,
            cursor,
        );
    }
    out
}

fn push_synthetic_gap(
    out: &mut Vec<CanonicalToken>,
    segment_id: &str,
    gap: &str,
    start_ms: u64,
    end_ms: u64,
    salt: usize,
) {
    let text = gap.trim();
    if text.is_empty() { return; }
    out.push(CanonicalToken {
        id: synthetic_token_id(segment_id, (salt, text, "text-gap")),
        text: text.to_string(),
        start_ms,
        end_ms: end_ms.max(start_ms),
        provenance: Provenance::multiple(Vec::new(), TimeSpan::new(start_ms, end_ms)),
    });
}

fn align_raw_token_spans(text: &str, tokens: &[RawToken]) -> Option<Vec<Range<usize>>> {
    let mut spans = Vec::with_capacity(tokens.len());
    let mut cursor = 0usize;
    for token in tokens {
        let needle = token.text.trim();
        if needle.is_empty() {
            spans.push(cursor..cursor);
            continue;
        }
        let rest = text.get(cursor..)?;
        let rel = rest.find(needle).or_else(|| find_ascii_case_insensitive(rest, needle))?;
        let start = cursor + rel;
        let end = start + needle.len();
        spans.push(start..end);
        cursor = end;
    }
    Some(spans)
}

fn find_ascii_case_insensitive(hay: &str, needle: &str) -> Option<usize> {
    if !hay.is_ascii() || !needle.is_ascii() {
        return None;
    }
    hay.to_ascii_lowercase().find(&needle.to_ascii_lowercase())
}

fn ends_strong(text: &str) -> bool {
    text.trim_end().chars().rev().find(|c| !matches!(c, '"' | '\'' | '”' | '’' | ')' | ']' | '】' | '》'))
        .is_some_and(|c| matches!(c, '.' | '?' | '!' | '。' | '！' | '？'))
}

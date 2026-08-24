use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

use crate::transcript::surface::analyze_decoder_surface;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SuspicionReason {
    MicroSegment,
    IsolatedMicroSegment,
    BoundaryFragment,
    EntityVariant,
    DecoderSurfaceDegeneration,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationSegment {
    pub id: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StableEntity {
    pub canonical: String,
    pub normalized: String,
    pub occurrences: usize,
    #[serde(default)]
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityMemory {
    pub stable: Vec<StableEntity>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SuspicionCandidate {
    /// Raw cue(s) that triggered suspicion. This span is never enlarged just to provide context.
    pub suspicious_indices: Vec<usize>,
    pub suspicious_segment_ids: Vec<String>,
    pub suspicious_start_ms: u64,
    pub suspicious_end_ms: u64,
    /// Minimal contiguous Raw cue span that verification is allowed to replace. For a tightly
    /// attached boundary fragment this may include the previous short cue; otherwise it is the
    /// suspicious cue itself. This is deliberately separate from the Expanded context window.
    pub target_indices: Vec<usize>,
    pub target_segment_ids: Vec<String>,
    pub start_ms: u64,
    pub end_ms: u64,
    pub score: f32,
    pub reasons: Vec<SuspicionReason>,
    /// Automatically learned document entities relevant to this candidate. These are model bias
    /// hints only; a verifier is never allowed to hard-replace text from this list.
    pub hotwords: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum VerificationDecision {
    Verified,
    Corrected,
    Uncertain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CorrectionKind {
    BoundaryReconstruction,
    FragmentRemoval,
    LexicalReplacement,
    EntityReplacement,
    LargeRewrite,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationResult {
    pub target_segment_ids: Vec<String>,
    pub suspicious_segment_ids: Vec<String>,
    pub suspicious_start_ms: u64,
    pub suspicious_end_ms: u64,
    pub start_ms: u64,
    pub end_ms: u64,
    pub context_start_ms: u64,
    pub context_end_ms: u64,
    pub reasons: Vec<SuspicionReason>,
    pub suspicion_score: f32,
    pub first_pass_text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expanded_nano_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expanded_target_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correction_kind: Option<CorrectionKind>,
    pub left_context_similarity: f32,
    pub right_context_similarity: f32,
    pub target_time_coverage: f32,
    /// True only when Expanded SRT provides cue-level material on both sides of RewriteSpan.
    /// A single cue covering the whole context window is explicitly not precise time grounding.
    #[serde(default)]
    pub time_grounded: bool,
    /// True when constrained left/target/right lexical alignment localized the Expanded rewrite.
    #[serde(default)]
    pub text_aligned: bool,
    pub edit_ratio: f32,
    pub replacement_ratio: f32,
    pub decision: VerificationDecision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replacement_text: Option<String>,
    pub confidence: f32,
    #[serde(default)]
    pub safety_reasons: Vec<String>,
    pub hotwords: Vec<String>,
    pub context: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExpandedSafetyAssessment {
    pub correction_kind: CorrectionKind,
    pub decision: VerificationDecision,
    pub replacement_text: Option<String>,
    pub confidence: f32,
    pub edit_ratio: f32,
    pub replacement_ratio: f32,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone)]
struct TokenSpan {
    norm: String,
    byte_start: usize,
    byte_end: usize,
}

fn is_cjk_like(ch: char) -> bool {
    matches!(
        ch as u32,
        0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0x3040..=0x30FF
            | 0x31F0..=0x31FF
            | 0xAC00..=0xD7AF
            | 0x1100..=0x11FF
    )
}

fn tokenize(text: &str) -> Vec<TokenSpan> {
    let mut out = Vec::new();
    let mut ascii_start: Option<usize> = None;
    let mut ascii_end = 0usize;

    let flush_ascii = |out: &mut Vec<TokenSpan>, start: &mut Option<usize>, end: usize| {
        if let Some(s) = start.take() {
            if end > s {
                let raw = &text[s..end];
                out.push(TokenSpan {
                    norm: raw.to_ascii_lowercase(),
                    byte_start: s,
                    byte_end: end,
                });
            }
        }
    };

    for (idx, ch) in text.char_indices() {
        let next = idx + ch.len_utf8();
        if ch.is_ascii_alphanumeric() || ch == '\'' || ch == '-' {
            if ascii_start.is_none() {
                ascii_start = Some(idx);
            }
            ascii_end = next;
            continue;
        }
        flush_ascii(&mut out, &mut ascii_start, ascii_end);
        if is_cjk_like(ch) {
            out.push(TokenSpan {
                norm: ch.to_string(),
                byte_start: idx,
                byte_end: next,
            });
        } else if ch.is_alphanumeric() {
            out.push(TokenSpan {
                norm: ch.to_lowercase().collect(),
                byte_start: idx,
                byte_end: next,
            });
        }
    }
    flush_ascii(&mut out, &mut ascii_start, ascii_end);
    out
}

fn normalize_consensus_token(token: &str) -> String {
    let lower = token.to_ascii_lowercase();
    match lower.as_str() {
        // These are orthographic variants of the same conventional English honorific.
        // This normalization is used only for verifier consensus; it does not rewrite Raw.
        "missus" | "mrs" => "mrs".to_string(),
        "mister" | "mr" => "mr".to_string(),
        "doctor" | "dr" => "dr".to_string(),
        _ => lower,
    }
}

fn normalized_surface(text: &str) -> String {
    tokenize(text)
        .into_iter()
        .map(|t| normalize_consensus_token(&t.norm))
        .collect::<Vec<_>>()
        .join("|")
}

pub fn surfaces_equivalent(a: &str, b: &str) -> bool {
    normalized_surface(a) == normalized_surface(b)
}


/// Preserve the First-pass surface for lexical tokens that Expanded Nano agrees on.
/// This prevents a useful lexical correction (for example deleting a hallucinated short cue)
/// from also importing Expanded-only casing such as `IT'S MY FAULT` into Canonical.
/// Inserted/replaced lexical tokens still use the Expanded surface.
pub fn preserve_first_surface_for_matching_tokens(first: &str, expanded: &str) -> String {
    let first_tokens = tokenize(first);
    let expanded_tokens = tokenize(expanded);
    if first_tokens.is_empty() || expanded_tokens.is_empty() {
        return expanded.to_string();
    }

    let a = first_tokens
        .iter()
        .map(|t| normalize_consensus_token(&t.norm))
        .collect::<Vec<_>>();
    let b = expanded_tokens
        .iter()
        .map(|t| normalize_consensus_token(&t.norm))
        .collect::<Vec<_>>();

    // LCS gives us only lexical agreements. Surface replacement is deliberately limited
    // to those agreements; punctuation remains an orthogonal Typography concern.
    let mut dp = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for i in 0..a.len() {
        for j in 0..b.len() {
            dp[i + 1][j + 1] = if a[i] == b[j] {
                dp[i][j] + 1
            } else {
                dp[i][j + 1].max(dp[i + 1][j])
            };
        }
    }

    let mut pairs = Vec::<(usize, usize)>::new();
    let (mut i, mut j) = (a.len(), b.len());
    while i > 0 && j > 0 {
        if a[i - 1] == b[j - 1] {
            pairs.push((i - 1, j - 1));
            i -= 1;
            j -= 1;
        } else if dp[i - 1][j] >= dp[i][j - 1] {
            i -= 1;
        } else {
            j -= 1;
        }
    }
    pairs.reverse();

    let mut out = expanded.to_string();
    for (first_index, expanded_index) in pairs.into_iter().rev() {
        let first_token = &first_tokens[first_index];
        let expanded_token = &expanded_tokens[expanded_index];
        if expanded_token.byte_end > out.len()
            || !out.is_char_boundary(expanded_token.byte_start)
            || !out.is_char_boundary(expanded_token.byte_end)
        {
            continue;
        }
        let first_surface = &first[first_token.byte_start..first_token.byte_end];
        let expanded_surface = &expanded[expanded_token.byte_start..expanded_token.byte_end];
        if first_surface != expanded_surface {
            out.replace_range(expanded_token.byte_start..expanded_token.byte_end, first_surface);
        }
    }
    out
}

fn normalized_units(text: &str) -> Vec<String> {
    tokenize(text)
        .into_iter()
        .map(|t| normalize_consensus_token(&t.norm))
        .collect()
}

pub fn context_tail(text: &str, max_units: usize) -> String {
    let tokens = tokenize(text);
    if tokens.is_empty() || tokens.len() <= max_units.max(1) {
        return text.trim().to_string();
    }
    let start = tokens[tokens.len() - max_units.max(1)].byte_start.min(text.len());
    text[start..].trim().to_string()
}

pub fn context_head(text: &str, max_units: usize) -> String {
    let tokens = tokenize(text);
    if tokens.is_empty() || tokens.len() <= max_units.max(1) {
        return text.trim().to_string();
    }
    let end = tokens[max_units.max(1) - 1].byte_end.min(text.len());
    text[..end].trim().to_string()
}

/// How much of a reference context survives in an observed Expanded transcription. Extra words in
/// the observed side do not hurt the score; the gate only asks whether the original context is
/// still recoverable. This is language-agnostic over the tokenizer above.
pub fn context_preservation(reference: &str, observed: &str) -> f32 {
    let a = normalized_units(reference);
    let b = normalized_units(observed);
    if a.is_empty() {
        return 1.0;
    }
    if b.is_empty() {
        return 0.0;
    }
    let mut prev = vec![0usize; b.len() + 1];
    for av in &a {
        let mut row = vec![0usize; b.len() + 1];
        for (j, bv) in b.iter().enumerate() {
            row[j + 1] = if av == bv {
                prev[j] + 1
            } else {
                row[j].max(prev[j + 1])
            };
        }
        prev = row;
    }
    (prev[b.len()] as f32 / a.len() as f32).clamp(0.0, 1.0)
}

fn token_edit_distance(a: &[String], b: &[String]) -> usize {
    let mut prev = (0..=b.len()).collect::<Vec<_>>();
    for (i, av) in a.iter().enumerate() {
        let mut row = vec![i + 1; b.len() + 1];
        for (j, bv) in b.iter().enumerate() {
            row[j + 1] = (prev[j + 1] + 1)
                .min(row[j] + 1)
                .min(prev[j] + usize::from(av != bv));
        }
        prev = row;
    }
    prev[b.len()]
}

pub fn token_edit_ratio(a: &str, b: &str) -> f32 {
    let aa = normalized_units(a);
    let bb = normalized_units(b);
    let denom = aa.len().max(bb.len()).max(1);
    (token_edit_distance(&aa, &bb) as f32 / denom as f32).clamp(0.0, 1.0)
}

#[derive(Debug, Clone, PartialEq)]
pub struct LocalRewriteExtraction {
    pub replacement_text: String,
    pub left_context_similarity: f32,
    pub right_context_similarity: f32,
}

fn strip_leading_boundary_marks(value: &str) -> &str {
    value.trim_start_matches(|c: char| {
        c.is_whitespace()
            || matches!(c, '.' | ',' | '?' | '!' | ':' | ';' | '，' | '。' | '？' | '！' | '：' | '；' | '、')
    })
}

/// Align `left + target + right` against the full Expanded surface and extract only the observed
/// middle between the aligned left/right contexts. Unlike the old exact-anchor gate, substitutions
/// inside the outer context are tolerated (for example Stingy/Stinge). The Safety Gate still
/// decides whether the resulting local rewrite is trustworthy.
pub fn extract_local_rewrite_by_alignment(
    left: &str,
    target: &str,
    right: &str,
    expanded: &str,
) -> Option<LocalRewriteExtraction> {
    let left_tokens = tokenize(left);
    let target_tokens = tokenize(target);
    let right_tokens = tokenize(right);
    let observed = tokenize(expanded);
    if left_tokens.is_empty() || right_tokens.is_empty() || observed.is_empty() {
        return None;
    }

    let left_len = left_tokens.len();
    let target_len = target_tokens.len();
    let mut reference = left_tokens.iter().map(|t| normalize_consensus_token(&t.norm)).collect::<Vec<_>>();
    reference.extend(target_tokens.iter().map(|t| normalize_consensus_token(&t.norm)));
    reference.extend(right_tokens.iter().map(|t| normalize_consensus_token(&t.norm)));
    let obs_norm = observed.iter().map(|t| normalize_consensus_token(&t.norm)).collect::<Vec<_>>();
    let m = reference.len();
    let n = obs_norm.len();
    if m == 0 || n == 0 || m.saturating_mul(n) > 40_000 {
        return None;
    }

    let mut dp = vec![vec![0usize; n + 1]; m + 1];
    for i in 0..=m { dp[i][0] = i; }
    for j in 0..=n { dp[0][j] = j; }
    for i in 1..=m {
        for j in 1..=n {
            let sub = dp[i - 1][j - 1] + usize::from(reference[i - 1] != obs_norm[j - 1]);
            dp[i][j] = sub.min(dp[i - 1][j] + 1).min(dp[i][j - 1] + 1);
        }
    }

    // Backtrack only diagonal alignments. Substitutions are intentionally retained as mappings;
    // deletions/insertions simply move on one side.
    let mut pairs = Vec::<(usize, usize)>::new();
    let (mut i, mut j) = (m, n);
    while i > 0 || j > 0 {
        if i > 0 && j > 0 {
            let cost = usize::from(reference[i - 1] != obs_norm[j - 1]);
            if dp[i][j] == dp[i - 1][j - 1] + cost {
                pairs.push((i - 1, j - 1));
                i -= 1;
                j -= 1;
                continue;
            }
        }
        if i > 0 && dp[i][j] == dp[i - 1][j] + 1 {
            i -= 1;
        } else if j > 0 {
            j -= 1;
        } else {
            break;
        }
    }
    pairs.reverse();

    let right_ref_start = left_len + target_len;
    let left_obs_end = pairs.iter()
        .filter(|(ri, _)| *ri < left_len)
        .map(|(_, oj)| *oj)
        .max()?;
    let right_obs_start = pairs.iter()
        .filter(|(ri, _)| *ri >= right_ref_start)
        .map(|(_, oj)| *oj)
        .min()?;
    if left_obs_end >= right_obs_start || right_obs_start > observed.len() {
        return None;
    }

    let raw_start = observed[left_obs_end].byte_end.min(expanded.len());
    let raw_end = observed[right_obs_start].byte_start.min(expanded.len());
    if raw_end < raw_start { return None; }
    let replacement = strip_leading_boundary_marks(&expanded[raw_start..raw_end])
        .trim()
        .to_string();

    let before = expanded[..observed[left_obs_end].byte_end.min(expanded.len())].trim();
    let after = expanded[observed[right_obs_start].byte_start.min(expanded.len())..].trim();
    Some(LocalRewriteExtraction {
        replacement_text: replacement,
        left_context_similarity: context_preservation(left, before),
        right_context_similarity: context_preservation(right, after),
    })
}

fn unit_ratio(first: &str, replacement: &str) -> f32 {
    let a = normalized_units(first).len().max(1) as f32;
    let b = normalized_units(replacement).len() as f32;
    b / a
}

fn classify_correction(
    first: &str,
    expanded: &str,
    reasons: &[SuspicionReason],
    edit_ratio: f32,
) -> CorrectionKind {
    if expanded.trim().is_empty() {
        return CorrectionKind::FragmentRemoval;
    }
    if reasons.iter().any(|r| matches!(r, SuspicionReason::BoundaryFragment)) && edit_ratio <= 0.65 {
        return CorrectionKind::BoundaryReconstruction;
    }
    if reasons.iter().any(|r| matches!(r, SuspicionReason::EntityVariant)) && edit_ratio <= 0.55 {
        return CorrectionKind::EntityReplacement;
    }
    if edit_ratio <= 0.55 {
        return CorrectionKind::LexicalReplacement;
    }
    let _ = first;
    CorrectionKind::LargeRewrite
}

/// Conservative v26 gate. Expanded Nano is the only re-ASR authority, but it may change Canonical
/// only when the change is local, time-grounded, and surrounded by preserved context. The gate
/// never tries to infer which word is semantically "more plausible".
pub fn assess_expanded_candidate(
    first_pass: &str,
    expanded_target: &str,
    reasons: &[SuspicionReason],
    suspicion_score: f32,
    left_context_similarity: f32,
    right_context_similarity: f32,
    target_time_coverage: f32,
    time_grounded: bool,
    text_aligned: bool,
) -> ExpandedSafetyAssessment {
    let expanded = expanded_target.trim();
    let edit_ratio = token_edit_ratio(first_pass, expanded);
    let replacement_ratio = unit_ratio(first_pass, expanded);
    let kind = classify_correction(first_pass, expanded, reasons, edit_ratio);
    let context_floor = left_context_similarity.min(right_context_similarity);
    let mut gate_reasons = Vec::<String>::new();

    if !time_grounded {
        gate_reasons.push("TARGET_NOT_PRECISELY_TIME_GROUNDED".into());
    }
    if !text_aligned {
        gate_reasons.push("TARGET_NOT_TEXT_ALIGNED".into());
    }

    if surfaces_equivalent(first_pass, expanded) {
        let temporal_ok = time_grounded && target_time_coverage >= 0.35;
        let alignment_ok = text_aligned && context_floor >= 0.65;
        let safe = context_floor >= 0.40 && (temporal_ok || alignment_ok);
        if !safe {
            if time_grounded && target_time_coverage < 0.35 {
                gate_reasons.push("TARGET_TIME_COVERAGE_LOW".into());
            }
            if context_floor < 0.40 {
                gate_reasons.push("CONTEXT_PRESERVATION_LOW".into());
            }
            if !temporal_ok && !alignment_ok {
                gate_reasons.push("NO_RELIABLE_LOCALIZATION".into());
            }
        } else {
            gate_reasons.push("EXPANDED_MATCHES_FIRST".into());
        }
        return ExpandedSafetyAssessment {
            correction_kind: kind,
            decision: if safe { VerificationDecision::Verified } else { VerificationDecision::Uncertain },
            replacement_text: None,
            confidence: if safe {
                let evidence_bonus = if time_grounded { 0.05 } else { 0.0 };
                (0.74 + suspicion_score * 0.08 + context_floor * 0.08 + evidence_bonus).min(0.94)
            } else {
                0.42
            },
            edit_ratio,
            replacement_ratio,
            reasons: gate_reasons,
        };
    }

    // Without either precise SRT localization or a successful constrained text alignment there is
    // no trustworthy way to map the Expanded surface back to RewriteSpan.
    if !time_grounded && !text_aligned {
        gate_reasons.push("NO_RELIABLE_LOCALIZATION".into());
        return ExpandedSafetyAssessment {
            correction_kind: kind,
            decision: VerificationDecision::Uncertain,
            replacement_text: None,
            confidence: 0.32,
            edit_ratio,
            replacement_ratio,
            reasons: gate_reasons,
        };
    }

    let accepted = match kind {
        CorrectionKind::BoundaryReconstruction => {
            let (context_min, edit_max, replacement_max) = if time_grounded {
                (0.55, 0.55, 1.65)
            } else {
                (0.72, 0.50, 1.50)
            };
            let coverage_ok = !time_grounded || target_time_coverage >= 0.45;
            let ok = context_floor >= context_min
                && coverage_ok
                && edit_ratio <= edit_max
                && replacement_ratio <= replacement_max;
            if context_floor < context_min { gate_reasons.push("CONTEXT_PRESERVATION_LOW".into()); }
            if !coverage_ok { gate_reasons.push("TARGET_TIME_COVERAGE_LOW".into()); }
            if edit_ratio > edit_max { gate_reasons.push("EDIT_NOT_LOCAL".into()); }
            if replacement_ratio > replacement_max { gate_reasons.push("REPLACEMENT_EXPANDS_TOO_MUCH".into()); }
            ok
        }
        CorrectionKind::FragmentRemoval => {
            // A zero-text target has no direct cue coverage by definition. Only allow deletion of a
            // very small suspicious/rewrite surface when both surrounding contexts survive strongly.
            let short_fragment = normalized_units(first_pass).len() <= 8;
            let context_min = if time_grounded { 0.80 } else { 0.85 };
            let ok = short_fragment
                && text_aligned
                && left_context_similarity >= context_min
                && right_context_similarity >= context_min;
            if !short_fragment { gate_reasons.push("FRAGMENT_TOO_LARGE_TO_DELETE".into()); }
            if !text_aligned { gate_reasons.push("DELETION_NOT_TEXT_ALIGNED".into()); }
            if left_context_similarity < context_min || right_context_similarity < context_min {
                gate_reasons.push("DELETION_CONTEXT_NOT_STABLE".into());
            }
            ok
        }
        CorrectionKind::LexicalReplacement => {
            let (context_min, edit_max, ratio_min, ratio_max) = if time_grounded {
                (0.70, 0.50, 0.50, 1.80)
            } else {
                (0.82, 0.35, 0.65, 1.50)
            };
            let coverage_ok = !time_grounded || target_time_coverage >= 0.55;
            let ok = context_floor >= context_min
                && coverage_ok
                && edit_ratio <= edit_max
                && (ratio_min..=ratio_max).contains(&replacement_ratio);
            if context_floor < context_min { gate_reasons.push("CONTEXT_PRESERVATION_LOW".into()); }
            if !coverage_ok { gate_reasons.push("TARGET_TIME_COVERAGE_LOW".into()); }
            if edit_ratio > edit_max { gate_reasons.push("EDIT_NOT_LOCAL".into()); }
            if !(ratio_min..=ratio_max).contains(&replacement_ratio) { gate_reasons.push("REPLACEMENT_SIZE_UNSTABLE".into()); }
            ok
        }
        CorrectionKind::EntityReplacement => {
            // Entity memory may select a candidate, but it must never make acceptance easier than
            // an ordinary lexical rewrite. Current-document memory is advisory only.
            let (context_min, edit_max, ratio_min, ratio_max) = if time_grounded {
                (0.72, 0.40, 0.60, 1.60)
            } else {
                (0.84, 0.30, 0.70, 1.40)
            };
            let coverage_ok = !time_grounded || target_time_coverage >= 0.50;
            let ok = context_floor >= context_min
                && coverage_ok
                && edit_ratio <= edit_max
                && (ratio_min..=ratio_max).contains(&replacement_ratio);
            if context_floor < context_min { gate_reasons.push("CONTEXT_PRESERVATION_LOW".into()); }
            if !coverage_ok { gate_reasons.push("TARGET_TIME_COVERAGE_LOW".into()); }
            if edit_ratio > edit_max { gate_reasons.push("EDIT_NOT_LOCAL".into()); }
            if !(ratio_min..=ratio_max).contains(&replacement_ratio) { gate_reasons.push("REPLACEMENT_SIZE_UNSTABLE".into()); }
            ok
        }
        CorrectionKind::LargeRewrite => {
            gate_reasons.push("LARGE_REWRITE_REJECTED".into());
            false
        }
    };

    if accepted {
        if time_grounded {
            gate_reasons.push("TIME_GROUNDED_LOCAL_REWRITE".into());
        } else {
            gate_reasons.push("TEXT_ALIGNED_LOCAL_REWRITE".into());
        }
        let replacement = preserve_first_surface_for_matching_tokens(first_pass, expanded);
        if replacement != expanded {
            gate_reasons.push("LEXICAL_REWRITE_SURFACE_PRESERVED".into());
        }
        let quality = (1.0 - edit_ratio) * 0.22
            + context_floor * 0.28
            + if time_grounded { target_time_coverage * 0.18 } else { 0.0 }
            + suspicion_score.clamp(0.0, 1.0) * 0.10;
        let (base, cap) = if time_grounded { (0.66, 0.96) } else { (0.60, 0.88) };
        ExpandedSafetyAssessment {
            correction_kind: kind,
            decision: VerificationDecision::Corrected,
            replacement_text: Some(replacement),
            confidence: (base + quality).min(cap),
            edit_ratio,
            replacement_ratio,
            reasons: gate_reasons,
        }
    } else {
        ExpandedSafetyAssessment {
            correction_kind: kind,
            decision: VerificationDecision::Uncertain,
            replacement_text: None,
            confidence: (0.28 + context_floor * 0.12 + if time_grounded { target_time_coverage * 0.08 } else { 0.0 }).min(0.58),
            edit_ratio,
            replacement_ratio,
            reasons: gate_reasons,
        }
    }
}

fn meaningful_units(text: &str) -> usize {
    let tokens = tokenize(text);
    if tokens.is_empty() {
        text.chars().filter(|c| c.is_alphanumeric() || is_cjk_like(*c)).count()
    } else {
        tokens.len()
    }
}

fn is_title_word(raw: &str) -> bool {
    let mut chars = raw.chars();
    let Some(first) = chars.next() else { return false };
    first.is_ascii_uppercase()
        && raw.chars().filter(|c| c.is_ascii_alphabetic()).count() >= 2
        && raw.chars().all(|c| c.is_ascii_alphabetic() || matches!(c, '\'' | '-'))
}

fn is_camel_or_acronym(raw: &str) -> bool {
    let upper = raw.chars().filter(|c| c.is_ascii_uppercase()).count();
    let lower = raw.chars().filter(|c| c.is_ascii_lowercase()).count();
    let acronym = raw.len() >= 2 && upper >= 2 && lower == 0;
    let camel = raw.len() >= 4 && upper >= 2 && lower >= 1;
    acronym || camel
}

fn is_entity_stopword(raw: &str) -> bool {
    matches!(
        raw.to_ascii_lowercase().as_str(),
        "a" | "an" | "and" | "are" | "as" | "at" | "be" | "been" | "but" | "by"
            | "for" | "from" | "had" | "has" | "have" | "he" | "her" | "hers" | "him"
            | "his" | "i" | "if" | "in" | "into" | "is" | "it" | "its" | "me" | "my"
            | "no" | "not" | "of" | "on" | "or" | "our" | "please" | "she" | "so"
            | "that" | "the" | "their" | "them" | "there" | "they" | "this" | "to"
            | "was" | "we" | "were" | "what" | "when" | "where" | "which" | "who"
            | "will" | "with" | "would" | "you" | "your"
    )
}

fn honorific_key(raw: &str) -> Option<&'static str> {
    match raw.trim_end_matches('.').to_ascii_lowercase().as_str() {
        "mrs" | "missus" => Some("mrs"),
        "mr" | "mister" => Some("mr"),
        "ms" => Some("ms"),
        "dr" | "doctor" => Some("dr"),
        _ => None,
    }
}

fn honorific_display(key: &str) -> &'static str {
    match key {
        "mrs" => "Mrs.",
        "mr" => "Mr.",
        "ms" => "Ms.",
        "dr" => "Dr.",
        _ => "",
    }
}

#[derive(Debug, Clone)]
struct EntityCandidate {
    surface: String,
    normalized: String,
}

fn ascii_words(text: &str) -> Vec<String> {
    let mut raw_words = Vec::<String>::new();
    let mut buf = String::new();
    for ch in text.chars() {
        if ch.is_ascii_alphabetic() || ch == '\'' || ch == '-' {
            buf.push(ch);
        } else if ch == '.' && !buf.is_empty() {
            // Flush at the period so compact forms such as `Mrs.Stingy` become
            // [`Mrs.`, `Stingy`] instead of one malformed token.
            buf.push(ch);
            raw_words.push(std::mem::take(&mut buf));
        } else if !buf.is_empty() {
            raw_words.push(std::mem::take(&mut buf));
        }
    }
    if !buf.is_empty() {
        raw_words.push(buf);
    }
    raw_words
}

fn normalized_entity_surface(surface: &str) -> String {
    ascii_words(surface)
        .into_iter()
        .map(|word| {
            honorific_key(&word)
                .map(str::to_string)
                .unwrap_or_else(|| word.trim_end_matches('.').to_ascii_lowercase())
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn entity_candidates(text: &str) -> Vec<EntityCandidate> {
    let words = ascii_words(text);
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < words.len() {
        let raw = words[i].trim_end_matches('.');
        if let Some(key) = honorific_key(&words[i]) {
            if let Some(next) = words.get(i + 1) {
                let next_clean = next.trim_end_matches('.');
                if is_title_word(next_clean) && !is_entity_stopword(next_clean) {
                    let surface = if words[i].trim_end_matches('.').eq_ignore_ascii_case("missus")
                        || words[i].trim_end_matches('.').eq_ignore_ascii_case("mister")
                        || words[i].trim_end_matches('.').eq_ignore_ascii_case("doctor")
                    {
                        format!("{} {}", words[i].trim_end_matches('.'), next_clean)
                    } else {
                        format!("{} {}", honorific_display(key), next_clean)
                    };
                    out.push(EntityCandidate {
                        normalized: format!("{} {}", key, next_clean.to_ascii_lowercase()),
                        surface,
                    });
                    i += 2;
                    continue;
                }
            }
        }

        if is_camel_or_acronym(raw) && !is_entity_stopword(raw) {
            out.push(EntityCandidate {
                surface: raw.to_string(),
                normalized: raw.to_ascii_lowercase(),
            });
        }
        i += 1;
    }
    out
}

fn canonical_alias_score(surface: &str, count: usize) -> (usize, usize, String) {
    let first = ascii_words(surface).first().cloned().unwrap_or_default();
    let abbreviated_honorific = honorific_key(&first).is_some()
        && first.trim_end_matches('.').len() <= 3;
    (usize::from(abbreviated_honorific), count, surface.to_ascii_lowercase())
}

fn looks_like_all_caps_prose(text: &str) -> bool {
    let letters = text.chars().filter(|c| c.is_ascii_alphabetic()).collect::<Vec<_>>();
    if letters.len() < 12 {
        return false;
    }
    let words = ascii_words(text)
        .into_iter()
        .filter(|w| w.chars().any(|c| c.is_ascii_alphabetic()))
        .count();
    if words < 3 {
        return false;
    }
    let uppercase = letters.iter().filter(|c| c.is_ascii_uppercase()).count();
    uppercase as f32 / letters.len() as f32 >= 0.90
}

fn is_plain_all_caps_entity_surface(surface: &str) -> bool {
    let letters = surface.chars().filter(|c| c.is_ascii_alphabetic()).collect::<Vec<_>>();
    letters.len() >= 2
        && letters.iter().all(|c| c.is_ascii_uppercase())
        && !surface.chars().any(|c| c.is_ascii_lowercase())
}

pub fn build_entity_memory(segments: &[VerificationSegment]) -> EntityMemory {
    let mut groups: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    for segment in segments {
        let all_caps_prose = looks_like_all_caps_prose(&segment.text);
        for entity in entity_candidates(&segment.text) {
            // Decoder-wide ALL-CAPS prose is a presentation state, not entity evidence.
            // Ignore plain uppercase tokens from such spans instead of teaching the current
            // document memory `IT'S`, `KNOW`, `GET`, ... as stable entities. Honorific phrases
            // (for example `Mrs. STINGY`) remain eligible because their surface is not plain
            // all-caps, and acronyms inside normal mixed-case/CJK prose remain eligible too.
            if all_caps_prose && is_plain_all_caps_entity_surface(&entity.surface) {
                continue;
            }
            *groups
                .entry(entity.normalized)
                .or_default()
                .entry(entity.surface)
                .or_default() += 1;
        }
    }

    let mut stable = groups
        .into_iter()
        .filter_map(|(normalized, aliases)| {
            let occurrences = aliases.values().copied().sum::<usize>();
            if occurrences < 2 {
                return None;
            }
            let mut alias_rows = aliases.into_iter().collect::<Vec<_>>();
            alias_rows.sort_by(|(a_surface, a_count), (b_surface, b_count)| {
                canonical_alias_score(b_surface, *b_count)
                    .cmp(&canonical_alias_score(a_surface, *a_count))
            });
            let canonical = alias_rows.first()?.0.clone();
            let aliases = alias_rows.into_iter().map(|(surface, _)| surface).collect::<Vec<_>>();
            Some(StableEntity {
                canonical,
                normalized,
                occurrences,
                aliases,
            })
        })
        .collect::<Vec<_>>();
    stable.sort_by(|a, b| {
        b.occurrences
            .cmp(&a.occurrences)
            .then_with(|| a.canonical.cmp(&b.canonical))
    });
    stable.truncate(32);
    EntityMemory { stable }
}

fn replace_ascii_case_insensitive(mut text: String, needle: &str, replacement: &str) -> String {
    if needle.is_empty() || needle.eq_ignore_ascii_case(replacement) {
        return text;
    }
    let needle_lower = needle.to_ascii_lowercase();
    let mut from = 0usize;
    loop {
        let lower = text.to_ascii_lowercase();
        let Some(relative) = lower[from..].find(&needle_lower) else { break };
        let start = from + relative;
        let end = start + needle.len();
        if end > text.len() {
            break;
        }
        text.replace_range(start..end, replacement);
        from = start + replacement.len();
        if from >= text.len() {
            break;
        }
    }
    text
}

fn generated_entity_aliases(entity: &StableEntity) -> Vec<String> {
    let mut aliases = entity.aliases.clone();
    aliases.push(entity.canonical.clone());
    let words = ascii_words(&entity.canonical);
    if words.len() >= 2 {
        let first = words[0].trim_end_matches('.');
        let rest = words[1..]
            .iter()
            .map(|w| w.trim_end_matches('.'))
            .collect::<Vec<_>>()
            .join(" ");
        match honorific_key(first) {
            Some("mrs") => {
                aliases.push(format!("Mrs {}", rest));
                aliases.push(format!("Mrs.{}", rest));
                aliases.push(format!("Missus {}", rest));
            }
            Some("mr") => {
                aliases.push(format!("Mr {}", rest));
                aliases.push(format!("Mr.{}", rest));
                aliases.push(format!("Mister {}", rest));
            }
            Some("dr") => {
                aliases.push(format!("Dr {}", rest));
                aliases.push(format!("Dr.{}", rest));
                aliases.push(format!("Doctor {}", rest));
            }
            _ => {}
        }
    }
    aliases.sort_by_key(|value| std::cmp::Reverse(value.len()));
    aliases.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    aliases
}

/// Canonicalize only *known, stable* document entities inside an Expanded Nano candidate.
/// Raw remains untouched. This is intentionally narrower than general text normalization.
pub fn canonicalize_known_entities(text: &str, memory: &EntityMemory) -> String {
    let mut out = text.to_string();
    for entity in &memory.stable {
        for alias in generated_entity_aliases(entity) {
            if normalized_entity_surface(&alias) == entity.normalized {
                out = replace_ascii_case_insensitive(out, &alias, &entity.canonical);
            }
        }
    }
    out
}

fn levenshtein_bounded(a: &str, b: &str, max_distance: usize) -> Option<usize> {
    let aa = a.as_bytes();
    let bb = b.as_bytes();
    if aa.len().abs_diff(bb.len()) > max_distance { return None; }
    let mut prev = (0..=bb.len()).collect::<Vec<_>>();
    for (i, ca) in aa.iter().enumerate() {
        let mut row = vec![i + 1; bb.len() + 1];
        let mut row_min = row[0];
        for (j, cb) in bb.iter().enumerate() {
            row[j + 1] = (prev[j + 1] + 1)
                .min(row[j] + 1)
                .min(prev[j] + usize::from(ca != cb));
            row_min = row_min.min(row[j + 1]);
        }
        if row_min > max_distance { return None; }
        prev = row;
    }
    let d = prev[bb.len()];
    (d <= max_distance).then_some(d)
}

fn entity_variants_in_segment(text: &str, memory: &EntityMemory) -> Vec<String> {
    let stable_set = memory
        .stable
        .iter()
        .map(|e| e.normalized.as_str())
        .collect::<HashSet<_>>();
    let mut variants = Vec::new();
    for candidate in entity_candidates(text) {
        let norm = candidate.normalized;
        if stable_set.contains(norm.as_str()) {
            continue;
        }
        for entity in &memory.stable {
            if norm.len().min(entity.normalized.len()) < 4 {
                continue;
            }
            let max = if norm.len().max(entity.normalized.len()) >= 10 { 2 } else { 1 };
            if levenshtein_bounded(&norm, &entity.normalized, max).is_some() {
                variants.push(entity.canonical.clone());
                break;
            }
        }
    }
    variants.sort();
    variants.dedup();
    variants
}

pub fn detect_suspicions(
    segments: &[VerificationSegment],
    memory: &EntityMemory,
    max_candidates: usize,
) -> Vec<SuspicionCandidate> {
    if segments.is_empty() { return Vec::new(); }
    let mut candidates = Vec::new();

    for (i, segment) in segments.iter().enumerate() {
        let duration_ms = segment.end_ms.saturating_sub(segment.start_ms);
        let units = meaningful_units(&segment.text);
        let micro = duration_ms <= 1_400 && units <= 6 || duration_ms <= 550;
        let left_gap = i.checked_sub(1).map(|p| segment.start_ms.saturating_sub(segments[p].end_ms));
        let right_gap = (i + 1 < segments.len()).then(|| segments[i + 1].start_ms.saturating_sub(segment.end_ms));
        let isolated = micro
            && i > 0
            && i + 1 < segments.len()
            && left_gap.is_some_and(|g| g <= 1_200)
            && right_gap.is_some_and(|g| g <= 1_200)
            && segments[i - 1].end_ms.saturating_sub(segments[i - 1].start_ms) >= 1_200
            && segments[i + 1].end_ms.saturating_sub(segments[i + 1].start_ms) >= 1_200;

        let variants = entity_variants_in_segment(&segment.text, memory);
        let decoder_health = analyze_decoder_surface(&segment.text, duration_ms);
        let mut reasons = Vec::new();
        let mut score = 0.0_f32;
        if micro { reasons.push(SuspicionReason::MicroSegment); score += 0.36; }
        if isolated { reasons.push(SuspicionReason::IsolatedMicroSegment); score += 0.34; }
        if isolated && left_gap.unwrap_or(u64::MAX) <= 600 && right_gap.unwrap_or(u64::MAX) <= 600 {
            reasons.push(SuspicionReason::BoundaryFragment);
            score += 0.14;
        }
        if !variants.is_empty() {
            reasons.push(SuspicionReason::EntityVariant);
            score += 0.58;
        }
        if decoder_health.severe {
            reasons.push(SuspicionReason::DecoderSurfaceDegeneration);
            score += 0.92;
        }
        if reasons.is_empty() || score < 0.52 { continue; }

        // SuspiciousSpan stays exact. RewriteSpan is allowed to include the previous cue only for a
        // tightly attached boundary fragment and only while the resulting rewrite remains small.
        // This prevents a 1s micro cue from accidentally turning a 12s previous cue into the target.
        let suspicious_indices = vec![i];
        let previous_duration = i.checked_sub(1)
            .map(|p| segments[p].end_ms.saturating_sub(segments[p].start_ms))
            .unwrap_or(u64::MAX);
        let previous_gap = left_gap.unwrap_or(u64::MAX);
        let include_previous = reasons.contains(&SuspicionReason::BoundaryFragment)
            && i > 0
            && previous_gap <= 600
            && previous_duration <= 6_000
            && segment.end_ms.saturating_sub(segments[i - 1].start_ms) <= 8_000;
        let target_indices = if include_previous { vec![i - 1, i] } else { vec![i] };
        let start_index = *target_indices.first().unwrap_or(&i);
        let end_index = *target_indices.last().unwrap_or(&i);
        // Ordinary Expanded verification needs one Raw cue on both sides as safety evidence.
        // Decoder-surface retry is different: it re-decodes only the exact suspect span in a fresh
        // process, so a severe first/last segment may still be inspected safely.
        let surface_degenerated = reasons.contains(&SuspicionReason::DecoderSurfaceDegeneration);
        if !surface_degenerated && (start_index == 0 || end_index + 1 >= segments.len()) { continue; }

        let mut hotwords = variants;
        for entity in memory.stable.iter().take(12) {
            if !hotwords.iter().any(|h| h.eq_ignore_ascii_case(&entity.canonical)) {
                hotwords.push(entity.canonical.clone());
            }
            if hotwords.len() >= 12 { break; }
        }

        candidates.push(SuspicionCandidate {
            suspicious_indices: suspicious_indices.clone(),
            suspicious_segment_ids: suspicious_indices.iter().map(|idx| segments[*idx].id.clone()).collect(),
            suspicious_start_ms: segment.start_ms,
            suspicious_end_ms: segment.end_ms,
            target_indices: target_indices.clone(),
            target_segment_ids: target_indices.iter().map(|idx| segments[*idx].id.clone()).collect(),
            start_ms: segments[start_index].start_ms,
            end_ms: segments[end_index].end_ms,
            score: score.min(1.0),
            reasons,
            hotwords,
        });
    }

    // Prefer high-value candidates and avoid re-verifying overlapping spans.
    candidates.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.start_ms.cmp(&b.start_ms)));
    let mut selected = Vec::new();
    let mut occupied = HashSet::new();
    for candidate in candidates {
        if candidate.suspicious_indices.iter().any(|i| occupied.contains(i)) { continue; }
        for i in &candidate.suspicious_indices { occupied.insert(*i); }
        selected.push(candidate);
        if selected.len() >= max_candidates { break; }
    }
    selected.sort_by_key(|c| c.start_ms);
    selected
}

pub fn target_surface(segments: &[VerificationSegment], candidate: &SuspicionCandidate) -> String {
    candidate.target_indices.iter()
        .filter_map(|idx| segments.get(*idx))
        .map(|s| s.text.trim())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(id: &str, start: u64, end: u64, text: &str) -> VerificationSegment {
        VerificationSegment { id: id.into(), start_ms: start, end_ms: end, text: text.into() }
    }

    #[test]
    fn finds_short_isolated_fragment_without_language_rules() {
        let segments = vec![
            seg("a", 0, 3_000, "She had no family and no friend."),
            seg("b", 3_020, 3_340, "起。"),
            seg("c", 3_360, 6_000, "There was only one thing she cared about."),
        ];
        // Need an outer left anchor, just as production verification does.
        let segments = [vec![seg("pre", 0, 1_500, "Mrs. Stingy lived alone.")], segments].concat();
        let memory = build_entity_memory(&segments);
        let found = detect_suspicions(&segments, &memory, 8);
        assert!(found.iter().any(|c| c.reasons.contains(&SuspicionReason::IsolatedMicroSegment)));
    }

    #[test]
    fn stable_entity_flags_near_variant() {
        let segments = vec![
            seg("a", 0, 2_000, "Mrs. Stingy lived alone."),
            seg("b", 2_100, 4_000, "Mrs. Stingy had an old house."),
            seg("c", 4_100, 6_000, "The next morning Mrs. Stinge woke early."),
            seg("d", 6_100, 8_000, "She smiled at the children."),
        ];
        let memory = build_entity_memory(&segments);
        assert!(memory.stable.iter().any(|e| e.canonical.eq_ignore_ascii_case("Mrs. Stingy")));
        let found = detect_suspicions(&segments, &memory, 8);
        assert!(found.iter().any(|c| c.reasons.contains(&SuspicionReason::EntityVariant)));
    }

    #[test]
    fn long_all_caps_unpunctuated_segment_is_surface_degeneration_candidate() {
        let segments = vec![
            seg("pre", 0, 2_000, "A normal sentence."),
            seg("bad", 2_100, 22_900, "HOW'S THE GAME PRETTY GOOD RIGHT I DON'T THINK IT'S FUN DO YOU THINK SO I'M NOT INTO IT EITHER I THINK WE SHOULD PLAY BRIDGE I THINK WE SHOULD PLAY BRIDGE TOO"),
            seg("post", 23_000, 25_000, "Another normal sentence."),
        ];
        let memory = build_entity_memory(&segments);
        let found = detect_suspicions(&segments, &memory, 8);
        assert!(found.iter().any(|candidate| {
            candidate.suspicious_segment_ids == vec!["bad".to_string()]
                && candidate.reasons.contains(&SuspicionReason::DecoderSurfaceDegeneration)
        }));
    }

    #[test]
    fn expanded_gate_accepts_local_boundary_reconstruction() {
        let assessed = assess_expanded_candidate(
            "She had no family and no friend. 起。",
            "She had no family and no friends.",
            &[SuspicionReason::MicroSegment, SuspicionReason::IsolatedMicroSegment, SuspicionReason::BoundaryFragment],
            0.84,
            0.90,
            0.95,
            0.88,
            true,
            true,
        );
        assert_eq!(assessed.decision, VerificationDecision::Corrected);
        assert_eq!(assessed.correction_kind, CorrectionKind::BoundaryReconstruction);
    }

    #[test]
    fn expanded_gate_rejects_large_rewrite() {
        let assessed = assess_expanded_candidate(
            "带着三人走。有。",
            "开着开着开着，八下车窗挂挂二档来三走。",
            &[SuspicionReason::MicroSegment],
            0.70,
            0.35,
            0.42,
            0.70,
            true,
            true,
        );
        assert_eq!(assessed.decision, VerificationDecision::Uncertain);
    }

    #[test]
    fn lexical_rewrite_preserves_matching_first_pass_casing() {
        let merged = preserve_first_surface_for_matching_tokens(
            "It's my fault. Why did I take on this task? 啊啊啊。",
            "IT'S MY FAULT. WHY DID I TAKE ON THIS TASK?",
        );
        assert!(merged.starts_with("It's my fault"));
        assert!(merged.contains("Why did I take on this task"));
        assert!(!merged.contains("IT'S MY FAULT"));
    }

    #[test]
    fn lexical_patch_does_not_decide_final_casing() {
        let merged = preserve_first_surface_for_matching_tokens(
            "YES, MY DUCKLINGS DON'T FORGET. GET RIGHT LEFT.",
            "Yes, my ducklings don't forget right left quack.",
        );
        assert!(merged.starts_with("YES, MY DUCKLINGS"));
        assert!(merged.to_ascii_lowercase().contains("right left quack"));
    }

    #[test]
    fn single_cue_text_alignment_is_not_labeled_time_grounded() {
        let assessed = assess_expanded_candidate(
            "It's my fault. 啊啊啊。",
            "IT'S MY FAULT.",
            &[SuspicionReason::MicroSegment, SuspicionReason::BoundaryFragment],
            0.84,
            0.86,
            0.86,
            1.0,
            false,
            true,
        );
        assert_eq!(assessed.decision, VerificationDecision::Corrected);
        assert!(assessed.reasons.iter().any(|r| r == "TEXT_ALIGNED_LOCAL_REWRITE"));
        assert!(!assessed.reasons.iter().any(|r| r == "TIME_GROUNDED_LOCAL_REWRITE"));
        assert!(assessed.confidence <= 0.88);
    }

    #[test]
    fn all_caps_prose_does_not_seed_plain_token_entities() {
        let segments = vec![
            seg("a", 0, 2_000, "IT'S MY FAULT AND I DON'T KNOW"),
            seg("b", 2_100, 4_000, "IT'S MY FAULT AND I DON'T KNOW"),
            seg("c", 4_100, 6_000, "城市 CLTC 折现率为 100.2%。"),
            seg("d", 6_100, 8_000, "另一次 CLTC 测试。"),
        ];
        let memory = build_entity_memory(&segments);
        assert!(!memory.stable.iter().any(|e| e.canonical.eq_ignore_ascii_case("IT'S")));
        assert!(!memory.stable.iter().any(|e| e.canonical.eq_ignore_ascii_case("KNOW")));
        assert!(memory.stable.iter().any(|e| e.canonical.eq_ignore_ascii_case("CLTC")));
    }

    #[test]
    fn consensus_normalization_ignores_punctuation_case_and_honorific_spelling() {
        assert!(surfaces_equivalent("Mrs. Stingy!", "mrs stingy"));
        assert!(surfaces_equivalent("Missus Stingy", "Mrs. Stingy"));
    }

    #[test]
    fn entity_memory_groups_missus_and_mrs_and_filters_sentence_words() {
        let segments = vec![
            seg("a", 0, 2_000, "Mrs. Stingy lived alone."),
            seg("b", 2_100, 4_000, "Missus Stingy had an old house."),
            seg("c", 4_100, 6_000, "The children saw Mrs.Stingy again."),
        ];
        let memory = build_entity_memory(&segments);
        let stingy = memory
            .stable
            .iter()
            .find(|e| e.normalized == "mrs stingy")
            .expect("stable Mrs. Stingy entity");
        assert_eq!(stingy.canonical, "Mrs. Stingy");
        assert_eq!(stingy.occurrences, 3);
        assert!(!memory.stable.iter().any(|e| e.canonical.eq_ignore_ascii_case("The")));
        assert_eq!(
            canonicalize_known_entities("Missus Stingy smiled.", &memory),
            "Mrs. Stingy smiled."
        );
        assert_eq!(
            canonicalize_known_entities("Mrs.Stingy smiled.", &memory),
            "Mrs. Stingy smiled."
        );
    }
}

pub mod alignment;
pub mod boundary;
pub mod dedupe;
pub mod edit;
pub mod entity;
pub mod integrity;
pub mod itn;
pub mod punctuation_repair;
pub mod stitch;
pub mod surface_repair;
pub mod semantic_boundary;
pub mod typography;
pub mod verification;

use crate::transcript::model::{CanonicalTranscript, LanguageProfile, RawTranscript};
use crate::transcript::transform::TransformLog;
pub use verification::VerificationRewriteEvidence;

/// Boundary evidence is deliberately typed. A pause is acoustic evidence only and must
/// never be promoted to a Canonical sentence split by itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundaryEvidenceKind {
    /// VAD/CTC detected a real pause. Useful for subtitle layout and for corroborating
    /// a textual sentence candidate, but not sufficient to split Canonical by itself.
    AcousticPause,
    /// A language/text detector found a strong sentence-ending punctuation candidate.
    StrongPunctuation,
    /// A word/token alignment located a textual sentence candidate on the audio timeline.
    AlignmentBoundary,
    /// Runtime/ASR chunk edge. This is diagnostic evidence, not a sentence boundary.
    ChunkBoundary,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BoundaryEvidence {
    pub segment_id: String,
    /// Character offset inside the whitespace-normalized Raw segment text.
    pub char_offset: usize,
    pub time_ms: u64,
    pub gap_ms: u64,
    pub confidence: f32,
    pub kind: BoundaryEvidenceKind,
}


/// A punctuation relocation is stronger than a generic pause but still separate from a sentence
/// split. The relocation first repairs the surface text; sentence candidates are derived afterwards.
#[derive(Debug, Clone, PartialEq)]
pub struct PunctuationRepairEvidence {
    pub segment_id: String,
    pub char_offset: usize,
    pub remove_segment_id: Option<String>,
    pub remove_char_offset: Option<usize>,
    pub time_ms: u64,
    pub confidence: f32,
}


#[derive(Debug, Clone, PartialEq)]
pub struct SurfaceRepairEvidence {
    pub target_segment_ids: Vec<String>,
    /// Decoder retry surface. The Canonical pipeline may copy punctuation separators from this
    /// text only when lexical units exactly match the current working segment.
    pub observed_text: String,
    pub confidence: f32,
    pub rule_id: String,
}

/// An authoritative bridge re-transcription may repair unstable text at a Raw cue edge.
/// Raw itself stays immutable; this evidence is applied only to the Canonical working copy.
#[derive(Debug, Clone, PartialEq)]
pub struct CrossBoundaryRewriteEvidence {
    pub left_segment_id: String,
    pub right_segment_id: String,
    pub left_text: Option<String>,
    pub right_text: Option<String>,
    /// When true the right segment is a verified ASR boundary fragment and is removed from
    /// the Canonical working copy after the bridge has repaired the left segment. Raw remains immutable.
    pub drop_right: bool,
    pub confidence: f32,
}

#[derive(Debug, Clone)]
pub struct PipelineConfig {
    /// Backward-compatible fallback only. `RawTranscript.language` is preferred when present.
    pub is_english_audio: bool,
    /// When true, preserves lexical surface fidelity (e.g. for MOSS), skipping aggressive ITN number mutators and lexical rewrites.
    pub preserve_lexical_fidelity: bool,
    /// Typed acoustic/alignment evidence. Acoustic pauses never directly split Canonical.
    pub boundary_evidence: Vec<BoundaryEvidence>,
    /// Optional high-confidence CTC punctuation relocations.
    pub punctuation_repairs: Vec<PunctuationRepairEvidence>,
    /// Audio-grounded verification rewrites accepted by the Expanded Nano Safety Gate.
    pub verification_rewrites: Vec<VerificationRewriteEvidence>,
    /// Presentation-only punctuation evidence from a reset/safe-window decoder retry.
    pub surface_repairs: Vec<SurfaceRepairEvidence>,
    /// Optional authoritative cross-Raw bridge rewrites.
    pub bridge_rewrites: Vec<CrossBoundaryRewriteEvidence>,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            is_english_audio: false,
            preserve_lexical_fidelity: false,
            boundary_evidence: Vec::new(),
            punctuation_repairs: Vec::new(),
            verification_rewrites: Vec::new(),
            surface_repairs: Vec::new(),
            bridge_rewrites: Vec::new(),
        }
    }
}

impl PipelineConfig {
    pub fn language_profile(&self, raw: &RawTranscript) -> LanguageProfile {
        let detected = LanguageProfile::from_language_tag(raw.language.as_deref());
        if detected != LanguageProfile::Auto {
            detected
        } else if self.is_english_audio {
            LanguageProfile::En
        } else {
            LanguageProfile::Auto
        }
    }
}

pub fn run_canonical_pipeline(raw: &RawTranscript, config: &PipelineConfig) -> (CanonicalTranscript, TransformLog) {
    let mut log = TransformLog::new(&raw.job_id);
    log.raw_revision_id = raw.metadata.raw_revision_id.clone();
    log.raw_content_hash = raw.metadata.raw_content_hash.clone();
    let profile = config.language_profile(raw);

    let guarded = integrity::run_integrity_guard(raw, profile, &mut log);
    let punctuated = punctuation_repair::apply_punctuation_repairs(guarded, profile, &config.punctuation_repairs, &mut log);
    let verified = if config.preserve_lexical_fidelity {
        punctuated
    } else {
        verification::apply_verification_rewrites(punctuated, &config.verification_rewrites, &mut log)
    };
    let stitched = if config.preserve_lexical_fidelity {
        verified
    } else {
        stitch::apply_cross_boundary_rewrites(verified, &config.bridge_rewrites, &mut log)
    };
    let surface_repaired = if config.preserve_lexical_fidelity {
        stitched
    } else {
        surface_repair::apply_surface_repairs(
            stitched,
            profile,
            &config.boundary_evidence,
            &config.surface_repairs,
            &mut log,
        )
    };
    let aligned_evidence = alignment::derive_sentence_boundary_evidence(&surface_repaired, profile, &config.boundary_evidence);
    let mut segments = boundary::resolve_boundaries(surface_repaired, profile, &aligned_evidence, &mut log);
    dedupe::run_conservative_dedupe(&mut segments, profile, &mut log);
    if !config.preserve_lexical_fidelity {
        itn::run_itn_engine(&mut segments, profile, &mut log);
        entity::run_entity_resolver(&mut segments, profile, &mut log);
    }
    semantic_boundary::run_final_semantic_boundary_review(&mut segments, profile, &mut log);
    typography::run_typography_normalizer(&mut segments, profile, &mut log);

    let canonical = CanonicalTranscript {
        job_id: raw.job_id.clone(),
        metadata: raw.metadata.clone(),
        language: raw.language.clone(),
        segments,
    };

    debug_assert!(validate_canonical_transcript(&canonical).is_ok());
    (canonical, log)
}

/// Verifies the invariants that make Canonical safe for subtitle/highlight/provenance consumers.
pub fn validate_canonical_transcript(canonical: &CanonicalTranscript) -> Result<(), Vec<String>> {
    let profile = LanguageProfile::from_language_tag(canonical.language.as_deref());
    let mut errors = Vec::new();
    for seg in &canonical.segments {
        if seg.start_ms > seg.end_ms {
            errors.push(format!("segment {} has invalid time range", seg.id));
        }
        for pair in seg.tokens.windows(2) {
            if pair[0].start_ms > pair[0].end_ms || pair[1].start_ms > pair[1].end_ms {
                errors.push(format!("segment {} contains invalid token time range", seg.id));
            }
            if pair[0].start_ms > pair[1].start_ms {
                errors.push(format!("segment {} tokens are not time ordered", seg.id));
            }
        }
        let projected = edit::render_tokens(&seg.tokens, profile);
        if projected != seg.text {
            errors.push(format!(
                "segment {} text/token projection mismatch: {:?} != {:?}",
                seg.id, seg.text, projected
            ));
        }
    }
    if errors.is_empty() { Ok(()) } else { Err(errors) }
}

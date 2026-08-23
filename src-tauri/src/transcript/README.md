# VideoNotes Transcript Pipeline v2.7.0

This module implements the conservative transcript architecture discussed for VideoNotes:

`Raw ASR (immutable) -> Integrity -> Verification -> Surface Repair -> Boundary/Punctuation -> Dedupe -> ITN -> Entity -> Final Semantic Boundary Review -> Typography -> Canonical -> Views`

## Core invariants

1. **Canonical does not guess user intent.** Fillers, short cross-language speech and rhetorical repetition are preserved.
2. **Canonical tokens are the fact layer; `CanonicalSegment.text` is their projection.** Every pipeline stage must keep them synchronized.
3. **All transformations are logged and source raw token ids are preserved when the ASR supplies tokens.**
4. **Raw revisions are immutable.** Production uses `save_raw_revision`, which stores each revision under `raw_revisions/` and updates `raw_transcript.json` only as the current compatibility pointer. The legacy `save_raw_transcript` API remains strict/no-overwrite.
5. **Domain terms are not hard-coded.** Core acronym logic turns a clear spoken sequence such as `C O T C` into `COTC`; a correction such as `COTC -> CLTC` belongs in an external glossary/hotword source.

## Important changes from the previous implementation

- Internal token-gap splitting is implemented in `pipeline/boundary.rs`.
- A strong terminal punctuation is no longer converted to a comma merely because a sentence is short.
- `Integrity` tags fillers/script outliers instead of deleting them.
- Canonical dedupe only removes chunk overlap and very high-confidence decode loops; ordinary `no no no` is preserved.
- ITN replacements mutate the affected `CanonicalToken` span and provenance together with text.
- `二百五`, `一千二`, `一万五` are treated as ambiguous and are not normalized in Canonical.
- English typography never deletes CJK content.
- Typography normalizes decoder-wide ALL-CAPS prose only in Canonical/Standard; Raw casing is never changed. Trusted acronym/name surfaces are protected conservatively.
- URL/code-like punctuation is protected conservatively.
- `LanguageProfile` is derived from `RawTranscript.language`; `PipelineConfig.is_english_audio` remains only as a backward-compatible fallback.
- `validate_canonical_transcript` checks the text/token projection invariant.

## Integration

The main public API is intentionally kept compatible:

```rust
let (canonical, transform_log) = run_canonical_pipeline(&raw, &PipelineConfig {
    is_english_audio: false,
    boundary_evidence: Vec::new(),
    punctuation_repairs: Vec::new(),
    verification_rewrites: Vec::new(),
    surface_repairs: Vec::new(),
    bridge_rewrites: Vec::new(),
});

validate_canonical_transcript(&canonical)?;
```

For domain aliases, inject them outside the core pipeline:

```rust
let glossary = vec![GlossaryEntry {
    canonical: "CLTC".into(),
    aliases: vec!["COTC".into()],
}];
apply_glossary(&mut canonical.segments, &glossary, LanguageProfile::Zh, &mut log);
```

Prefer ASR hotwords/context for known names when the backend supports them; use the glossary as deterministic post-processing.

## Persistence

- `raw_transcript.json`: current Raw revision compatibility pointer (FunASR output before legacy cleanup/merge).
- `raw_revisions/raw-*.json`: immutable Raw revision history.
- `canonical_transcript.json`: current derived canonical output.
- `transform_log.json`: deterministic transformation audit trail.
- `pipeline_manifest.json`: canonical pipeline version plus the Raw revision/hash it was derived from.

Raw revisions use a stable content fingerprint that ignores wall-clock `createdAt`. An identical rerun reuses the same revision; a genuinely different ASR result creates a new immutable revision while the previous one remains in `raw_revisions/`.

`AsrMetadata.pipeline_version` is retained only for source compatibility with the current VideoNotes code. New code should not treat it as an ASR property; the authoritative pipeline version is `pipeline_manifest.json` / `CURRENT_PIPELINE_VERSION`.

## Verification note

The module contains unit tests covering ITN/provenance synchronization, internal boundary splitting, strong-boundary preservation, filler/cross-language preservation, domain-agnostic acronym handling, rhetorical repetition preservation and raw-store immutability.

The generation environment used for this package did not contain a Rust toolchain, so `cargo test` could not be executed here. Run your project's normal `cargo test` / `cargo clippy` after copying the module in; the test code is included specifically for that purpose.

## Production integration

- Legacy pre-Raw cleaners (`clean_segment_text`, `consolidate_short_segments`, sentence-case cleanup and related repetition/casing helpers) have been removed rather than left dormant. `Finished` segments are the true FunASR Raw output.
- Selective English CTC/VAD `PauseBoundaryRepair` now carries optional segment-local offsets. `asr.rs` converts those into `BoundaryEvidence`, allowing Canonical Boundary Resolver to split long segments even before full word-level RawToken alignment is available.
- CTC-confirmed boundaries are merge-locked so Stage 2 cannot immediately join them back together.
- Human-readable detector labels (`Chinese`, `English`, `Japanese`, `Korean`) are accepted by `LanguageProfile`.
- Raw and Canonical storage failures are propagated; Canonical is always generated from the exact Raw revision reloaded from disk.
- `transcript.json` is explicitly treated as a derived Standard View. Windows Simplified-Chinese conversion is presentation-only and never modifies `canonical_transcript.json`.
- `sourceAudioHash` is populated with a deterministic sampled media fingerprint. It is an integrity fingerprint, not a cryptographic hash.

### Current deliberate limitation

Production FunASR integration still does **not** invent a full word-level `RawToken` timeline when the backend has not supplied one. Selective CTC boundary evidence is integrated now; full token-level provenance/highlighting can be added later from a real aligner output without changing the Raw/Canonical architecture.

## Pipeline 2.3.0: verified boundary fragments
Tiny cross-language cues at an ASR boundary are now treated as *candidates*, not deletions. The runtime selectively re-transcribes the neighboring audio and only emits a `drop_right` Canonical bridge rewrite when stable left/right lexical anchors both confirm a lexical continuation. This prevents word-specific patches such as `起 -> s` while allowing morphology split at a cue boundary to be recovered from the audio.

## Pipeline 2.6.1: evidence-aware Expanded Nano verification

Selective verification still uses one Expanded Nano re-decode only. `SuspiciousSpan`, `RewriteSpan`,
and `ContextSpan` remain distinct. Cue-level SRT timing and constrained lexical alignment are now
tracked as separate evidence: a single Expanded cue covering the whole context window is **not**
reported as precise time grounding. Text-aligned-only corrections use stricter thresholds and lower
confidence caps.

Entity memory is two-phase within the document. A conservative bootstrap snapshot may only help
select suspicious entity variants; plain uppercase tokens from decoder-wide ALL-CAPS prose are not
learned as entities. After Candidate + Safety Gate decisions, a committed snapshot is rebuilt from
accepted replacements while `UNCERTAIN` suspicious spans are excluded. The committed current-pass
memory is not fed back into the same verification pass, preventing self-reinforcement.

Lexical corrections preserve the First-pass surface for matching lexical tokens; Candidate/Safety Gate
do not decide final casing. Final casing belongs to Typography. Target Nano and SenseVoice remain
outside the path.

## Pipeline 2.6.2: decoder-surface resilience

- Nano runtime capabilities are detected from `--help`. When the runtime exposes `--vad-maxseg`,
  production VAD is capped at the configured safe segment length (15s by default). Expanded Nano and
  degeneration retries use `--chunk <= 15` when that capability is advertised.
- `DecoderSurfaceDegeneration` detects the high-risk combination of a long segment, punctuation
  collapse, and decoder-wide casing/repetition symptoms. A short shouted ALL-CAPS sentence is not
  treated as severe degeneration.
- Severe surface degeneration gets at most one fresh-process safe-window Nano retry. Retry text may
  contribute punctuation/casing evidence only when its normalized lexical units exactly match First;
  lexical disagreement remains `UNCERTAIN` and does not overwrite Raw/Canonical facts.
- `surface_repair` imports punctuation-only retry evidence. If punctuation is still collapsed, strong
  existing acoustic pause evidence may add conservative sentence stops; it never guesses a lexical word.
- Typography owns Standard casing. ALL-CAPS prose and mixed Frankenstein casing are normalized after
  lexical/boundary work, while trusted acronym/name surfaces learned from non-degenerate Canonical text
  are preserved.
- Entity Memory keeps the v2.6.1 two-phase trusted-commit behavior; current-pass uncertain text never
  self-validates through the memory.


## Pipeline 2.7.0: final semantic segmentation

- Standard now has a final conservative English semantic-boundary pass after lexical/entity repair and before final typography.
- The pass repairs high-confidence sentence fragments, joins dangling subordinate/coordinate fragments, and can relocate a false terminal boundary when a linking predicate's complement was pushed into the next segment.
- Boundary changes operate on Canonical tokens, so timing follows the token timeline when English CTC alignment is available; Raw remains immutable.
- The pass is intentionally conservative and language-profile gated. It does not use translation output to rewrite Standard.
- Pipeline version is bumped to 2.7.0 so stale Canonical artifacts are not silently reused.

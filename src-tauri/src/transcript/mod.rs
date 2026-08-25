pub mod model;
pub mod pipeline;
pub mod storage;
pub(crate) mod surface;
pub mod transform;
pub mod views;
pub mod verification;

pub use model::{
    AsrMetadata, CanonicalSegment, CanonicalToken, CanonicalTranscript, LanguageProfile, Provenance,
    RawSegment, RawToken, RawTranscript, TimeSpan, TokenId,
};
pub use pipeline::{
    run_canonical_pipeline, validate_canonical_transcript, BoundaryEvidence, BoundaryEvidenceKind,
    CrossBoundaryRewriteEvidence, PunctuationRepairEvidence, SurfaceRepairEvidence, VerificationRewriteEvidence, PipelineConfig,
};
pub use pipeline::entity::{apply_glossary, GlossaryEntry};
pub use storage::{
    load_canonical_transcript, load_raw_transcript, load_transform_log, pipeline_is_current, pipeline_version, raw_content_hash, raw_revision_id,
    save_canonical_transcript, save_raw_revision, save_raw_transcript, save_raw_transcript_force, CURRENT_PIPELINE_VERSION,
};
pub use transform::{TransformLog, TransformOperation, TransformRecord, TransformStage};
pub use verification::{
    assess_expanded_candidate, build_entity_memory, detect_suspicions,
    extract_local_rewrite_by_alignment, surfaces_equivalent, target_surface, CorrectionKind,
    EntityMemory, ExpandedSafetyAssessment, StableEntity, SuspicionCandidate, SuspicionReason,
    VerificationDecision, VerificationResult, VerificationSegment,
};
pub use views::{render_note_input_view, render_raw_view, render_standard_view, ViewSegment};

#[cfg(test)]
mod tests {
    use super::*;

    fn token(id: u64, text: &str, start: u64, end: u64) -> RawToken {
        RawToken { id, text: text.into(), start_ms: start, end_ms: end, confidence: 0.98 }
    }

    fn raw_with_tokens(language: &str, segments: Vec<RawSegment>) -> RawTranscript {
        RawTranscript {
            job_id: "test-job".into(),
            metadata: AsrMetadata {
                pipeline_version: CURRENT_PIPELINE_VERSION.into(),
                asr_backend: "test-asr".into(),
                asr_model_version: Some("test".into()),
                created_at: "2026-08-20T00:00:00Z".into(),
                source_audio_hash: None,
                raw_revision_id: None,
                raw_content_hash: None,
            },
            language: Some(language.into()),
            segments,
        }
    }

    #[test]
    fn pipeline_keeps_text_and_tokens_in_sync_after_itn() {
        let raw = raw_with_tokens("zh", vec![RawSegment {
            id: "s1".into(), start_ms: 0, end_ms: 1200,
            text: "我们行驶了579点。八公里，达成率百分之八十二点六。".into(),
            tokens: vec![
                token(1,"我们",0,100), token(2,"行驶了",110,240), token(3,"579",250,350),
                token(4,"点",355,400), token(5,"。",405,420), token(6,"八",425,470),
                token(7,"公里",480,600), token(8,"，",605,620), token(9,"达成率",630,760),
                token(10,"百分之",770,850), token(11,"八十二",855,950), token(12,"点",955,980),
                token(13,"六",985,1030), token(14,"。",1040,1060),
            ],
        }]);
        let (canonical, log) = run_canonical_pipeline(&raw, &PipelineConfig::default());
        assert_eq!(canonical.segments.len(), 1);
        assert!(canonical.segments[0].text.contains("579.8"));
        assert!(canonical.segments[0].text.contains("82.6%"));
        assert!(validate_canonical_transcript(&canonical).is_ok());
        let itn_records: Vec<_> = log.records.iter().filter(|r| r.stage == TransformStage::Itn).collect();
        assert!(itn_records.iter().any(|r| r.before_text.contains("579") && !r.source_token_ids.is_empty()));
    }

    #[test]
    fn boundary_splits_inside_raw_segment_on_large_token_gap() {
        let raw = raw_with_tokens("zh", vec![RawSegment {
            id: "long".into(), start_ms: 0, end_ms: 5000, text: "第一句。第二句。".into(),
            tokens: vec![
                token(1,"第一句",0,600), token(2,"。",610,630),
                token(3,"第二句",1900,2500), token(4,"。",2510,2530),
            ],
        }]);
        let (canonical, log) = run_canonical_pipeline(&raw, &PipelineConfig::default());
        assert_eq!(canonical.segments.len(), 2);
        assert!(log.records.iter().any(|r| r.operation == TransformOperation::SplitBoundary));
    }

    #[test]
    fn punctuation_missing_from_alignment_tokens_is_preserved() {
        let raw = raw_with_tokens("zh", vec![RawSegment {
            id: "p".into(), start_ms: 0, end_ms: 1200, text: "第一句。第二句！".into(),
            tokens: vec![token(1, "第一句", 0, 400), token(2, "第二句", 500, 900)],
        }]);
        let (canonical, _) = run_canonical_pipeline(&raw, &PipelineConfig::default());
        let all = canonical.segments.iter().map(|s| s.text.as_str()).collect::<Vec<_>>().join("");
        assert!(all.contains('。'));
        assert!(all.contains('！'));
        assert!(validate_canonical_transcript(&canonical).is_ok());
    }

    #[test]
    fn strong_terminal_is_not_merged_just_because_sentence_is_short() {
        let raw = raw_with_tokens("zh", vec![
            RawSegment { id:"a".into(), start_ms:0, end_ms:300, text:"好的。".into(), tokens:vec![token(1,"好的",0,250), token(2,"。",251,270)] },
            RawSegment { id:"b".into(), start_ms:500, end_ms:900, text:"下一句。".into(), tokens:vec![token(3,"下一句",500,850), token(4,"。",851,880)] },
        ]);
        let (canonical, _) = run_canonical_pipeline(&raw, &PipelineConfig::default());
        assert_eq!(canonical.segments.len(), 2);
        assert_eq!(canonical.segments[0].text, "好的。");
    }

    #[test]
    fn integrity_preserves_real_filler_and_cross_language_text() {
        let raw = raw_with_tokens("en", vec![
            RawSegment { id:"a".into(), start_ms:0, end_ms:200, text:"um".into(), tokens:vec![token(1,"um",0,200)] },
            RawSegment { id:"b".into(), start_ms:1000, end_ms:1400, text:"人工智能".into(), tokens:vec![token(2,"人工智能",1000,1400)] },
        ]);
        let (canonical, _) = run_canonical_pipeline(&raw, &PipelineConfig { is_english_audio: true, preserve_lexical_fidelity: false, boundary_evidence: Vec::new(), punctuation_repairs: Vec::new(), verification_rewrites: Vec::new(), surface_repairs: Vec::new(), bridge_rewrites: Vec::new() });
        assert_eq!(canonical.segments.len(), 2);
        assert!(canonical.segments.iter().any(|s| s.text.contains("人工智能")));
    }

    #[test]
    fn acronym_stitching_is_domain_agnostic() {
        let raw = raw_with_tokens("zh", vec![RawSegment {
            id:"a".into(), start_ms:0, end_ms:700, text:"C O T C".into(),
            tokens:vec![token(1,"C",0,80), token(2,"O",120,200), token(3,"T",240,320), token(4,"C",360,440)],
        }]);
        let (canonical, _) = run_canonical_pipeline(&raw, &PipelineConfig::default());
        assert_eq!(canonical.segments[0].text, "COTC");
        assert_ne!(canonical.segments[0].text, "CLTC");
    }

    #[test]
    fn ordinary_three_word_repetition_is_not_deleted_from_canonical() {
        let raw = raw_with_tokens("en", vec![RawSegment {
            id:"a".into(), start_ms:0, end_ms:800, text:"no no no!".into(),
            tokens:vec![token(1,"no",0,150), token(2,"no",180,330), token(3,"no",360,510), token(4,"!",520,530)],
        }]);
        let (canonical, _) = run_canonical_pipeline(&raw, &PipelineConfig { is_english_audio: true, preserve_lexical_fidelity: false, boundary_evidence: Vec::new(), punctuation_repairs: Vec::new(), verification_rewrites: Vec::new(), surface_repairs: Vec::new(), bridge_rewrites: Vec::new() });
        assert_eq!(canonical.segments[0].text, "no no no!");
    }

    #[test]
    fn raw_storage_is_immutable_by_default() {
        let base = std::env::temp_dir().join(format!("videonotes-transcript-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let raw1 = raw_with_tokens("zh", vec![]);
        save_raw_transcript(&base, &raw1).unwrap();
        save_raw_transcript(&base, &raw1).unwrap();
        let mut raw2 = raw1.clone();
        raw2.job_id = "changed".into();
        assert!(save_raw_transcript(&base, &raw2).is_err());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn language_profile_accepts_human_readable_detector_names() {
        assert_eq!(LanguageProfile::from_language_tag(Some("Chinese")), LanguageProfile::Zh);
        assert_eq!(LanguageProfile::from_language_tag(Some("English")), LanguageProfile::En);
        assert_eq!(LanguageProfile::from_language_tag(Some("Japanese")), LanguageProfile::Ja);
        assert_eq!(LanguageProfile::from_language_tag(Some("Korean")), LanguageProfile::Ko);
    }

    #[test]
    fn raw_storage_ignores_created_at_for_same_revision() {
        let base = std::env::temp_dir().join(format!("videonotes-transcript-created-at-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let raw1 = raw_with_tokens("zh", vec![RawSegment {
            id: "s1".into(), start_ms: 0, end_ms: 500, text: "你好。".into(), tokens: vec![]
        }]);
        let mut raw2 = raw1.clone();
        raw2.metadata.created_at = "later".into();
        save_raw_transcript(&base, &raw1).unwrap();
        save_raw_transcript(&base, &raw2).unwrap();
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn severe_decoder_surface_can_import_punctuation_then_normalize_casing() {
        let raw_text = "HOW'S THE GAME PRETTY GOOD RIGHT I DON'T THINK IT'S FUN";
        let raw = raw_with_tokens("English", vec![RawSegment {
            id: "bad".into(),
            start_ms: 0,
            end_ms: 20_800,
            text: raw_text.into(),
            tokens: vec![],
        }]);
        let config = PipelineConfig {
            is_english_audio: true,
            preserve_lexical_fidelity: false,
            boundary_evidence: Vec::new(),
            punctuation_repairs: Vec::new(),
            verification_rewrites: Vec::new(),
            surface_repairs: vec![SurfaceRepairEvidence {
                target_segment_ids: vec!["bad".into()],
                observed_text: "How's the game? Pretty good, right. I don't think it's fun.".into(),
                confidence: 0.92,
                rule_id: "decoder_surface_retry_punctuation_projection".into(),
            }],
            bridge_rewrites: Vec::new(),
        };
        let (canonical, log) = run_canonical_pipeline(&raw, &config);
        let standard = canonical.segments.iter().map(|segment| segment.text.as_str()).collect::<Vec<_>>().join(" ");
        assert_eq!(standard, "How's the game? Pretty good, right. I don't think it's fun.");
        assert_eq!(raw.segments[0].text, raw_text);
        assert!(log.records.iter().any(|record| record.rule_id == "decoder_surface_retry_punctuation_projection"));
        assert!(log.records.iter().any(|record| record.operation == TransformOperation::NormalizeCasing));
    }

    #[test]
    fn acoustic_pause_does_not_split_canonical_sentence() {
        let raw = raw_with_tokens("English", vec![RawSegment {
            id: "s1".into(),
            start_ms: 0,
            end_ms: 5_000,
            text: "Please have some.".into(),
            tokens: vec![],
        }]);
        let evidence = BoundaryEvidence {
            segment_id: "s1".into(),
            char_offset: "Please ".chars().count(),
            time_ms: 2_000,
            gap_ms: 650,
            confidence: 0.92,
            kind: BoundaryEvidenceKind::AcousticPause,
        };
        let (canonical, _) = run_canonical_pipeline(&raw, &PipelineConfig {
            is_english_audio: true,
            preserve_lexical_fidelity: false,
            boundary_evidence: vec![evidence],
            punctuation_repairs: Vec::new(),
            verification_rewrites: Vec::new(),
            surface_repairs: Vec::new(),
            bridge_rewrites: Vec::new(),
        });
        assert_eq!(canonical.segments.len(), 1);
        assert_eq!(canonical.segments[0].text, "Please have some.");
    }

    #[test]
    fn english_standard_splits_sentences_not_comma_or_pause_fragments() {
        let text = "In a small village, there lived a woman known as Mrs. Stingy. It wasn't her real name. Her clothes were worn out, torn, and covered with patches. Please have some.";
        let raw = raw_with_tokens("English", vec![RawSegment {
            id: "eng".into(), start_ms: 0, end_ms: 30_000, text: text.into(), tokens: vec![],
        }]);
        let pause = BoundaryEvidence {
            segment_id: "eng".into(),
            char_offset: text.find("torn").unwrap(),
            time_ms: 22_000,
            gap_ms: 1_100,
            confidence: 0.95,
            kind: BoundaryEvidenceKind::AcousticPause,
        };
        let (canonical, _) = run_canonical_pipeline(&raw, &PipelineConfig {
            is_english_audio: true,
            preserve_lexical_fidelity: false,
            boundary_evidence: vec![pause],
            punctuation_repairs: Vec::new(),
            verification_rewrites: Vec::new(),
            surface_repairs: Vec::new(),
            bridge_rewrites: Vec::new(),
        });
        assert_eq!(canonical.segments.len(), 4);
        assert!(canonical.segments.iter().any(|s| s.text == "Her clothes were worn out, torn, and covered with patches."));
        assert!(canonical.segments.iter().any(|s| s.text == "Please have some."));
        assert!(!canonical.segments.iter().any(|s| s.text == "torn, and covered with patches."));
    }

    #[test]
    fn strong_punctuation_splits_long_segment_without_tokens() {
        let raw = raw_with_tokens("Chinese", vec![RawSegment {
            id: "s1".into(),
            start_ms: 0,
            end_ms: 30_000,
            text: "第一句很完整。第二句也很完整。第三句继续。".into(),
            tokens: vec![],
        }]);
        let (canonical, log) = run_canonical_pipeline(&raw, &PipelineConfig::default());
        assert_eq!(canonical.segments.len(), 3);
        assert!(log.records.iter().any(|r| r.rule_id == "sentence_split_strong_punctuation_estimated_time"));
    }


    #[test]
    fn authoritative_bridge_rewrite_is_applied_only_in_canonical() {
        let raw = raw_with_tokens("Chinese", vec![
            RawSegment { id: "a".into(), start_ms: 0, end_ms: 1_000, text: "很多上班族".into(), tokens: vec![] },
            RawSegment { id: "b".into(), start_ms: 1_000, end_ms: 2_000, text: "足的一天。".into(), tokens: vec![] },
        ]);
        let config = PipelineConfig {
            is_english_audio: false,
            preserve_lexical_fidelity: false,
            boundary_evidence: Vec::new(),
            punctuation_repairs: Vec::new(),
            verification_rewrites: Vec::new(),
            surface_repairs: Vec::new(),
            bridge_rewrites: vec![CrossBoundaryRewriteEvidence {
                left_segment_id: "a".into(),
                right_segment_id: "b".into(),
                left_text: None,
                right_text: Some("的一天。".into()),
                drop_right: false,
                confidence: 0.96,
            }],
        };
        let (canonical, log) = run_canonical_pipeline(&raw, &config);
        assert_eq!(raw.segments[1].text, "足的一天。");
        assert_eq!(canonical.segments.len(), 1);
        assert_eq!(canonical.segments[0].text, "很多上班族的一天。");
        assert!(log.records.iter().any(|r| r.operation == TransformOperation::CrossBoundaryRewrite));
    }

    #[test]
    fn verified_boundary_fragment_can_repair_left_and_drop_fragment() {
        let raw = raw_with_tokens("English", vec![
            RawSegment { id: "a".into(), start_ms: 0, end_ms: 3_000, text: "She had no family and no friend.".into(), tokens: vec![] },
            RawSegment { id: "b".into(), start_ms: 3_000, end_ms: 3_300, text: "起。".into(), tokens: vec![] },
            RawSegment { id: "c".into(), start_ms: 3_300, end_ms: 6_000, text: "There was only one thing she truly cared about.".into(), tokens: vec![] },
        ]);
        let config = PipelineConfig {
            is_english_audio: true,
            preserve_lexical_fidelity: false,
            boundary_evidence: Vec::new(),
            punctuation_repairs: Vec::new(),
            verification_rewrites: Vec::new(),
            surface_repairs: Vec::new(),
            bridge_rewrites: vec![CrossBoundaryRewriteEvidence {
                left_segment_id: "a".into(),
                right_segment_id: "b".into(),
                left_text: Some("She had no family and no friends.".into()),
                right_text: None,
                drop_right: true,
                confidence: 0.97,
            }],
        };
        let (canonical, log) = run_canonical_pipeline(&raw, &config);
        assert!(canonical.segments.iter().any(|s| s.text == "She had no family and no friends."));
        assert!(!canonical.segments.iter().any(|s| s.text.contains('起')));
        assert_eq!(raw.segments[1].text, "起。");
        assert!(log.records.iter().any(|r| r.operation == TransformOperation::RepairBoundaryFragment));
    }

    #[test]
    fn audio_grounded_verification_rewrite_can_replace_contiguous_raw_span() {
        let raw = raw_with_tokens("English", vec![
            RawSegment { id: "pre".into(), start_ms: 0, end_ms: 1_500, text: "Mrs. Stingy lived all alone.".into(), tokens: vec![] },
            RawSegment { id: "a".into(), start_ms: 1_520, end_ms: 3_000, text: "She had no family and no friend.".into(), tokens: vec![] },
            RawSegment { id: "b".into(), start_ms: 3_020, end_ms: 3_340, text: "起。".into(), tokens: vec![] },
            RawSegment { id: "next".into(), start_ms: 3_360, end_ms: 6_000, text: "There was only one thing she truly cared about.".into(), tokens: vec![] },
        ]);
        let config = PipelineConfig {
            is_english_audio: true,
            preserve_lexical_fidelity: false,
            boundary_evidence: Vec::new(),
            punctuation_repairs: Vec::new(),
            surface_repairs: Vec::new(),
            verification_rewrites: vec![VerificationRewriteEvidence {
                target_segment_ids: vec!["a".into(), "b".into()],
                replacement_text: "She had no family and no friends.".into(),
                confidence: 0.97,
                rule_id: "test_expanded_nano_safety_gate".into(),
            }],
            bridge_rewrites: Vec::new(),
        };
        let (canonical, log) = run_canonical_pipeline(&raw, &config);
        assert!(canonical.segments.iter().any(|s| s.text == "She had no family and no friends."));
        assert!(!canonical.segments.iter().any(|s| s.text.contains('起')));
        assert_eq!(raw.segments[2].text, "起。");
        assert!(log.records.iter().any(|r| r.stage == TransformStage::Verification && r.operation == TransformOperation::VerificationCorrection));
    }

    #[test]
    fn moss_preserves_spoken_numbers_without_itn_mangling() {
        let raw = raw_with_tokens("Chinese", vec![RawSegment {
            id: "moss1".into(),
            start_ms: 0,
            end_ms: 3_000,
            text: "报道时间八月十三日十三点三十六分。".into(),
            tokens: vec![],
        }]);
        let config = PipelineConfig {
            preserve_lexical_fidelity: true,
            ..PipelineConfig::default()
        };
        let (canonical, _) = run_canonical_pipeline(&raw, &config);
        assert_eq!(canonical.segments.len(), 1);
        assert_eq!(canonical.segments[0].text, "报道时间八月十三日十三点三十六分。");
    }

    #[test]
    fn raw_revision_store_preserves_history() {
        let base = std::env::temp_dir().join(format!("videonotes-transcript-revisions-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let mut raw1 = raw_with_tokens("zh", vec![RawSegment {
            id: "s1".into(), start_ms: 0, end_ms: 500, text: "第一版。".into(), tokens: vec![]
        }]);
        let h1 = raw_content_hash(&raw1).unwrap();
        let r1 = raw_revision_id(&raw1).unwrap();
        raw1.metadata.raw_content_hash = Some(h1);
        raw1.metadata.raw_revision_id = Some(r1.clone());
        save_raw_revision(&base, &raw1).unwrap();

        let mut raw2 = raw1.clone();
        raw2.segments[0].text = "第二版。".into();
        raw2.metadata.raw_content_hash = None;
        raw2.metadata.raw_revision_id = None;
        let h2 = raw_content_hash(&raw2).unwrap();
        raw2.metadata.raw_content_hash = Some(h2);
        let r2 = raw_revision_id(&raw2).unwrap();
        raw2.metadata.raw_revision_id = Some(r2.clone());
        save_raw_revision(&base, &raw2).unwrap();

        assert_ne!(r1, r2);
        assert!(base.join("raw_revisions").join(format!("{r1}.json")).is_file());
        assert!(base.join("raw_revisions").join(format!("{r2}.json")).is_file());
        let current = load_raw_transcript(&base).unwrap().unwrap();
        assert_eq!(current.metadata.raw_revision_id.as_deref(), Some(r2.as_str()));
        let _ = std::fs::remove_dir_all(&base);
    }
}

use crate::{
    media::{self, MediaPreparationResult},
    native_manager::{self, NativeInstallEvent, NativeModelRequest},
    transcriber::{self, AsrConfig, StartTranscriptionRequest, TranscriptionEvent},
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tauri::{ipc::Channel, AppHandle, Emitter, Manager};

#[cfg(windows)]
use windows_sys::Win32::Globalization::{
    LCMapStringEx, LCMAP_SIMPLIFIED_CHINESE, LOCALE_NAME_INVARIANT,
};

fn append_verification_debug(path: &Path, stage: &str, message: impl AsRef<str>) {
    use std::io::Write as _;
    let timestamp_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|v| v.as_millis())
        .unwrap_or(0);
    if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "[{timestamp_ms}] [{stage}] {}", message.as_ref());
    }
}

pub const DEFAULT_MODEL_ID: &str = "funasr-nano";
pub const MODEL_NAME: &str = "Fun-ASR-Nano (GGUF + VAD + CTC + 标点)";
pub const MODEL_SIZE_LABEL: &str = "约 1.2 GiB";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AsrModelStatus {
    pub id: String,
    pub name: String,
    /// Primary Nano transcription bundle readiness.
    pub installed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_size: Option<u64>,
    pub size_label: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelDownloadProgress {
    pub model_id: String,
    pub downloaded_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_bytes: Option<u64>,
    pub progress: u8,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AsrPhaseProgress {
    pub job_id: String,
    pub phase: String,
    pub completed: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    pub unit: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AsrPhaseEvent {
    pub job_id: String,
    pub phase: String,
    pub state: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AsrSnapshot {
    pub job_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    pub segments: Vec<TranscriptSegment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    pub processed_until: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pause_repairs: Option<Vec<transcriber::PauseBoundaryRepair>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provisional: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptSegment {
    pub id: String,
    #[serde(default)]
    pub chunk_index: usize,
    #[serde(default)]
    pub start: f64,
    #[serde(default)]
    pub end: f64,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub translated_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avg_confidence: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptResult {
    pub job_id: String,
    pub model_id: String,
    pub language: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub translation_language: Option<String>,
    pub text: String,
    pub segments: Vec<TranscriptSegment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pause_repairs: Option<Vec<transcriber::PauseBoundaryRepair>>,
}

pub fn models_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("models")
}

pub fn model_status(app: &AppHandle, app_data_dir: &Path) -> AsrModelStatus {
    let req = NativeModelRequest {
        model_kind: "nano".to_string(),
    };
    match native_manager::status(app, &req) {
        Ok(status) => {
            let model_path = PathBuf::from(&status.paths.model_path);
            let size = fs::metadata(&model_path).map(|m| m.len()).ok();
            AsrModelStatus {
                id: DEFAULT_MODEL_ID.to_string(),
                name: MODEL_NAME.to_string(),
                installed: status.ready,
                file_size: size,
                size_label: MODEL_SIZE_LABEL.to_string(),
                path: status.paths.install_root,
            }
        }
        Err(_) => AsrModelStatus {
            id: DEFAULT_MODEL_ID.to_string(),
            name: MODEL_NAME.to_string(),
            installed: false,
            file_size: None,
            size_label: MODEL_SIZE_LABEL.to_string(),
            path: models_dir(app_data_dir)
                .join(DEFAULT_MODEL_ID)
                .to_string_lossy()
                .into_owned(),
        },
    }
}

pub fn emit_download_progress(
    app: &AppHandle,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
    progress: u8,
    message: impl Into<String>,
) {
    let payload = ModelDownloadProgress {
        model_id: DEFAULT_MODEL_ID.to_string(),
        downloaded_bytes,
        total_bytes,
        progress: progress.min(100),
        message: message.into(),
    };
    let _ = app.emit("model-download-progress", payload);
}

pub fn emit_asr_phase(
    app: &AppHandle,
    job_id: &str,
    phase: impl Into<String>,
    state: impl Into<String>,
    message: impl Into<String>,
) {
    let payload = AsrPhaseEvent {
        job_id: job_id.to_string(),
        phase: phase.into(),
        state: state.into(),
        message: message.into(),
    };
    let _ = app.emit("asr-phase", payload);
}

pub fn emit_asr_phase_progress(
    app: &AppHandle,
    job_id: &str,
    phase: impl Into<String>,
    completed: u64,
    total: Option<u64>,
    unit: impl Into<String>,
    message: impl Into<String>,
) {
    let payload = AsrPhaseProgress {
        job_id: job_id.to_string(),
        phase: phase.into(),
        completed,
        total,
        unit: unit.into(),
        message: message.into(),
    };
    let _ = app.emit("asr-phase-progress", payload);
}

pub async fn download_default_model(app: &AppHandle) -> Result<AsrModelStatus, String> {
    let req = NativeModelRequest {
        model_kind: "nano".to_string(),
    };
    let app_handle = app.clone();
    let channel_app = app.clone();
    let channel = Channel::new(move |body: tauri::ipc::InvokeResponseBody| {
        if let tauri::ipc::InvokeResponseBody::Json(json_str) = body {
            if let Ok(event) = serde_json::from_str::<NativeInstallEvent>(&json_str) {
                match event {
                    NativeInstallEvent::Started { total_items } => {
                        emit_download_progress(
                            &channel_app,
                            0,
                            None,
                            0,
                            format!("开始安装 Fun-ASR-Nano（共 {total_items} 个组件）..."),
                        );
                    }
                    NativeInstallEvent::ItemStarted { label, index, total_items, .. } => {
                        let progress = ((index * 100) / total_items.max(1)) as u8;
                        emit_download_progress(
                            &channel_app,
                            0,
                            None,
                            progress,
                            format!("[{index}/{total_items}] 正在下载 {label}..."),
                        );
                    }
                    NativeInstallEvent::Progress {
                        label,
                        current_file,
                        downloaded_bytes,
                        total_bytes,
                        index,
                        total_items,
                    } => {
                        let file_pct = if total_bytes > 0 {
                            (downloaded_bytes as f64 / total_bytes as f64).clamp(0.0, 1.0)
                        } else {
                            0.0
                        };
                        let overall_pct = (((index as f64 + file_pct) / total_items.max(1) as f64) * 100.0) as u8;
                        emit_download_progress(
                            &channel_app,
                            downloaded_bytes,
                            (total_bytes > 0).then_some(total_bytes),
                            overall_pct,
                            format!("[{index}/{total_items}] {label} · {current_file}"),
                        );
                    }
                    NativeInstallEvent::ItemFinished { label, index, total_items, .. } => {
                        let progress = (((index + 1) * 100) / total_items.max(1)) as u8;
                        emit_download_progress(
                            &channel_app,
                            0,
                            None,
                            progress,
                            format!("[{index}/{total_items}] {label} 下载完成"),
                        );
                    }
                    NativeInstallEvent::Finished { .. } => {
                        emit_download_progress(&channel_app, 0, None, 100, "Fun-ASR-Nano 安装完成");
                    }
                    NativeInstallEvent::Error { message } => {
                        emit_download_progress(&channel_app, 0, None, 0, format!("安装出错：{message}"));
                    }
                }
            }
        }
        Ok(())
    });

    native_manager::install(app_handle.clone(), req, channel).await?;
    let app_data_dir = app_handle
        .path()
        .app_data_dir()
        .unwrap_or_else(|_| PathBuf::from("data"));
    Ok(model_status(&app_handle, &app_data_dir))
}

pub fn delete_default_model(app: &AppHandle) -> Result<(), String> {
    let req = NativeModelRequest {
        model_kind: "nano".to_string(),
    };
    if let Ok(status) = native_manager::status(app, &req) {
        let install_root = PathBuf::from(&status.paths.install_root);
        if install_root.is_dir() {
            fs::remove_dir_all(&install_root)
                .map_err(|e| format!("无法删除模型目录（{}）：{e}", install_root.display()))?;
        }
    }
    Ok(())
}

pub fn load_transcript(task_data_dir: &Path, job_id: &str) -> Result<TranscriptResult, String> {
    media::validate_job_id(job_id)?;
    let task_dir = task_data_dir.join("tasks").join(job_id);
    let path = task_dir.join("transcript").join("transcript.json");
    if path.is_file() {
        let mut file = File::open(&path)
            .map_err(|error| format!("无法读取转录结果（{}）：{error}", path.display()))?;
        let mut content = String::new();
        file.read_to_string(&mut content)
            .map_err(|error| format!("无法解析转录内容：{error}"))?;
        let mut transcript: TranscriptResult =
            serde_json::from_str(&content).map_err(|error| format!("转录格式无效：{error}"))?;
        if transcript.job_id.is_empty() {
            transcript.job_id = job_id.to_string();
        }
        return Ok(transcript);
    }

    let checkpoint_path = task_dir.join("moss_checkpoint.json");
    if checkpoint_path.is_file() {
        if let Ok(content) = fs::read_to_string(&checkpoint_path) {
            if let Ok(data) = serde_json::from_str::<crate::transcriber::MossCheckpointData>(&content) {
                let raw_segments = match data {
                    crate::transcriber::MossCheckpointData::Structured { segments, .. } => segments,
                    crate::transcriber::MossCheckpointData::Legacy(segments) => segments,
                };
                let segments = raw_segments
                    .into_iter()
                    .enumerate()
                    .map(|(idx, s)| TranscriptSegment {
                        id: s.id,
                        chunk_index: idx,
                        start: s.start,
                        end: s.end,
                        start_ms: (s.start * 1000.0).round() as u64,
                        end_ms: (s.end * 1000.0).round() as u64,
                        text: s.text,
                        translated_text: None,
                        avg_confidence: None,
                    })
                    .collect::<Vec<_>>();
                let text = segments.iter().map(|s| s.text.as_str()).collect::<Vec<_>>().join("\n");
                return Ok(TranscriptResult {
                    job_id: job_id.to_string(),
                    model_id: "openasr-moss-q4".to_string(),
                    language: "zh".to_string(),
                    translation_language: None,
                    text,
                    segments,
                    pause_repairs: None,
                });
            }
        }
    }

    let raw_path = task_dir.join("transcript").join("raw_transcript.json");
    if raw_path.is_file() {
        if let Ok(content) = fs::read_to_string(&raw_path) {
            if let Ok(raw) = serde_json::from_str::<crate::transcript::model::RawTranscript>(&content) {
                let segments = raw
                    .segments
                    .into_iter()
                    .enumerate()
                    .map(|(idx, s)| TranscriptSegment {
                        id: if s.id.is_empty() { format!("raw-{idx}") } else { s.id },
                        chunk_index: idx,
                        start: s.start_ms as f64 / 1000.0,
                        end: s.end_ms as f64 / 1000.0,
                        start_ms: s.start_ms,
                        end_ms: s.end_ms,
                        text: s.text,
                        translated_text: None,
                        avg_confidence: None,
                    })
                    .collect::<Vec<_>>();
                let text = segments.iter().map(|s| s.text.as_str()).collect::<Vec<_>>().join("\n");
                let lang = raw.language.unwrap_or_else(|| "zh".to_string());
                return Ok(TranscriptResult {
                    job_id: job_id.to_string(),
                    model_id: raw.metadata.asr_backend,
                    language: lang,
                    translation_language: None,
                    text,
                    segments,
                    pause_repairs: None,
                });
            }
        }
    }

    Ok(TranscriptResult {
        job_id: job_id.to_string(),
        model_id: DEFAULT_MODEL_ID.to_string(),
        language: "zh".to_string(),
        translation_language: None,
        text: String::new(),
        segments: Vec::new(),
        pause_repairs: None,
    })
}

pub fn save_transcript(
    task_data_dir: &Path,
    job_id: &str,
    transcript: &TranscriptResult,
) -> Result<(), String> {
    media::validate_job_id(job_id)?;
    let dir = task_data_dir.join("tasks").join(job_id).join("transcript");
    fs::create_dir_all(&dir).map_err(|error| format!("无法创建转录目录：{error}"))?;
    let json_path = dir.join("transcript.json");
    let serialized = serde_json::to_string_pretty(transcript)
        .map_err(|error| format!("转录结果序列化失败：{error}"))?;
    fs::write(&json_path, serialized)
        .map_err(|error| format!("无法保存转录 JSON：{error}"))?;
    let txt_path = dir.join("transcript.txt");
    fs::write(&txt_path, &transcript.text)
        .map_err(|error| format!("无法保存转录文本：{error}"))?;
    Ok(())
}

pub async fn transcribe_job(
    app: &AppHandle,
    _model_data_dir: &Path,
    task_data_dir: &Path,
    job_id: &str,
    backend: &str,
    asr_config_json: Option<&str>,
    resume: bool,
    cancelled: Arc<AtomicBool>,
) -> Result<TranscriptResult, String> {
    media::validate_job_id(job_id)?;

    let is_moss = backend == crate::openasr::MODEL_ID || backend == "openasr-moss-q4";
    let expected_model_id = if is_moss { crate::openasr::MODEL_ID } else { DEFAULT_MODEL_ID };
    let native_status_opt = native_manager::status(app, &NativeModelRequest { model_kind: "nano".to_string() }).ok();
    let native_status = if is_moss {
        None
    } else {
        let status = native_status_opt.as_ref().ok_or_else(|| {
            format!("MODEL_NOT_INSTALLED:请先到“模型”页面下载 {MODEL_NAME}")
        })?;
        if !status.ready {
            return Err(format!(
                "MODEL_NOT_INSTALLED:请先到“模型”页面下载 {MODEL_NAME}（缺少{}）",
                status.missing.join("、")
            ));
        }
        Some(status.clone())
    };
    if is_moss {
        let _ = crate::openasr::model_is_ready(app)?;
    }
    let fallback_paths = native_status_opt.map(|s| s.paths);
    let task_dir = task_data_dir.join("tasks").join(job_id);
    let transcript_dir = task_dir.join("transcript");
    let checkpoint_path = task_dir.join("moss_checkpoint.json");

    let initial_segments = if resume && is_moss && checkpoint_path.is_file() {
        fs::read_to_string(&checkpoint_path)
            .ok()
            .and_then(|raw| {
                serde_json::from_str::<crate::transcriber::MossCheckpointData>(&raw).ok().map(|data| match data {
                    crate::transcriber::MossCheckpointData::Structured { segments, .. } => segments,
                    crate::transcriber::MossCheckpointData::Legacy(segments) => segments,
                })
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    let _result_path = transcript_dir.join("transcript.json");

    if initial_segments.is_empty() {
        if checkpoint_path.is_file() {
            let _ = fs::remove_file(&checkpoint_path);
        }
        if transcript_dir.is_dir() {
            fs::remove_dir_all(&transcript_dir)
                .map_err(|error| format!("无法替换旧的转录结果：{error}"))?;
        }
    }
    fs::create_dir_all(&transcript_dir)
        .map_err(|e| format!("无法创建转录目录：{e}"))?;
    // Translation and notes are derived from the Standard transcript.  A real
    // re-run invalidates them immediately so downstream actions cannot combine
    // artifacts from the previous ASR backend with the new result.
    for derived_dir in [task_dir.join("translation"), task_dir.join("note")] {
        if derived_dir.is_dir() {
            fs::remove_dir_all(&derived_dir)
                .map_err(|error| format!("无法清理旧的派生结果：{error}"))?;
        }
    }
    let verification_debug_log_path = transcript_dir.join("log.txt");
    let _ = fs::write(
        &verification_debug_log_path,
        format!(
            "VideoNotes selective verification diagnostic\npipeline={} (v2.7.0 semantic-boundary + stable-id translation)\njob={}\nmodel={}\n\n",
            crate::transcript::storage::CURRENT_PIPELINE_VERSION, job_id, expected_model_id,
        ),
    );
    append_verification_debug(&verification_debug_log_path, "ASR-BEGIN", "starting fresh diagnostic transcription run");

    let manifest_raw = fs::read_to_string(task_dir.join("media.json"))
        .map_err(|_| "尚未找到已准备的媒体文件，请先重新准备媒体".to_string())?;
    let media: MediaPreparationResult = serde_json::from_str(&manifest_raw)
        .map_err(|error| format!("媒体清单格式无效：{error}"))?;

    let audio_path = if task_dir.join("media.wav").is_file() {
        task_dir.join("media.wav")
    } else if let Some(video) = media.video_file.as_ref() {
        PathBuf::from(video)
    } else {
        PathBuf::from(&media.source_file)
    };

    let media_tools = media::resolve_media_tools(app)?;
    let sys = sysinfo::System::new_all();
    let thread_count = (sys.cpus().len()).clamp(1, 8);

    let moss_config = asr_config_json
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .unwrap_or_else(|| serde_json::json!({"chunkSeconds": 30.0, "overlapSeconds": 1.0}));
    let config = AsrConfig {
        backend: if is_moss { "openasr-moss-q4".to_string() } else { "funasr-nano".to_string() },
        funasr_mode: "nano".to_string(),
        funasr_runtime_path: native_status.as_ref().map(|s| s.paths.runtime_path.clone()).unwrap_or_default(),
        funasr_model_path: native_status.as_ref().map(|s| s.paths.model_path.clone()).unwrap_or_default(),
        funasr_encoder_model_path: native_status.as_ref().map(|s| s.paths.encoder_model_path.clone()).unwrap_or_default(),
        funasr_vad_model_path: native_status.as_ref().map(|s| s.paths.vad_model_path.clone()).unwrap_or_default(),
        punctuation_runtime_path: native_status
            .as_ref()
            .map(|s| s.paths.punctuation_runtime_path.clone())
            .or_else(|| fallback_paths.as_ref().map(|p| p.punctuation_runtime_path.clone()))
            .unwrap_or_default(),
        punctuation_model_path: native_status
            .as_ref()
            .map(|s| s.paths.punctuation_model_path.clone())
            .or_else(|| fallback_paths.as_ref().map(|p| p.punctuation_model_path.clone()))
            .unwrap_or_default(),
        alignment_model_path: native_status.as_ref().map(|s| s.paths.alignment_model_path.clone()).unwrap_or_default(),
        alignment_tokens_path: native_status.as_ref().map(|s| s.paths.alignment_tokens_path.clone()).unwrap_or_default(),
        openasr_runtime_path: if is_moss { crate::openasr::model_is_ready(app)?.0.to_string_lossy().into_owned() } else { String::new() },
        openasr_model_path: if is_moss { crate::openasr::model_is_ready(app)?.1.to_string_lossy().into_owned() } else { String::new() },
        moss_chunk_seconds: moss_config.get("chunkSeconds").and_then(|v| v.as_f64()).unwrap_or(30.0),
        moss_overlap_seconds: moss_config.get("overlapSeconds").and_then(|v| v.as_f64()).unwrap_or(1.0),
        verification_debug_log_path: verification_debug_log_path.to_string_lossy().into_owned(),
        funasr_chunk_seconds: 15.0,
        ffmpeg_path: media_tools.ffmpeg.to_string_lossy().into_owned(),
        ffprobe_path: media_tools.ffprobe.to_string_lossy().into_owned(),
        threads: thread_count,
    };

    append_verification_debug(
        &verification_debug_log_path,
        "MODEL-STATUS",
        format!("backend={} modelReady={} verifier=expanded-nano+surface-retry", backend, native_status.as_ref().map(|s| s.ready).unwrap_or(is_moss)),
    );

    // Keep an immutable copy for the post-ASR alignment layer. The transcriber owns its request.
    let alignment_config = config.clone();
    let request = StartTranscriptionRequest {
        video_path: audio_path.to_string_lossy().into_owned(),
        media_duration: Some(media.duration_seconds),
        config,
        initial_segments,
        checkpoint_file: is_moss.then(|| checkpoint_path.to_string_lossy().into_owned()),
    };

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<TranscriptionEvent>();
    let channel = Channel::new(move |body: tauri::ipc::InvokeResponseBody| {
        if let tauri::ipc::InvokeResponseBody::Json(json_str) = body {
            if let Ok(event) = serde_json::from_str::<TranscriptionEvent>(&json_str) {
                let _ = tx.send(event);
            }
        }
        Ok(())
    });

    let app_event = app.clone();
    let job_id_event = job_id.to_string();

    let collector_task = tokio::spawn(async move {
        let mut final_segments = Vec::new();
        let mut lang = None;
        let mut final_repairs = None;
        let mut final_bridge_repairs = Vec::new();
        let mut final_verification_results = Vec::new();

        while let Some(event) = rx.recv().await {
            match event {
                TranscriptionEvent::Started { duration: _ } => {}
                TranscriptionEvent::PhaseStarted { phase, message } => {
                    emit_asr_phase(&app_event, &job_id_event, phase, "started", message);
                }
                TranscriptionEvent::PhaseCompleted { phase, message } => {
                    emit_asr_phase(&app_event, &job_id_event, phase, "completed", message);
                }
                TranscriptionEvent::PhaseProgress { phase, completed, total, unit, message } => {
                    emit_asr_phase_progress(
                        &app_event,
                        &job_id_event,
                        phase,
                        completed,
                        total,
                        unit,
                        message,
                    );
                }
                TranscriptionEvent::Snapshot { segments, language, processed_until } => {
                    // Snapshots are deliberately Raw-only. Standard is a derived view and must
                    // not be shown until the complete Raw revision has passed the canonical
                    // pipeline and has been persisted successfully.
                    let raw_snapshot_segments = segments
                        .iter()
                        .enumerate()
                        .map(|(idx, s)| TranscriptSegment {
                            id: s.id.clone(),
                            chunk_index: idx,
                            start: s.start,
                            end: s.end,
                            start_ms: (s.start * 1000.0).round() as u64,
                            end_ms: (s.end * 1000.0).round() as u64,
                            text: s.text.clone(),
                            translated_text: None,
                            avg_confidence: None,
                        })
                        .collect();
                    let raw_payload = AsrSnapshot {
                        job_id: job_id_event.clone(),
                        model_id: Some(expected_model_id.to_string()),
                        segments: raw_snapshot_segments,
                        language: language.clone(),
                        processed_until,
                        pause_repairs: None,
                        view: Some("raw".into()),
                        provisional: Some(true),
                    };
                    let _ = app_event.emit("asr-snapshot", raw_payload);
                }
                TranscriptionEvent::PauseRepairUpdate { .. } => {
                    // Pause repairs are persisted with Finished and must not make
                    // the Standard view appear before canonicalization completes.
                }
                TranscriptionEvent::Finished {
                    segments,
                    language,
                    pause_repairs,
                    bridge_repairs,
                    verification_results,
                } => {
                    final_segments = segments
                        .into_iter()
                        .enumerate()
                        .map(|(idx, s)| TranscriptSegment {
                            id: s.id,
                            chunk_index: idx,
                            start: s.start,
                            end: s.end,
                            start_ms: (s.start * 1000.0).round() as u64,
                            end_ms: (s.end * 1000.0).round() as u64,
                            text: s.text,
                            translated_text: None,
                            avg_confidence: None,
                        })
                        .collect();
                    lang = language;
                    final_repairs = Some(pause_repairs);
                    final_bridge_repairs = bridge_repairs;
                    final_verification_results = verification_results;
                }
                _ => {}
            }
        }
        (final_segments, lang, final_repairs, final_bridge_repairs, final_verification_results)
    });

    transcriber::run(request, channel, &cancelled).await?;
    let (result_segments, detected_language, pause_repairs, bridge_repairs, verification_results) = collector_task
        .await
        .map_err(|e| format!("转写事件收集异常：{e}"))?;

    if cancelled.load(Ordering::Relaxed) {
        return Err("任务已取消".to_string());
    }

    if result_segments.is_empty() {
        return Err("语音识别未产生任何有效文本".to_string());
    }

    // `result_segments` are the immutable backend Raw output. No legacy text cleanup or
    // segment consolidation has run before this point.
    let raw_text = result_segments
        .iter()
        .map(|s| s.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    let language = detected_language.unwrap_or_else(|| {
        if transcriber::is_chinese_text(&raw_text) {
            "zh".to_string()
        } else {
            "en".to_string()
        }
    });
    let is_english = is_english_language(&language);

    let mut raw_segments: Vec<crate::transcript::model::RawSegment> = result_segments
        .iter()
        .map(|s| crate::transcript::model::RawSegment {
            id: s.id.clone(),
            start_ms: s.start_ms,
            end_ms: s.end_ms,
            text: s.text.clone(),
            // Current production integration has selective CTC boundary evidence, not a
            // complete word-level aligned timeline yet. Never invent RawToken ids here.
            tokens: vec![],
        })
        .collect();

    // English gets a complete CTC word timeline. This is intentionally separate from
    // PauseBoundaryRepair: punctuation decides *whether* a Canonical sentence ends, while
    // CTC supplies the exact clock for that textual candidate.
    if !is_moss && is_english
        && Path::new(&alignment_config.punctuation_runtime_path).is_file()
        && Path::new(&alignment_config.alignment_model_path).is_file()
        && Path::new(&alignment_config.alignment_tokens_path).is_file()
    {
        emit_asr_phase(
            app,
            job_id,
            "word_alignment",
            "started",
            "正在建立 English CTC word timeline",
        );
        let alignment_audio_path = audio_path.to_string_lossy().into_owned();
        let completion_message = match crate::pause_alignment::build_english_alignment_timeline(
            &alignment_config.ffmpeg_path,
            &alignment_audio_path,
            &raw_segments,
            Path::new(&alignment_config.punctuation_runtime_path),
            Path::new(&alignment_config.alignment_model_path),
            Path::new(&alignment_config.alignment_tokens_path),
            alignment_config.threads,
        ).await {
            Ok(timelines) => {
                for (segment_id, tokens) in timelines {
                    if tokens.is_empty() { continue; }
                    if let Some(segment) = raw_segments.iter_mut().find(|s| s.id == segment_id) {
                        segment.tokens = tokens;
                    }
                }
                "English CTC word timeline 处理完成".to_string()
            }
            Err(error) => format!("English CTC word timeline 跳过：{error}"),
        };
        emit_asr_phase(
            app,
            job_id,
            "word_alignment",
            "completed",
            completion_message,
        );
    }

    emit_asr_phase(
        app,
        job_id,
        "standardization",
        "started",
        "正在从原始时间轴生成校正（标准）结果",
    );

    let created_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| format!("{}", d.as_secs()))
        .unwrap_or_else(|_| "0".to_string());

    let source_audio_hash = Some(source_file_fingerprint(&audio_path)?);
    let mut raw_candidate = crate::transcript::model::RawTranscript {
        job_id: job_id.to_string(),
        metadata: crate::transcript::model::AsrMetadata {
            // Legacy compatibility only; authoritative pipeline version lives in pipeline_manifest.json.
            pipeline_version: crate::transcript::storage::CURRENT_PIPELINE_VERSION.to_string(),
            asr_backend: if is_moss { "openasr".to_string() } else { "funasr".to_string() },
            asr_model_version: Some(expected_model_id.to_string()),
            created_at,
            source_audio_hash,
            raw_revision_id: None,
            raw_content_hash: None,
        },
        language: Some(language.clone()),
        segments: raw_segments,
    };

    // Revision identity is derived from Raw content and ignores createdAt/pipelineVersion.
    let raw_hash = crate::transcript::storage::raw_content_hash(&raw_candidate)?;
    let revision_id = crate::transcript::storage::raw_revision_id(&raw_candidate)?;
    raw_candidate.metadata.raw_content_hash = Some(raw_hash);
    raw_candidate.metadata.raw_revision_id = Some(revision_id);

    crate::transcript::storage::save_raw_revision(&transcript_dir, &raw_candidate)?;

    // Always derive Canonical from the exact Raw revision persisted on disk. This prevents
    // a same-job rerun from producing Canonical against different in-memory Raw metadata.
    let raw_transcript = crate::transcript::storage::load_raw_transcript(&transcript_dir)?
        .ok_or_else(|| "保存 Raw ASR 后无法重新读取 raw_transcript.json".to_string())?;

    // Publish the authoritative persisted Raw timeline once more. Streaming snapshots can miss
    // a final parser-flush segment; this guarantees the "原始" view exactly matches Raw storage.
    let final_raw_segments: Vec<TranscriptSegment> = crate::transcript::views::render_raw_view(&raw_transcript)
        .into_iter()
        .enumerate()
        .map(|(idx, s)| TranscriptSegment {
            id: s.id,
            chunk_index: idx,
            start: s.start_ms as f64 / 1000.0,
            end: s.end_ms as f64 / 1000.0,
            start_ms: s.start_ms,
            end_ms: s.end_ms,
            text: s.text,
            translated_text: s.translated_text,
            avg_confidence: None,
        })
        .collect();
    let final_raw_processed_until = final_raw_segments
        .last()
        .map(|segment| segment.end)
        .unwrap_or(0.0);
    let _ = app.emit(
        "asr-snapshot",
        AsrSnapshot {
            job_id: job_id.to_string(),
            model_id: Some(expected_model_id.to_string()),
            segments: final_raw_segments,
            language: Some(language.clone()),
            processed_until: final_raw_processed_until,
            pause_repairs: None,
            view: Some("raw".into()),
            provisional: Some(false),
        },
    );

    let boundary_evidence = pause_repairs
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .filter_map(|repair| {
            Some(crate::transcript::pipeline::BoundaryEvidence {
                segment_id: repair.segment_id.clone()?,
                char_offset: repair.segment_char_offset?,
                time_ms: (repair.time.max(0.0) * 1000.0).round() as u64,
                gap_ms: (repair.gap.max(0.0) * 1000.0).round() as u64,
                confidence: repair.confidence.clamp(0.0, 1.0) as f32,
                kind: crate::transcript::pipeline::BoundaryEvidenceKind::AcousticPause,
            })
        })
        .collect::<Vec<_>>();

    let punctuation_repairs = pause_repairs
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .filter_map(|repair| {
            // Be conservative: a long pause alone is not enough to rewrite punctuation. Only
            // semantic punctuation-model corroboration plus a concrete punctuation to relocate
            // may modify Canonical surface text.
            if !repair.punctuation_relocation_supported || repair.remove_punctuation_offset.is_none() {
                return None;
            }
            Some(crate::transcript::pipeline::PunctuationRepairEvidence {
                segment_id: repair.segment_id.clone()?,
                char_offset: repair.segment_char_offset?,
                remove_segment_id: repair.remove_segment_id.clone(),
                remove_char_offset: repair.remove_segment_char_offset,
                time_ms: (repair.time.max(0.0) * 1000.0).round() as u64,
                confidence: repair.confidence.clamp(0.0, 1.0) as f32,
            })
        })
        .collect::<Vec<_>>();

    // Persist every verifier outcome, including VERIFIED and UNCERTAIN. Only CORRECTED entries
    // become Canonical rewrite evidence. This keeps review/debugging separate from mutation.
    let verification_json = serde_json::to_string_pretty(&verification_results)
        .map_err(|e| format!("Verification Log 序列化失败：{e}"))?;
    fs::write(transcript_dir.join("verification_log.json"), verification_json)
        .map_err(|e| format!("无法保存 verification_log.json：{e}"))?;
    append_verification_debug(
        &verification_debug_log_path, "VERIFICATION-PERSIST",
        format!("verificationResults={} saved=verification_log.json", verification_results.len()),
    );
    for (index, result) in verification_results.iter().enumerate() {
        append_verification_debug(
            &verification_debug_log_path, "VERIFICATION-RESULT",
            format!(
                "index={} suspiciousIds={:?} targetIds={:?} decision={:?} kind={:?} score={:.3} confidence={:.3} coverage={:.3} timeGrounded={} textAligned={} leftSim={:.3} rightSim={:.3} editRatio={:.3} replacementRatio={:.3} first={:?} expandedNano={:?} expandedTarget={:?} replacement={:?} safetyReasons={:?} context={:?}",
                index, result.suspicious_segment_ids, result.target_segment_ids, result.decision, result.correction_kind,
                result.suspicion_score, result.confidence, result.target_time_coverage, result.time_grounded, result.text_aligned, result.left_context_similarity,
                result.right_context_similarity, result.edit_ratio, result.replacement_ratio, result.first_pass_text,
                result.expanded_nano_text, result.expanded_target_text, result.replacement_text, result.safety_reasons, result.context
            ),
        );
    }

    let verification_rewrites = verification_results
        .iter()
        .filter(|r| matches!(r.decision, crate::transcript::verification::VerificationDecision::Corrected))
        .filter_map(|r| {
            Some(crate::transcript::pipeline::VerificationRewriteEvidence {
                target_segment_ids: r.target_segment_ids.clone(),
                replacement_text: r.replacement_text.clone()?,
                confidence: r.confidence,
                rule_id: "v2_6_2_expanded_nano_safety_gate".into(),
            })
        })
        .collect::<Vec<_>>();
    append_verification_debug(
        &verification_debug_log_path, "REWRITE-EVIDENCE",
        format!("verificationRewrites={}", verification_rewrites.len()),
    );

    let surface_repairs = verification_results
        .iter()
        .filter(|result| matches!(result.decision, crate::transcript::verification::VerificationDecision::Verified))
        .filter(|result| result.reasons.iter().any(|reason| {
            matches!(reason, crate::transcript::verification::SuspicionReason::DecoderSurfaceDegeneration)
        }))
        .filter_map(|result| {
            Some(crate::transcript::pipeline::SurfaceRepairEvidence {
                target_segment_ids: result.target_segment_ids.clone(),
                observed_text: result.expanded_target_text.clone()?,
                confidence: result.confidence,
                rule_id: "decoder_surface_retry_punctuation_projection".into(),
            })
        })
        .collect::<Vec<_>>();
    append_verification_debug(
        &verification_debug_log_path,
        "SURFACE-REPAIR-EVIDENCE",
        format!("surfaceRepairs={}", surface_repairs.len()),
    );

    let bridge_rewrites = bridge_repairs
        .into_iter()
        .map(|repair| crate::transcript::pipeline::CrossBoundaryRewriteEvidence {
            left_segment_id: repair.left_segment_id,
            right_segment_id: repair.right_segment_id,
            left_text: repair.left_text,
            right_text: repair.right_text,
            drop_right: repair.drop_right,
            confidence: repair.confidence.clamp(0.0, 1.0) as f32,
        })
        .collect::<Vec<_>>();

    let pipeline_config = crate::transcript::pipeline::PipelineConfig {
        is_english_audio: is_english,
        preserve_lexical_fidelity: is_moss,
        boundary_evidence,
        punctuation_repairs,
        verification_rewrites,
        surface_repairs,
        bridge_rewrites,
    };

    emit_asr_phase(
        app,
        job_id,
        "semantic_segmentation",
        "started",
        "正在进行最终语义分段与边界复核",
    );
    let (canonical_transcript, transform_log) =
        crate::transcript::pipeline::run_canonical_pipeline(&raw_transcript, &pipeline_config);
    emit_asr_phase(
        app,
        job_id,
        "semantic_segmentation",
        "completed",
        "最终语义分段与边界复核完成",
    );
    let verification_transform_records = transform_log.records.iter()
        .filter(|record| matches!(record.stage, crate::transcript::transform::TransformStage::Verification))
        .collect::<Vec<_>>();
    append_verification_debug(
        &verification_debug_log_path, "CANONICAL-VERIFY",
        format!(
            "rewriteEvidence={} appliedVerificationTransforms={} canonicalSegments={}",
            pipeline_config.verification_rewrites.len(), verification_transform_records.len(), canonical_transcript.segments.len()
        ),
    );
    for (index, record) in verification_transform_records.iter().enumerate() {
        append_verification_debug(
            &verification_debug_log_path, "CANONICAL-REWRITE",
            format!("index={} before={:?} after={:?} rule={} confidence={:.3}", index, record.before_text, record.after_text, record.rule_id, record.confidence),
        );
    }
    if pipeline_config.verification_rewrites.len() != verification_transform_records.len() {
        append_verification_debug(
            &verification_debug_log_path, "CANONICAL-WARNING",
            "verification rewrite evidence count differs from applied Verification transform count; inspect target segment ids/contiguity",
        );
    }

    if let Err(errors) = crate::transcript::pipeline::validate_canonical_transcript(&canonical_transcript) {
        append_verification_debug(&verification_debug_log_path, "CANONICAL-ERROR", format!("validation failed: {}", errors.join("; ")));
        return Err(format!("Canonical transcript invariant failed: {}", errors.join("; ")));
    }
    append_verification_debug(&verification_debug_log_path, "CANONICAL-OK", "canonical transcript validation passed");

    crate::transcript::storage::save_canonical_transcript(
        &transcript_dir,
        &canonical_transcript,
        Some(&transform_log),
    )?;

    // `transcript.json` remains a derived Standard View for compatibility with the existing UI.
    // Windows Simplified-Chinese mapping is presentation-only and never writes back to Canonical.
    let standard_view = crate::transcript::views::render_standard_view(&canonical_transcript);
    let canonical_result_segments: Vec<TranscriptSegment> = standard_view
        .into_iter()
        .enumerate()
        .map(|(idx, s)| TranscriptSegment {
            id: s.id,
            chunk_index: idx,
            start: s.start_ms as f64 / 1000.0,
            end: s.end_ms as f64 / 1000.0,
            start_ms: s.start_ms,
            end_ms: s.end_ms,
            text: s.text,
            translated_text: s.translated_text,
            avg_confidence: None,
        })
        .collect();

    let mut result = TranscriptResult {
        job_id: job_id.to_string(),
        model_id: expected_model_id.to_string(),
        language,
        translation_language: None,
        text: String::new(),
        segments: canonical_result_segments,
        pause_repairs,
    };

    if is_chinese_language(&result.language) {
        apply_standard_view_script_preference(&mut result);
    }
    result.text = result
        .segments
        .iter()
        .map(|s| s.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    save_transcript(task_data_dir, job_id, &result)?;

    // Publish the Standard view only after the complete canonical result has been persisted.
    let final_processed_until = result
        .segments
        .last()
        .map(|segment| segment.end)
        .unwrap_or(0.0);
    let final_snapshot = AsrSnapshot {
        job_id: job_id.to_string(),
        model_id: Some(expected_model_id.to_string()),
        segments: result.segments.clone(),
        language: Some(result.language.clone()),
        processed_until: final_processed_until,
        pause_repairs: result.pause_repairs.clone(),
        view: Some("standard".into()),
        provisional: Some(false),
    };
    let _ = app.emit("asr-snapshot", final_snapshot);
    emit_asr_phase(
        app,
        job_id,
        "standardization",
        "completed",
        "校正（标准）时间轴已定稿",
    );
    append_verification_debug(
        &verification_debug_log_path, "ASR-END",
        format!("finalSegments={} finalTextChars={} status=success", result.segments.len(), result.text.chars().count()),
    );
    Ok(result)
}

fn is_chinese_language(language: &str) -> bool {
    let normalized = language.trim().to_ascii_lowercase();
    normalized == "zh"
        || normalized.starts_with("zh-")
        || normalized.contains("chinese")
        || normalized == "cmn"
        || normalized == "yue"
}

fn is_english_language(language: &str) -> bool {
    let normalized = language.trim().to_ascii_lowercase();
    normalized == "en" || normalized.starts_with("en-") || normalized == "english"
}


/// Fast deterministic media fingerprint used only to bind a Raw ASR revision to the
/// prepared input file. It samples the beginning/middle/end and file length using
/// FNV-1a; it is not intended as a cryptographic security hash.
fn source_file_fingerprint(path: &Path) -> Result<String, String> {
    const SAMPLE: usize = 64 * 1024;
    let mut file = File::open(path).map_err(|e| format!("无法打开音频用于指纹计算：{e}"))?;
    let len = file.metadata().map_err(|e| format!("无法读取音频元数据：{e}"))?.len();
    let mut hash = 0xcbf29ce484222325u64;
    fn update(hash: &mut u64, data: &[u8]) {
        for &b in data {
            *hash ^= b as u64;
            *hash = (*hash).wrapping_mul(0x100000001b3);
        }
    }
    update(&mut hash, &len.to_le_bytes());
    let mut buf = vec![0u8; SAMPLE];
    for offset in [0u64, len.saturating_sub(SAMPLE as u64) / 2, len.saturating_sub(SAMPLE as u64)] {
        file.seek(SeekFrom::Start(offset)).map_err(|e| format!("定位音频指纹采样失败：{e}"))?;
        let n = file.read(&mut buf).map_err(|e| format!("读取音频指纹采样失败：{e}"))?;
        update(&mut hash, &offset.to_le_bytes());
        update(&mut hash, &buf[..n]);
    }
    Ok(format!("media-fnv1a64:{hash:016x}"))
}

#[cfg(windows)]
pub fn simplify_chinese_text(value: &str) -> String {
    let source = value.encode_utf16().collect::<Vec<_>>();
    if source.is_empty() {
        return String::new();
    }
    let required = unsafe {
        LCMapStringEx(
            LOCALE_NAME_INVARIANT,
            LCMAP_SIMPLIFIED_CHINESE,
            source.as_ptr(),
            source.len() as i32,
            std::ptr::null_mut(),
            0,
            std::ptr::null(),
            std::ptr::null(),
            0,
        )
    };
    if required <= 0 {
        return value.to_string();
    }
    let mut buffer = vec![0u16; required as usize];
    let written = unsafe {
        LCMapStringEx(
            LOCALE_NAME_INVARIANT,
            LCMAP_SIMPLIFIED_CHINESE,
            source.as_ptr(),
            source.len() as i32,
            buffer.as_mut_ptr(),
            buffer.len() as i32,
            std::ptr::null(),
            std::ptr::null(),
            0,
        )
    };
    if written <= 0 {
        return value.to_string();
    }
    String::from_utf16_lossy(&buffer[..written as usize])
}

#[cfg(not(windows))]
pub fn simplify_chinese_text(value: &str) -> String {
    value.to_string()
}

fn apply_standard_view_script_preference(transcript: &mut TranscriptResult) {
    transcript.text = simplify_chinese_text(&transcript.text);
    for segment in &mut transcript.segments {
        segment.text = simplify_chinese_text(&segment.text);
    }
}

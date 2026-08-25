mod asr;
mod chunk_stitcher;
mod ctc_alignment_ffi;
pub mod error;
mod media;
mod native_manager;
mod openasr;
mod pause_alignment;
mod punctuation_ffi;
mod search;
mod summary;
pub mod transcript;
mod transcriber;
mod translation;
mod workflow;

pub use error::AppError;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs, io,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;

struct AppState {
    database: Arc<Mutex<Connection>>,
    app_data_dir: PathBuf,
    task_data_dir: Arc<Mutex<PathBuf>>,
    model_download_active: AtomicBool,
    workflow: Arc<workflow::WorkflowState>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DataDirectorySettings {
    current_path: String,
    default_path: String,
    is_default: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredSettings {
    task_data_directory: String,
}

fn settings_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("settings.json")
}

fn load_task_data_directory(app_data_dir: &Path) -> PathBuf {
    let path = settings_path(app_data_dir);
    let configured = fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str::<StoredSettings>(&content).ok())
        .map(|settings| PathBuf::from(settings.task_data_directory));
    let directory = configured.unwrap_or_else(|| app_data_dir.to_path_buf());
    if fs::create_dir_all(directory.join("tasks")).is_ok() {
        directory
    } else {
        let _ = fs::create_dir_all(app_data_dir.join("tasks"));
        app_data_dir.to_path_buf()
    }
}

fn save_task_data_directory(app_data_dir: &Path, directory: &Path) -> Result<(), String> {
    let settings = StoredSettings {
        task_data_directory: directory.to_string_lossy().into_owned(),
    };
    let content = serde_json::to_vec_pretty(&settings)
        .map_err(|error| format!("无法保存目录设置：{error}"))?;
    fs::write(settings_path(app_data_dir), content)
        .map_err(|error| format!("无法保存目录设置：{error}"))
}

fn current_task_data_directory(state: &State<'_, AppState>) -> Result<PathBuf, String> {
    state
        .task_data_dir
        .lock()
        .map(|directory| directory.clone())
        .map_err(|_| "数据目录当前不可用".to_string())
}

fn data_directory_settings(state: &State<'_, AppState>) -> Result<DataDirectorySettings, String> {
    let current = current_task_data_directory(state)?;
    Ok(DataDirectorySettings {
        current_path: current.to_string_lossy().into_owned(),
        default_path: state.app_data_dir.to_string_lossy().into_owned(),
        is_default: current == state.app_data_dir,
    })
}

fn copy_directory_contents(source: &Path, destination: &Path) -> Result<(), String> {
    if !source.is_dir() {
        return Ok(());
    }
    fs::create_dir_all(destination).map_err(|error| format!("无法创建新数据目录：{error}"))?;
    for entry in fs::read_dir(source).map_err(|error| format!("无法读取原数据目录：{error}"))?
    {
        let entry = entry.map_err(|error| format!("无法读取原数据目录：{error}"))?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if source_path.is_dir() {
            copy_directory_contents(&source_path, &destination_path)?;
        } else {
            fs::copy(&source_path, &destination_path)
                .map_err(|error| format!("复制任务数据失败：{error}"))?;
        }
    }
    Ok(())
}

fn rewrite_local_paths(value: &mut Value, old_task: &Path, new_task: &Path) {
    match value {
        Value::String(path) => {
            let candidate = PathBuf::from(path.as_str());
            if let Ok(relative) = candidate.strip_prefix(old_task) {
                *path = new_task.join(relative).to_string_lossy().into_owned();
            }
        }
        Value::Array(values) => {
            for value in values {
                rewrite_local_paths(value, old_task, new_task);
            }
        }
        Value::Object(values) => {
            for value in values.values_mut() {
                rewrite_local_paths(value, old_task, new_task);
            }
        }
        _ => {}
    }
}

fn rewrite_copied_media_manifests(old_root: &Path, new_root: &Path) -> Result<(), String> {
    let old_tasks = old_root.join("tasks");
    let new_tasks = new_root.join("tasks");
    if !new_tasks.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(&new_tasks).map_err(|error| format!("无法检查新数据目录：{error}"))?
    {
        let entry = entry.map_err(|error| format!("无法检查新数据目录：{error}"))?;
        if !entry.path().is_dir() {
            continue;
        }
        let job_id = entry.file_name();
        let old_task = old_tasks.join(&job_id);
        let new_task = new_tasks.join(&job_id);
        let manifest_path = new_task.join("media.json");
        if !manifest_path.is_file() {
            continue;
        }
        let content = fs::read_to_string(&manifest_path)
            .map_err(|error| format!("无法读取媒体清单：{error}"))?;
        let mut manifest: Value =
            serde_json::from_str(&content).map_err(|error| format!("无法解析媒体清单：{error}"))?;
        rewrite_local_paths(&mut manifest, &old_task, &new_task);
        fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest)
                .map_err(|error| format!("无法更新媒体清单：{error}"))?,
        )
        .map_err(|error| format!("无法更新媒体清单：{error}"))?;
    }
    Ok(())
}

fn change_task_data_directory(
    state: &State<'_, AppState>,
    requested_directory: PathBuf,
) -> Result<DataDirectorySettings, String> {
    if state.workflow.has_active() {
        return Err("有任务正在处理，请等待完成或取消任务后再更改目录".to_string());
    }

    fs::create_dir_all(&requested_directory)
        .map_err(|error| format!("无法使用所选目录：{error}"))?;
    let new_root = requested_directory
        .canonicalize()
        .map_err(|error| format!("无法读取所选目录：{error}"))?;
    let old_root = current_task_data_directory(state)?;
    let old_tasks = old_root.join("tasks");
    let new_tasks = new_root.join("tasks");
    if new_tasks.starts_with(&old_tasks) && new_tasks != old_tasks {
        return Err("不能把新数据目录放在当前任务目录内部".to_string());
    }
    if new_root == old_root {
        return data_directory_settings(state);
    }

    copy_directory_contents(&old_tasks, &new_tasks)?;
    rewrite_copied_media_manifests(&old_root, &new_root)?;
    save_task_data_directory(&state.app_data_dir, &new_root)?;
    *state
        .task_data_dir
        .lock()
        .map_err(|_| "数据目录当前不可用".to_string())? = new_root;
    data_directory_settings(state)
}

#[tauri::command]
fn inspect_data_directory(state: State<'_, AppState>) -> Result<DataDirectorySettings, String> {
    data_directory_settings(&state)
}

#[tauri::command]
async fn choose_data_directory(
    app: AppHandle,
    window: tauri::WebviewWindow,
) -> Result<Option<DataDirectorySettings>, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_parent(&window)
        .set_title("选择视频与解析数据目录")
        .pick_folder(move |path| {
            let _ = tx.send(path);
        });
    let selected = rx.await.map_err(|_| "文件夹选择已取消或异常中断".to_string())?;
    let Some(selected) = selected else {
        return Ok(None);
    };
    let directory = selected
        .into_path()
        .map_err(|_| "所选位置不是有效的本地目录".to_string())?;
    let app_handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app_handle.state::<AppState>();
        change_task_data_directory(&state, directory).map(Some)
    })
    .await
    .map_err(|error| format!("更改目录任务异常退出：{error}"))?
}

#[tauri::command]
async fn reset_data_directory(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<DataDirectorySettings, String> {
    let default_dir = state.app_data_dir.clone();
    let app_handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app_handle.state::<AppState>();
        change_task_data_directory(&state, default_dir)
    })
    .await
    .map_err(|error| format!("重置目录任务异常退出：{error}"))?
}

#[tauri::command]
async fn export_video_audio(
    video_id: String,
    suggested_filename: String,
    app: AppHandle,
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    media::validate_job_id(&video_id)?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_parent(&window)
        .set_title("导出音频")
        .set_file_name(suggested_filename)
        .add_filter("M4A 音频", &["m4a"])
        .save_file(move |path| {
            let _ = tx.send(path);
        });
    let selected = rx.await.map_err(|_| "文件保存已取消或异常中断".to_string())?;
    let Some(selected) = selected else {
        return Ok(None);
    };
    let mut output_path = selected
        .into_path()
        .map_err(|_| "所选位置不是可写入的本地文件".to_string())?;
    if output_path.extension().and_then(|value| value.to_str()) != Some("m4a") {
        output_path.set_extension("m4a");
    }
    let tools = media::resolve_media_tools(&app)?;
    let task_data_dir = current_task_data_directory(&state)?;
    let worker_app = app.clone();
    let worker_video_id = video_id.clone();
    let worker_output_path = output_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        media::export_audio(
            &worker_app,
            &tools,
            &task_data_dir,
            &worker_video_id,
            &worker_output_path,
        )
    })
    .await
    .map_err(|error| format!("音频导出任务异常退出：{error}"))??;
    Ok(Some(output_path.to_string_lossy().into_owned()))
}

#[tauri::command]
fn inspect_media_tools(app: AppHandle) -> media::MediaToolsStatus {
    media::inspect_media_tools(&app)
}

#[tauri::command]
async fn parse_video_input(input: String, app: AppHandle) -> Result<media::SourcePreview, String> {
    let tools = media::resolve_media_tools(&app)?;
    let app_handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || media::probe_source(&app_handle, &tools, &input))
        .await
        .map_err(|error| format!("媒体解析任务异常退出：{error}"))?
}

#[tauri::command]
async fn search_videos(
    keyword: String,
    order: Option<String>,
    duration: Option<usize>,
    page: Option<usize>,
    app: AppHandle,
) -> Result<search::SearchResultResponse, String> {
    search::search_videos(&app, keyword, order, duration, page).await
}

#[tauri::command]
fn load_video_media(
    video_id: String,
    state: State<'_, AppState>,
) -> Result<media::MediaPreparationResult, AppError> {
    Ok(media::load_media(&current_task_data_directory(&state)?, &video_id)?)
}

#[tauri::command]
fn inspect_asr_model(app: AppHandle, state: State<'_, AppState>) -> asr::AsrModelStatus {
    asr::model_status(&app, &state.app_data_dir)
}

#[tauri::command]
async fn download_asr_model(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<asr::AsrModelStatus, AppError> {
    if state.model_download_active.swap(true, Ordering::Relaxed) {
        return Err(AppError::new("ALREADY_DOWNLOADING", "语音模型已经在下载中"));
    }
    let worker_app = app.clone();
    let result = asr::download_default_model(&worker_app).await.map_err(AppError::from);
    state.model_download_active.store(false, Ordering::Relaxed);
    result
}

#[tauri::command]
fn delete_asr_model(app: AppHandle, state: State<'_, AppState>) -> Result<(), AppError> {
    if state.model_download_active.load(Ordering::Relaxed) {
        return Err(AppError::new("ALREADY_DOWNLOADING", "模型正在下载，暂时无法删除"));
    }
    Ok(asr::delete_default_model(&app)?)
}

#[tauri::command]
fn inspect_moss_model(app: AppHandle) -> openasr::OpenAsrModelStatus {
    openasr::model_status(&app)
}

#[tauri::command]
async fn download_moss_model(app: AppHandle, state: State<'_, AppState>) -> Result<openasr::OpenAsrModelStatus, AppError> {
    if state.model_download_active.swap(true, Ordering::Relaxed) {
        return Err(AppError::new("ALREADY_DOWNLOADING", "已有模型正在下载中"));
    }
    let result = openasr::download_model(&app).await.map_err(AppError::from);
    state.model_download_active.store(false, Ordering::Relaxed);
    result
}

#[tauri::command]
fn delete_moss_model(app: AppHandle, state: State<'_, AppState>) -> Result<(), AppError> {
    if state.model_download_active.load(Ordering::Relaxed) {
        return Err(AppError::new("ALREADY_DOWNLOADING", "模型正在下载，暂时无法删除"));
    }
    Ok(openasr::delete_model(&app)?)
}

#[tauri::command]
fn inspect_summary_model(state: State<'_, AppState>) -> summary::SummaryModelStatus {
    summary::model_status(&state.app_data_dir)
}

#[tauri::command]
async fn download_summary_model(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<summary::SummaryModelStatus, AppError> {
    if state.model_download_active.swap(true, Ordering::Relaxed) {
        return Err(AppError::new("ALREADY_DOWNLOADING", "已有模型正在下载中"));
    }
    let app_data_dir = state.app_data_dir.clone();
    let worker_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        summary::download_default_model(&worker_app, &app_data_dir)
    })
    .await;
    state.model_download_active.store(false, Ordering::Relaxed);
    Ok(result.map_err(|error| AppError::failed(format!("模型下载任务异常退出：{error}")))??)
}

#[tauri::command]
fn delete_summary_model(state: State<'_, AppState>) -> Result<(), AppError> {
    if state.model_download_active.load(Ordering::Relaxed) {
        return Err(AppError::new("ALREADY_DOWNLOADING", "模型正在下载，暂时无法删除"));
    }
    Ok(summary::delete_default_model(&state.app_data_dir)?)
}

#[tauri::command]
fn inspect_translation_model(state: State<'_, AppState>) -> translation::TranslationModelStatus {
    translation::model_status(&state.app_data_dir)
}

#[tauri::command]
async fn download_translation_model(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<translation::TranslationModelStatus, AppError> {
    if state.model_download_active.swap(true, Ordering::Relaxed) {
        return Err(AppError::new("ALREADY_DOWNLOADING", "已有模型正在下载中"));
    }
    let app_data_dir = state.app_data_dir.clone();
    let worker_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        translation::download_default_model(&worker_app, &app_data_dir)
    })
    .await;
    state.model_download_active.store(false, Ordering::Relaxed);
    Ok(result.map_err(|error| AppError::failed(format!("模型下载任务异常退出：{error}")))??)
}

#[tauri::command]
fn delete_translation_model(state: State<'_, AppState>) -> Result<(), AppError> {
    if state.model_download_active.load(Ordering::Relaxed) {
        return Err(AppError::new("ALREADY_DOWNLOADING", "模型正在下载，暂时无法删除"));
    }
    Ok(translation::remove_default_model(&state.app_data_dir).map(|_| ())?)
}

#[tauri::command]
fn open_models_directory(state: State<'_, AppState>) -> Result<(), AppError> {
    let directory = asr::models_dir(&state.app_data_dir);
    fs::create_dir_all(&directory).map_err(|error| format!("无法创建模型目录：{error}"))?;
    #[cfg(windows)]
    {
        Command::new("explorer.exe")
            .arg(&directory)
            .spawn()
            .map_err(|error| format!("无法打开模型目录：{error}"))?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = directory;
        Err("当前系统暂不支持自动打开模型目录".to_string())
    }
}

#[tauri::command]
fn load_video_transcript(
    video_id: String,
    state: State<'_, AppState>,
) -> Result<asr::TranscriptResult, AppError> {
    Ok(asr::load_transcript(&current_task_data_directory(&state)?, &video_id)?)
}

/// 修改某条转录段文本并落盘(transcript.json + transcript.txt 同步更新)。
#[tauri::command]
fn update_video_transcript_segment(
    video_id: String,
    segment_id: String,
    text: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let task_data_dir = current_task_data_directory(&state)?;
    let mut transcript = asr::load_transcript(&task_data_dir, &video_id)?;
    let Some(segment) = transcript
        .segments
        .iter_mut()
        .find(|segment| segment.id == segment_id)
    else {
        return Err("找不到该转录段".to_string());
    };
    segment.text = text;
    // Translation is derived from Standard text. Any human edit invalidates the old
    // translation for this segment instead of silently keeping stale Chinese text.
    segment.translated_text = None;
    if transcript.segments.iter().all(|segment| segment.translated_text.is_none()) {
        transcript.translation_language = None;
    }
    transcript.text = transcript
        .segments
        .iter()
        .map(|segment| segment.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    asr::save_transcript(&task_data_dir, &video_id, &transcript)
        .map_err(|error| format!("无法保存转录修改：{error}"))?;

    // Keep old derived files for recovery/export, but make their dependency
    // state explicit. They must never be silently presented as current.
    let db = state.database.lock().map_err(|_| "数据库当前不可用".to_string())?;
    workflow::mark_derived_stale(&db, &video_id)?;
    Ok(())
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn organize_video_notes(
    video_id: String,
    title: String,
    source_url: String,
    platform: String,
    duration: String,
    force: bool,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<summary::NoteResult, AppError> {
    let cancelled = state.workflow.enqueue_cancel(&video_id).map_err(AppError::failed)?;
    let workflow = state.workflow.clone();
    let model_data_dir = state.app_data_dir.clone();
    let task_data_dir = current_task_data_directory(&state)?;
    let worker_task_data_dir = task_data_dir.clone();
    let worker_app = app.clone();
    let worker_video_id = video_id.clone();
    let worker_result = tauri::async_runtime::spawn_blocking(move || {
        let _permit = workflow.acquire_heavy(&worker_video_id, &cancelled).map_err(|e| e.to_string())?;
        summary::organize_job(
            &worker_app,
            &model_data_dir,
            &worker_task_data_dir,
            &worker_video_id,
            &title,
            &source_url,
            &platform,
            &duration,
            force,
            cancelled,
        )
    })
    .await;
    state.workflow.release_cancel(&video_id);
    let note = worker_result.map_err(|error| AppError::failed(format!("内容整理任务异常退出：{error}")))??;
    let db = state.database.lock().map_err(|_| AppError::failed("数据库当前不可用"))?;
    workflow::mark_artifact_ready(&db, &video_id, "note", "note/note.json").map_err(AppError::failed)?;
    Ok(note)
}

/// 用户主动触发的翻译任务：仅把非中文标准转录翻译为简体中文并保存，不做笔记整理。
#[tauri::command]
async fn translate_video_transcript(
    video_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let cancelled = state.workflow.enqueue_cancel(&video_id).map_err(AppError::failed)?;
    let workflow = state.workflow.clone();
    let model_data_dir = state.app_data_dir.clone();
    let task_data_dir = current_task_data_directory(&state)?;
    let worker_task_data_dir = task_data_dir.clone();
    let worker_app = app.clone();
    let worker_video_id = video_id.clone();
    let worker_result = tauri::async_runtime::spawn_blocking(move || {
        let _permit = workflow.acquire_heavy(&worker_video_id, &cancelled).map_err(|e| e.to_string())?;
        summary::translate_job(
            &worker_app,
            &model_data_dir,
            &worker_task_data_dir,
            &worker_video_id,
            cancelled,
        )
    })
    .await;
    state.workflow.release_cancel(&video_id);
    worker_result.map_err(|error| AppError::failed(format!("翻译任务异常退出：{error}")))??;
    workflow::write_translation_artifact(&task_data_dir, &video_id).map_err(AppError::failed)?;
    let db = state.database.lock().map_err(|_| AppError::failed("数据库当前不可用"))?;
    workflow::mark_artifact_ready(&db, &video_id, "translation", "translation/translation.json").map_err(AppError::failed)?;
    Ok(())
}

#[tauri::command]
fn load_video_note(video_id: String, state: State<'_, AppState>) -> Result<summary::NoteResult, AppError> {
    Ok(summary::load_note(&current_task_data_directory(&state)?, &video_id)?)
}

#[tauri::command]
async fn export_markdown(
    suggested_filename: String,
    markdown: String,
    app: AppHandle,
    window: tauri::WebviewWindow,
) -> Result<Option<String>, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_parent(&window)
        .set_title("导出 Markdown")
        .set_file_name(suggested_filename)
        .add_filter("Markdown", &["md"])
        .save_file(move |path| {
            let _ = tx.send(path);
        });
    let selected_file = rx.await.map_err(|_| "文件保存已取消或异常中断".to_string())?;
    let Some(selected_file) = selected_file else {
        return Ok(None);
    };
    let path = selected_file
        .into_path()
        .map_err(|_| "所选位置不是可写入的本地文件".to_string())?;
    fs::write(&path, markdown.as_bytes()).map_err(|error| format!("无法保存 Markdown：{error}"))?;

    Ok(Some(path.to_string_lossy().into_owned()))
}

// --- Persistent workflow commands -----------------------------------------------------------
// These are the only commands that create or mutate queue/library state.  The
// media and ASR functions below are workers used by WorkflowState, not public
// front-end orchestration entry points.

#[tauri::command]
fn list_videos(state: State<'_, AppState>) -> Result<Vec<workflow::VideoRecord>, String> {
    let db = state.database.lock().map_err(|_| "数据库当前不可用".to_string())?;
    let root = current_task_data_directory(&state)?;
    workflow::list_videos(&db, &root).map_err(|e| e.to_string())
}

#[tauri::command]
fn list_queue_items(state: State<'_, AppState>) -> Result<Vec<workflow::QueueItem>, String> {
    let db = state.database.lock().map_err(|_| "数据库当前不可用".to_string())?;
    workflow::list_queue(&db).map_err(|e| e.to_string())
}

#[tauri::command]
fn enqueue_sources(inputs: Vec<workflow::EnqueueInput>, state: State<'_, AppState>) -> Result<Vec<workflow::EnqueueOutcome>, String> {
    let out = {
        let db = state.database.lock().map_err(|_| "数据库当前不可用".to_string())?;
        workflow::enqueue(&db, inputs).map_err(|e| e.to_string())?
    };
    state.workflow.start_scheduler();
    state.workflow.emit("queue-updated");
    Ok(out)
}

#[tauri::command]
async fn pause_queue_item(id: String, state: State<'_, AppState>) -> Result<(), String> {
    state.workflow.cancel(&id);
    for _ in 0..50 {
        if !state.workflow.is_active(&id) { break; }
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }
    if state.workflow.is_active(&id) {
        return Err("任务仍在停止中，请稍后重试".into());
    }
    let db = state.database.lock().map_err(|_| "数据库当前不可用".to_string())?;
    workflow::queue_command(&db, &id, "pause").map_err(|e| e.to_string())
}

#[tauri::command]
fn resume_queue_item(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let db = state.database.lock().map_err(|_| "数据库当前不可用".to_string())?;
    workflow::queue_command(&db, &id, "resume").map_err(|e| e.to_string())?;
    drop(db);
    state.workflow.start_scheduler();
    Ok(())
}

#[tauri::command]
fn retry_queue_item(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let root = current_task_data_directory(&state)?;
    let db = state.database.lock().map_err(|_| "数据库当前不可用".to_string())?;
    workflow::retry_queue(&db, &root, &id)?;
    drop(db);
    state.workflow.start_scheduler();
    Ok(())
}

#[tauri::command]
fn requeue_video(video_id: String, asr_backend: Option<String>, asr_config_json: Option<String>, state: State<'_, AppState>) -> Result<workflow::QueueItem, String> {
    media::validate_job_id(&video_id)?;
    let root = current_task_data_directory(&state)?;
    let db = state.database.lock().map_err(|_| "数据库当前不可用".to_string())?;
    let item = workflow::requeue_video(&db, &root, &video_id, asr_backend.as_deref(), asr_config_json.as_deref())?;
    drop(db);
    state.workflow.start_scheduler();
    state.workflow.emit("queue-updated");
    Ok(item)
}

#[tauri::command]
async fn remove_queue_item(id: String, state: State<'_, AppState>) -> Result<(), String> {
    if state.workflow.cancel(&id) {
        for _ in 0..50 {
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            if !state.workflow.is_active(&id) { break; }
        }
    }
    if state.workflow.is_active(&id) {
        return Err("任务仍在停止中，请稍后重试".into());
    }
    let db = state.database.lock().map_err(|_| "数据库当前不可用".to_string())?;
    workflow::queue_command(&db, &id, "remove").map_err(|e| e.to_string())
}

#[tauri::command]
fn move_queue_item(id: String, direction: String, state: State<'_, AppState>) -> Result<(), String> {
    let db = state.database.lock().map_err(|_| "数据库当前不可用".to_string())?;
    workflow::move_queue(&db, &id, &direction).map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_video_results(video_id: String, state: State<'_, AppState>) -> Result<(), String> {
    media::validate_job_id(&video_id)?;
    if state.workflow.is_active(&video_id) { return Err("该视频正在处理中，请先等待操作停止".into()); }
    let root = current_task_data_directory(&state)?;
    let mut db = state.database.lock().map_err(|_| "数据库当前不可用".to_string())?;
    if workflow::has_live_queue(&db, &video_id).map_err(|error| error.to_string())? {
        return Err("该视频仍在队列中，请先移除队列项".into());
    }
    workflow::delete_results(&mut db, &root, &video_id)?;
    state.workflow.emit("library-updated");
    Ok(())
}

#[tauri::command]
fn delete_video_completely(video_id: String, state: State<'_, AppState>) -> Result<(), String> {
    media::validate_job_id(&video_id)?;
    if state.workflow.is_active(&video_id) { return Err("该视频正在处理中，请先等待操作停止".into()); }
    let root = current_task_data_directory(&state)?;
    let mut db = state.database.lock().map_err(|_| "数据库当前不可用".to_string())?;
    if workflow::has_live_queue(&db, &video_id).map_err(|error| error.to_string())? {
        return Err("该视频仍在队列中，请先移除队列项".into());
    }
    workflow::delete_completely(&mut db, &root, &video_id)?;
    state.workflow.emit("queue-updated");
    state.workflow.emit("library-updated");
    Ok(())
}

#[tauri::command]
fn update_translation_segment(video_id: String, segment_id: String, text: String, state: State<'_, AppState>) -> Result<(), String> {
    let root = current_task_data_directory(&state)?;
    workflow::mark_translation(&root, &video_id, &segment_id, &text)?;
    let db = state.database.lock().map_err(|_| "数据库当前不可用".to_string())?;
    workflow::mark_note_stale(&db, &video_id)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
#[allow(deprecated)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let app_data_dir = app.path().app_data_dir()?;
            fs::create_dir_all(&app_data_dir)?;
            let task_data_dir = load_task_data_directory(&app_data_dir);
            let database_path = app_data_dir.join("video-notes.db");
            let connection = Connection::open(database_path)
                .map_err(|error| io::Error::other(error.to_string()))?;
            workflow::initialize_database(&connection, &task_data_dir)
                .map_err(|error| io::Error::other(error.to_string()))?;
            let database = Arc::new(Mutex::new(connection));
            let task_data_dir = Arc::new(Mutex::new(task_data_dir));
            let workflow = Arc::new(workflow::WorkflowState::new(database.clone(), task_data_dir.clone()));
            workflow.set_app(app.handle().clone());
            app.manage(AppState {
                database,
                app_data_dir,
                task_data_dir,
                model_download_active: AtomicBool::new(false),
                workflow,
            });
            // Recover queued work after the database/filesystem migration. The
            // scheduler itself verifies reusable media and transcript files.
            app.state::<AppState>().workflow.start_scheduler();
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_videos,
            list_queue_items,
            enqueue_sources,
            pause_queue_item,
            resume_queue_item,
            retry_queue_item,
            requeue_video,
            remove_queue_item,
            move_queue_item,
            delete_video_results,
            delete_video_completely,
            update_translation_segment,
            inspect_data_directory,
            choose_data_directory,
            reset_data_directory,
            export_video_audio,
            parse_video_input,
            search_videos,
            inspect_media_tools,
            load_video_media,
            inspect_asr_model,
            download_asr_model,
            delete_asr_model,
            inspect_moss_model,
            download_moss_model,
            delete_moss_model,
            inspect_summary_model,
            download_summary_model,
            delete_summary_model,
            inspect_translation_model,
            download_translation_model,
            delete_translation_model,
            open_models_directory,
            load_video_transcript,
            update_video_transcript_segment,
            organize_video_notes,
            translate_video_transcript,
            load_video_note,
            export_markdown
        ])
        .run(tauri::generate_context!())
        .expect("failed to run VideoNotes");
}

#[cfg(test)]
mod tests {
    use super::{copy_directory_contents, rewrite_local_paths};
    use serde_json::json;
    use std::{
        fs,
        path::PathBuf,
        process,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn rewrites_media_manifest_paths_for_new_task_directory() {
        let old_task = PathBuf::from(r"C:\old\tasks\job-1");
        let new_task = PathBuf::from(r"D:\VideoNotes\tasks\job-1");
        let mut manifest = json!({
            "videoFile": old_task.join("source").join("video.mp4").to_string_lossy(),
            "chunks": [{
                "path": old_task.join("chunks").join("chunk-000.wav").to_string_lossy()
            }],
            "sourceUrl": "https://example.com/video"
        });

        rewrite_local_paths(&mut manifest, &old_task, &new_task);

        assert_eq!(
            manifest["videoFile"],
            new_task
                .join("source")
                .join("video.mp4")
                .to_string_lossy()
                .as_ref()
        );
        assert_eq!(
            manifest["chunks"][0]["path"],
            new_task
                .join("chunks")
                .join("chunk-000.wav")
                .to_string_lossy()
                .as_ref()
        );
        assert_eq!(manifest["sourceUrl"], "https://example.com/video");
    }

    #[test]
    fn directory_copy_refreshes_existing_task_files() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("video-notes-copy-{}-{unique}", process::id()));
        let source = root.join("source");
        let destination = root.join("destination");
        fs::create_dir_all(&source).expect("create source");
        fs::create_dir_all(&destination).expect("create destination");
        fs::write(source.join("transcript.txt"), "new").expect("write source");
        fs::write(destination.join("transcript.txt"), "old").expect("write destination");

        copy_directory_contents(&source, &destination).expect("copy directory");

        assert_eq!(
            fs::read_to_string(destination.join("transcript.txt")).unwrap(),
            "new"
        );
        fs::remove_dir_all(root).expect("remove test directory");
    }

}

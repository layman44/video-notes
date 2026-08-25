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

pub use error::AppError;

use rusqlite::{params, Connection};
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
use sysinfo::System;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;

struct AppState {
    database: Mutex<Connection>,
    app_data_dir: PathBuf,
    task_data_dir: Mutex<PathBuf>,
    cancellations: Mutex<media::CancellationMap>,
    model_download_active: AtomicBool,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JobRecord {
    id: String,
    title: String,
    platform: String,
    duration: String,
    updated_at: String,
    status: String,
    progress: u8,
    #[serde(default)]
    phase: Option<String>,
    #[serde(default)]
    phase_completed: Option<u64>,
    #[serde(default)]
    phase_total: Option<u64>,
    #[serde(default)]
    phase_unit: Option<String>,
    source_url: String,
    thumbnail_url: Option<String>,
    error_message: Option<String>,
    status_message: Option<String>,
    #[serde(default)]
    asr_backend: Option<String>,
    #[serde(default)]
    asr_config_json: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SystemProfile {
    memory_gb: u64,
    logical_cores: usize,
    recommended_threads: usize,
    gpu_mode: &'static str,
}

fn initialize_database(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS jobs (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            platform TEXT NOT NULL,
            duration TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            status TEXT NOT NULL,
            progress INTEGER NOT NULL,
            phase TEXT,
            phase_completed INTEGER,
            phase_total INTEGER,
            phase_unit TEXT,
            source_url TEXT NOT NULL,
            thumbnail_url TEXT,
            error_message TEXT,
            status_message TEXT,
            asr_backend TEXT,
            asr_config_json TEXT
        );",
    )?;
    let _ = connection.execute("ALTER TABLE jobs ADD COLUMN thumbnail_url TEXT", []);
    let _ = connection.execute("ALTER TABLE jobs ADD COLUMN error_message TEXT", []);
    let _ = connection.execute("ALTER TABLE jobs ADD COLUMN status_message TEXT", []);
    let _ = connection.execute("ALTER TABLE jobs ADD COLUMN phase TEXT", []);
    let _ = connection.execute("ALTER TABLE jobs ADD COLUMN phase_completed INTEGER", []);
    let _ = connection.execute("ALTER TABLE jobs ADD COLUMN phase_total INTEGER", []);
    let _ = connection.execute("ALTER TABLE jobs ADD COLUMN phase_unit TEXT", []);
    let _ = connection.execute("ALTER TABLE jobs ADD COLUMN asr_backend TEXT", []);
    let _ = connection.execute("ALTER TABLE jobs ADD COLUMN asr_config_json TEXT", []);

    let existing_jobs: i64 =
        connection.query_row("SELECT COUNT(*) FROM jobs", [], |row| row.get(0))?;
    if existing_jobs == 0 {
        let demo_jobs = [
            JobRecord {
                id: "rag-overview".into(),
                title: "从零理解 RAG 的工作原理".into(),
                platform: "bilibili".into(),
                duration: "28:47".into(),
                updated_at: "今天 10:24".into(),
                status: "completed".into(),
                progress: 100,
                phase: None,
                phase_completed: None,
                phase_total: None,
                phase_unit: None,
                source_url: "https://www.bilibili.com/video/BV1RAGDEMO".into(),
                thumbnail_url: None,
                error_message: None,
                status_message: None,
                asr_backend: None,
                asr_config_json: None,
            },
            JobRecord {
                id: "rust-async".into(),
                title: "Rust 异步编程完整指南".into(),
                platform: "douyin".into(),
                duration: "56:18".into(),
                updated_at: "今天 09:58".into(),
                status: "processing".into(),
                progress: 68,
                phase: Some("recognition".into()),
                phase_completed: None,
                phase_total: None,
                phase_unit: Some("milliseconds".into()),
                source_url: "https://v.douyin.com/rust-demo/".into(),
                thumbnail_url: None,
                error_message: None,
                status_message: None,
                asr_backend: None,
                asr_config_json: None,
            },
            JobRecord {
                id: "user-interview".into(),
                title: "产品经理如何做好用户访谈".into(),
                platform: "bilibili".into(),
                duration: "34:12".into(),
                updated_at: "昨天 21:16".into(),
                status: "paused".into(),
                progress: 41,
                phase: Some("recognition".into()),
                phase_completed: None,
                phase_total: None,
                phase_unit: Some("milliseconds".into()),
                source_url: "https://www.bilibili.com/video/BV1USERDEMO".into(),
                thumbnail_url: None,
                error_message: None,
                status_message: None,
                asr_backend: None,
                asr_config_json: None,
            },
        ];

        for job in demo_jobs {
            insert_or_update_job(connection, &job)?;
        }
    }

    Ok(())
}

fn insert_or_update_job(connection: &Connection, job: &JobRecord) -> rusqlite::Result<()> {
    connection.execute(
        "INSERT INTO jobs (id, title, platform, duration, updated_at, status, progress, phase, phase_completed, phase_total, phase_unit, source_url, thumbnail_url, error_message, status_message, asr_backend, asr_config_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
         ON CONFLICT(id) DO UPDATE SET
           title = excluded.title,
           platform = excluded.platform,
           duration = excluded.duration,
           updated_at = excluded.updated_at,
           status = excluded.status,
           progress = excluded.progress,
           phase = excluded.phase,
           phase_completed = excluded.phase_completed,
           phase_total = excluded.phase_total,
           phase_unit = excluded.phase_unit,
           source_url = excluded.source_url,
           thumbnail_url = excluded.thumbnail_url,
           error_message = excluded.error_message,
           status_message = excluded.status_message,
           asr_backend = excluded.asr_backend,
           asr_config_json = excluded.asr_config_json",
        params![
            job.id,
            job.title,
            job.platform,
            job.duration,
            job.updated_at,
            job.status,
            job.progress,
            job.phase,
            job.phase_completed,
            job.phase_total,
            job.phase_unit,
            job.source_url,
            job.thumbnail_url,
            job.error_message,
            job.status_message,
            job.asr_backend,
            job.asr_config_json,
        ],
    )?;
    Ok(())
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
    if state
        .cancellations
        .lock()
        .map_err(|_| "任务控制器当前不可用".to_string())?
        .is_empty()
        == false
    {
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

fn remove_path_if_exists(path: &Path) -> Result<(), String> {
    if path.is_dir() {
        fs::remove_dir_all(path).map_err(|error| format!("无法清理任务数据：{error}"))?;
    } else if path.is_file() {
        fs::remove_file(path).map_err(|error| format!("无法清理任务数据：{error}"))?;
    }
    Ok(())
}

fn ensure_job_idle(job_id: &str, state: &State<'_, AppState>) -> Result<(), String> {
    let cancellations = state
        .cancellations
        .lock()
        .map_err(|_| "任务控制器当前不可用".to_string())?;
    if cancellations.contains_key(job_id) {
        Err("任务正在处理中，请先取消并等待处理停止".to_string())
    } else {
        Ok(())
    }
}

fn task_directory(job_id: &str, state: &State<'_, AppState>) -> Result<PathBuf, String> {
    media::validate_job_id(job_id)?;
    Ok(current_task_data_directory(state)?
        .join("tasks")
        .join(job_id))
}

#[tauri::command]
fn list_jobs(state: State<'_, AppState>) -> Result<Vec<JobRecord>, String> {
    let database = state
        .database
        .lock()
        .map_err(|_| "数据库当前不可用".to_string())?;
    let mut statement = database
        .prepare(
            "SELECT id, title, platform, duration, updated_at, status, progress, phase, phase_completed, phase_total, phase_unit, source_url, thumbnail_url, error_message, status_message, asr_backend, asr_config_json
             FROM jobs ORDER BY rowid DESC",
        )
        .map_err(|error| error.to_string())?;

    let rows = statement
        .query_map([], |row| {
            Ok(JobRecord {
                id: row.get(0)?,
                title: row.get(1)?,
                platform: row.get(2)?,
                duration: row.get(3)?,
                updated_at: row.get(4)?,
                status: row.get(5)?,
                progress: row.get(6)?,
                phase: row.get(7)?,
                phase_completed: row.get(8)?,
                phase_total: row.get(9)?,
                phase_unit: row.get(10)?,
                source_url: row.get(11)?,
                thumbnail_url: row.get(12)?,
                error_message: row.get(13)?,
                status_message: row.get(14)?,
                asr_backend: row.get(15)?,
                asr_config_json: row.get(16)?,
            })
        })
        .map_err(|error| error.to_string())?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReconciledJob {
    id: String,
    status: String,
    progress: u8,
    phase: Option<String>,
    phase_completed: Option<u64>,
    phase_total: Option<u64>,
    phase_unit: Option<String>,
    status_message: Option<String>,
    error_message: Option<String>,
}

/// 根据任务目录中实际落盘的结果,推断被中断任务可恢复到的状态。
/// 返回 (status, progress, status_message, error_message)。
fn job_disk_state(task_dir: &Path) -> (String, u8, String, Option<String>) {
    let note_path = task_dir.join("note").join("note.json");
    if let Ok(raw) = fs::read_to_string(&note_path) {
        if let Ok(value) = serde_json::from_str::<Value>(&raw) {
            let has_markdown = value
                .get("markdown")
                .and_then(|value| value.as_str())
                .map(|text| !text.trim().is_empty())
                .unwrap_or(false);
            if has_markdown {
                return (
                    "completed".into(),
                    100,
                    "上次处理已完成笔记整理".into(),
                    None,
                );
            }
        }
    }
    let transcript_path = task_dir.join("transcript").join("transcript.json");
    if let Ok(raw) = fs::read_to_string(&transcript_path) {
        if let Ok(value) = serde_json::from_str::<Value>(&raw) {
            let segment_count = value
                .get("segments")
                .and_then(|value| value.as_array())
                .map(|segments| segments.len())
                .unwrap_or(0);
            if segment_count > 0 {
                return (
                    "transcribed".into(),
                    100,
                    "语音转写已完成；可先人工校正，再按需翻译或生成笔记".into(),
                    None,
                );
            }
        }
    }
    if task_dir.join("media.json").is_file() {
        return (
            "waiting".into(),
            100,
            "本地媒体已就绪；上次处理被中断，可点“开始转写”继续".into(),
            None,
        );
    }
    (
        "failed".into(),
        0,
        "上次处理在保存本地结果前被中断，可重新尝试".into(),
        Some("上次处理被中断，任务目录中没有可恢复的结果".into()),
    )
}

/// 应用启动后调用:把遗留的 processing/paused 任务按磁盘实际结果修正,
/// 避免重启后任务永远停留在“处理中”而实际没有任何后台进程。
#[tauri::command]
fn reconcile_jobs(state: State<'_, AppState>) -> Result<Vec<ReconciledJob>, String> {
    let task_root = current_task_data_directory(&state)?;
    let database = state
        .database
        .lock()
        .map_err(|_| "数据库当前不可用".to_string())?;
    let mut statement = database
        .prepare(
            "SELECT id, title, platform, duration, updated_at, status, progress, phase, phase_completed, phase_total, phase_unit, source_url, thumbnail_url, error_message, status_message, asr_backend, asr_config_json
             FROM jobs",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok(JobRecord {
                id: row.get(0)?,
                title: row.get(1)?,
                platform: row.get(2)?,
                duration: row.get(3)?,
                updated_at: row.get(4)?,
                status: row.get(5)?,
                progress: row.get(6)?,
                phase: row.get(7)?,
                phase_completed: row.get(8)?,
                phase_total: row.get(9)?,
                phase_unit: row.get(10)?,
                source_url: row.get(11)?,
                thumbnail_url: row.get(12)?,
                error_message: row.get(13)?,
                status_message: row.get(14)?,
                asr_backend: row.get(15)?,
                asr_config_json: row.get(16)?,
            })
        })
        .map_err(|error| error.to_string())?;
    let jobs = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;

    let mut reconciled = Vec::new();
    for mut job in jobs {
        if job.status != "processing" && job.status != "paused" {
            continue;
        }
        let task_dir = task_root.join("tasks").join(&job.id);
        let (status, progress, status_message, error_message) = job_disk_state(&task_dir);
        job.status = status;
        job.progress = progress;
        job.phase = None;
        job.phase_completed = None;
        job.phase_total = None;
        job.phase_unit = None;
        job.status_message = Some(status_message);
        job.error_message = error_message;
        insert_or_update_job(&database, &job).map_err(|error| error.to_string())?;
        reconciled.push(ReconciledJob {
            id: job.id,
            status: job.status,
            progress: job.progress,
            phase: job.phase,
            phase_completed: job.phase_completed,
            phase_total: job.phase_total,
            phase_unit: job.phase_unit,
            status_message: job.status_message,
            error_message: job.error_message,
        });
    }
    Ok(reconciled)
}

#[tauri::command]
fn save_job(job: JobRecord, state: State<'_, AppState>) -> Result<(), String> {
    let database = state
        .database
        .lock()
        .map_err(|_| "数据库当前不可用".to_string())?;
    insert_or_update_job(&database, &job).map_err(|error| error.to_string())
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
fn open_task_directory(job_id: String, state: State<'_, AppState>) -> Result<(), String> {
    let directory = task_directory(&job_id, &state)?.join("source");
    fs::create_dir_all(&directory).map_err(|error| format!("无法创建视频目录：{error}"))?;
    #[cfg(windows)]
    {
        Command::new("explorer.exe")
            .arg(&directory)
            .spawn()
            .map_err(|error| format!("无法打开视频目录：{error}"))?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = directory;
        Err("当前系统暂不支持自动打开任务目录".to_string())
    }
}

#[tauri::command]
fn reset_task_media(job_id: String, state: State<'_, AppState>) -> Result<(), String> {
    media::validate_job_id(&job_id)?;
    ensure_job_idle(&job_id, &state)?;
    let directory = task_directory(&job_id, &state)?;
    for path in [
        directory.join("source"),
        directory.join("chunks"),
        directory.join("transcript"),
        directory.join("translation"),
        directory.join("note"),
        directory.join("media.json"),
    ] {
        remove_path_if_exists(&path)?;
    }
    Ok(())
}

#[tauri::command]
fn reset_task_transcript(job_id: String, state: State<'_, AppState>) -> Result<(), String> {
    media::validate_job_id(&job_id)?;
    ensure_job_idle(&job_id, &state)?;
    let directory = task_directory(&job_id, &state)?;
    for path in [
        directory.join("transcript"),
        directory.join("translation"),
        directory.join("note"),
    ] {
        remove_path_if_exists(&path)?;
    }
    Ok(())
}

#[tauri::command]
fn delete_task(job_id: String, state: State<'_, AppState>) -> Result<(), String> {
    media::validate_job_id(&job_id)?;
    ensure_job_idle(&job_id, &state)?;
    let directory = task_directory(&job_id, &state)?;
    remove_path_if_exists(&directory)?;
    let database = state
        .database
        .lock()
        .map_err(|_| "数据库当前不可用".to_string())?;
    database
        .execute("DELETE FROM jobs WHERE id = ?1", params![job_id])
        .map_err(|error| format!("无法删除任务记录：{error}"))?;
    Ok(())
}

#[tauri::command]
async fn export_task_audio(
    job_id: String,
    suggested_filename: String,
    app: AppHandle,
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    media::validate_job_id(&job_id)?;
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
    let worker_job_id = job_id.clone();
    let worker_output_path = output_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        media::export_audio(
            &worker_app,
            &tools,
            &task_data_dir,
            &worker_job_id,
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
async fn prepare_media(
    job_id: String,
    source_url: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<media::MediaPreparationResult, AppError> {
    let tools = media::resolve_media_tools(&app)?;
    let cancelled = Arc::new(AtomicBool::new(false));
    {
        let mut cancellations = state
            .cancellations
            .lock()
            .map_err(|_| AppError::failed("任务控制器当前不可用"))?;
        if cancellations.contains_key(&job_id) {
            return Err(AppError::new("ALREADY_ACTIVE", "该任务已经在处理中"));
        }
        cancellations.insert(job_id.clone(), cancelled.clone());
    }

    let task_data_dir = current_task_data_directory(&state)?;
    let worker_app = app.clone();
    let worker_job_id = job_id.clone();
    let worker_result = tauri::async_runtime::spawn_blocking(move || {
        media::prepare_media(
            &worker_app,
            &tools,
            &task_data_dir,
            &worker_job_id,
            &source_url,
            cancelled,
        )
    })
    .await;

    if let Ok(mut cancellations) = state.cancellations.lock() {
        cancellations.remove(&job_id);
    }
    Ok(worker_result.map_err(|error| AppError::failed(format!("媒体处理任务异常退出：{error}")))??)
}

#[tauri::command]
fn load_media(
    job_id: String,
    state: State<'_, AppState>,
) -> Result<media::MediaPreparationResult, AppError> {
    Ok(media::load_media(&current_task_data_directory(&state)?, &job_id)?)
}

#[tauri::command]
async fn cancel_media_preparation(
    job_id: String,
    state: State<'_, AppState>,
) -> Result<bool, AppError> {
    let has_task = {
        let cancellations = state
            .cancellations
            .lock()
            .map_err(|_| AppError::failed("任务控制器当前不可用"))?;
        if let Some(cancelled) = cancellations.get(&job_id) {
            cancelled.store(true, Ordering::Relaxed);
            true
        } else {
            false
        }
    };
    if has_task {
        // 等待后台处理线程完全退出并清理注销任务锁（最多等待 5 秒）
        for _ in 0..50 {
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            if let Ok(cancellations) = state.cancellations.lock() {
                if !cancellations.contains_key(&job_id) {
                    break;
                }
            }
        }
    }
    Ok(has_task)
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
async fn transcribe_media(
    job_id: String,
    resume: Option<bool>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<asr::TranscriptResult, AppError> {
    let cancelled = Arc::new(AtomicBool::new(false));
    {
        let mut cancellations = state
            .cancellations
            .lock()
            .map_err(|_| AppError::failed("任务控制器当前不可用"))?;
        if cancellations.contains_key(&job_id) {
            return Err(AppError::new("ALREADY_ACTIVE", "该任务已经在处理中"));
        }
        cancellations.insert(job_id.clone(), cancelled.clone());
    }

    let model_data_dir = state.app_data_dir.clone();
    let task_data_dir = current_task_data_directory(&state)?;
    // Snapshot the backend/config at the start of the run.  This keeps a retry
    // deterministic even if the user changes the global ASR setting while it is
    // processing.
    let (asr_backend, asr_config_json) = {
        let database = state
            .database
            .lock()
            .map_err(|_| AppError::failed("数据库当前不可用"))?;
        database
            .query_row(
                "SELECT asr_backend, asr_config_json FROM jobs WHERE id = ?1",
                [&job_id],
                |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .unwrap_or((None, None))
    };
    let asr_backend = asr_backend.unwrap_or_else(|| "funasr-nano".to_string());
    let worker_app = app.clone();
    let worker_job_id = job_id.clone();
    let result = asr::transcribe_job(
        &worker_app,
        &model_data_dir,
        &task_data_dir,
        &worker_job_id,
        &asr_backend,
        asr_config_json.as_deref(),
        resume.unwrap_or(false),
        cancelled,
    )
    .await
    .map_err(AppError::from);
    if let Ok(mut cancellations) = state.cancellations.lock() {
        cancellations.remove(&job_id);
    }
    result
}

#[tauri::command]
fn load_transcript(
    job_id: String,
    state: State<'_, AppState>,
) -> Result<asr::TranscriptResult, AppError> {
    Ok(asr::load_transcript(&current_task_data_directory(&state)?, &job_id)?)
}

fn view_segments_to_transcript_result(
    job_id: &str,
    model_id: &str,
    language: &str,
    translation_language: Option<String>,
    segments: Vec<crate::transcript::views::ViewSegment>,
) -> asr::TranscriptResult {
    let mapped = segments
        .into_iter()
        .enumerate()
        .map(|(idx, segment)| asr::TranscriptSegment {
            id: segment.id,
            chunk_index: idx,
            start: segment.start_ms as f64 / 1000.0,
            end: segment.end_ms as f64 / 1000.0,
            start_ms: segment.start_ms,
            end_ms: segment.end_ms,
            text: segment.text,
            translated_text: segment.translated_text,
            avg_confidence: None,
        })
        .collect::<Vec<_>>();
    asr::TranscriptResult {
        job_id: job_id.to_string(),
        model_id: model_id.to_string(),
        language: language.to_string(),
        translation_language,
        text: mapped.iter().map(|s| s.text.as_str()).collect::<Vec<_>>().join("\n"),
        segments: mapped,
        pause_repairs: None,
    }
}

/// Loads a backend-owned transcript view. Only Raw and Standard are product views;
/// the frontend must not re-implement Canonical transforms or invent timestamps.
#[tauri::command]
fn load_transcript_view(
    job_id: String,
    view: String,
    state: State<'_, AppState>,
) -> Result<asr::TranscriptResult, String> {
    media::validate_job_id(&job_id)?;
    let task_data_dir = current_task_data_directory(&state)?;
    let transcript_dir = task_data_dir.join("tasks").join(&job_id).join("transcript");
    let standard = asr::load_transcript(&task_data_dir, &job_id)?;

    match view.trim().to_ascii_lowercase().as_str() {
        "standard" => Ok(standard),
        "raw" => {
            if let Some(raw) = crate::transcript::storage::load_raw_transcript(&transcript_dir)? {
                let language = raw.language.as_deref().unwrap_or(&standard.language);
                Ok(view_segments_to_transcript_result(
                    &job_id,
                    &standard.model_id,
                    language,
                    None,
                    crate::transcript::views::render_raw_view(&raw),
                ))
            } else {
                Err("该任务没有可用的 Raw 原始转录；为避免把 Standard 误显示为原始内容，已拒绝回退。".to_string())
            }
        }
        _ => Err(format!("不支持的 transcript view：{view}")),
    }
}

/// 修改某条转录段文本并落盘(transcript.json + transcript.txt 同步更新)。
#[tauri::command]
fn update_transcript_segment(
    job_id: String,
    segment_id: String,
    text: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let task_data_dir = current_task_data_directory(&state)?;
    let mut transcript = asr::load_transcript(&task_data_dir, &job_id)?;
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
    asr::save_transcript(&task_data_dir, &job_id, &transcript)
        .map_err(|error| format!("无法保存转录修改：{error}"))?;

    // Notes are also derived from Standard text. Remove stale note artifacts so a later
    // manual “生成笔记” action always rebuilds from the edited transcript.
    let note_dir = task_data_dir.join("tasks").join(&job_id).join("note");
    if note_dir.exists() {
        fs::remove_dir_all(&note_dir).map_err(|error| format!("无法使旧笔记失效：{error}"))?;
    }
    Ok(())
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn organize_notes(
    job_id: String,
    title: String,
    source_url: String,
    platform: String,
    duration: String,
    force: bool,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<summary::NoteResult, AppError> {
    let cancelled = Arc::new(AtomicBool::new(false));
    {
        let mut cancellations = state
            .cancellations
            .lock()
            .map_err(|_| AppError::failed("任务控制器当前不可用"))?;
        if cancellations.contains_key(&job_id) {
            return Err(AppError::new("ALREADY_ACTIVE", "该任务已经在处理中"));
        }
        cancellations.insert(job_id.clone(), cancelled.clone());
    }

    let model_data_dir = state.app_data_dir.clone();
    let task_data_dir = current_task_data_directory(&state)?;
    let worker_app = app.clone();
    let worker_job_id = job_id.clone();
    let worker_result = tauri::async_runtime::spawn_blocking(move || {
        summary::organize_job(
            &worker_app,
            &model_data_dir,
            &task_data_dir,
            &worker_job_id,
            &title,
            &source_url,
            &platform,
            &duration,
            force,
            cancelled,
        )
    })
    .await;
    if let Ok(mut cancellations) = state.cancellations.lock() {
        cancellations.remove(&job_id);
    }
    Ok(worker_result.map_err(|error| AppError::failed(format!("内容整理任务异常退出：{error}")))??)
}

/// 用户主动触发的翻译任务：仅把非中文标准转录翻译为简体中文并保存，不做笔记整理。
#[tauri::command]
async fn translate_transcript(
    job_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let cancelled = Arc::new(AtomicBool::new(false));
    {
        let mut cancellations = state
            .cancellations
            .lock()
            .map_err(|_| AppError::failed("任务控制器当前不可用"))?;
        if cancellations.contains_key(&job_id) {
            return Err(AppError::new("ALREADY_ACTIVE", "该任务已经在处理中"));
        }
        cancellations.insert(job_id.clone(), cancelled.clone());
    }

    let model_data_dir = state.app_data_dir.clone();
    let task_data_dir = current_task_data_directory(&state)?;
    let worker_app = app.clone();
    let worker_job_id = job_id.clone();
    let worker_result = tauri::async_runtime::spawn_blocking(move || {
        summary::translate_job(
            &worker_app,
            &model_data_dir,
            &task_data_dir,
            &worker_job_id,
            cancelled,
        )
    })
    .await;
    if let Ok(mut cancellations) = state.cancellations.lock() {
        cancellations.remove(&job_id);
    }
    Ok(worker_result.map_err(|error| AppError::failed(format!("翻译任务异常退出：{error}")))??)
}

#[tauri::command]
fn load_note(job_id: String, state: State<'_, AppState>) -> Result<summary::NoteResult, AppError> {
    Ok(summary::load_note(&current_task_data_directory(&state)?, &job_id)?)
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

#[tauri::command]
fn system_profile() -> SystemProfile {
    let logical_cores = std::thread::available_parallelism().map_or(4, usize::from);
    let system = System::new_all();
    let memory_gb = (system.total_memory() as f64 / 1024_f64.powi(3)).round() as u64;

    SystemProfile {
        memory_gb,
        logical_cores,
        recommended_threads: logical_cores.saturating_sub(2).max(2),
        gpu_mode: "cpu",
    }
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
            initialize_database(&connection)
                .map_err(|error| io::Error::other(error.to_string()))?;
            app.manage(AppState {
                database: Mutex::new(connection),
                app_data_dir,
                task_data_dir: Mutex::new(task_data_dir),
                cancellations: Mutex::new(media::CancellationMap::new()),
                model_download_active: AtomicBool::new(false),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_jobs,
            reconcile_jobs,
            save_job,
            inspect_data_directory,
            choose_data_directory,
            reset_data_directory,
            open_task_directory,
            reset_task_media,
            reset_task_transcript,
            delete_task,
            export_task_audio,
            parse_video_input,
            search_videos,
            inspect_media_tools,
            prepare_media,
            load_media,
            cancel_media_preparation,
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
            transcribe_media,
            load_transcript,
            load_transcript_view,
            update_transcript_segment,
            organize_notes,
            translate_transcript,
            load_note,
            export_markdown,
            system_profile
        ])
        .run(tauri::generate_context!())
        .expect("failed to run VideoNotes");
}

#[cfg(test)]
mod tests {
    use super::{copy_directory_contents, job_disk_state, rewrite_local_paths};
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

    #[test]
    fn job_disk_state_detects_latest_recoverable_stage() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("video-notes-reconcile-{}-{unique}", process::id()));
        let task = root.join("tasks").join("job-1");
        fs::create_dir_all(&task).expect("create task directory");

        // 目录为空 → 无任何可恢复结果
        assert_eq!(job_disk_state(&task).0, "failed");

        // 只有媒体清单 → waiting
        fs::write(task.join("media.json"), "{}").expect("write media.json");
        assert_eq!(job_disk_state(&task).0, "waiting");

        // 转录完成 → transcribed
        let transcript_dir = task.join("transcript");
        fs::create_dir_all(&transcript_dir).expect("create transcript directory");
        fs::write(
            transcript_dir.join("transcript.json"),
            r#"{"segments":[{"id":"0-0","startMs":0,"endMs":1000,"text":"hi"}]}"#,
        )
        .expect("write transcript.json");
        assert_eq!(job_disk_state(&task).0, "transcribed");

        // 笔记已生成 → completed
        let note_dir = task.join("note");
        fs::create_dir_all(&note_dir).expect("create note directory");
        fs::write(note_dir.join("note.json"), r##"{"markdown":"# 标题"}"##)
            .expect("write note.json");
        assert_eq!(job_disk_state(&task).0, "completed");

        fs::remove_dir_all(root).expect("remove test directory");
    }
}

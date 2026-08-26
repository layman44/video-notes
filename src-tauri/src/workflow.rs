//! Persistent video/queue workflow.
//!
//! This module is deliberately the only owner of queue state.  Media and ASR
//! remain in their existing modules; this layer owns their ordering,
//! persistence, recovery, and the library/queue boundary.

use crate::{asr, media, openasr, translation};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter, Listener, Manager};

pub const SCHEMA_VERSION: i32 = 3;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoRecord {
    pub id: String,
    pub title: String,
    pub platform: String,
    pub duration: String,
    pub source_url: String,
    pub normalized_source_key: String,
    pub author: Option<String>,
    pub thumbnail_url: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub library_available_at: Option<String>,
    pub deleted_at: Option<String>,
    pub transcript_status: String,
    pub translation_status: String,
    pub note_status: String,
    pub media_status: String,
    pub transcript_language: Option<String>,
    pub queue_item_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoPage {
    pub items: Vec<VideoRecord>,
    pub total: u64,
    pub page: u32,
    pub page_size: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoSourceLookupInput {
    pub platform: String,
    pub source_url: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoSourceLookup {
    pub platform: String,
    pub source_url: String,
    pub video: Option<VideoRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueItem {
    pub id: String,
    pub video_id: String,
    pub position: i64,
    pub state: String,
    pub stage: String,
    pub progress: u8,
    pub phase_completed: Option<u64>,
    pub phase_total: Option<u64>,
    pub phase_unit: Option<String>,
    pub attempt_count: u32,
    pub asr_backend: Option<String>,
    pub asr_config_json: Option<String>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub updated_at: String,
    pub finished_at: Option<String>,
    pub progress_completed: Option<u64>,
    pub progress_total: Option<u64>,
    pub status_message: Option<String>,
    pub title: String,
    pub platform: String,
    pub duration: String,
    pub source_url: String,
    pub author: Option<String>,
    pub thumbnail_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnqueueInput {
    pub title: String,
    pub platform: String,
    pub duration: String,
    pub source_url: String,
    pub author: Option<String>,
    pub thumbnail_url: Option<String>,
    pub asr_backend: Option<String>,
    pub asr_config_json: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnqueueOutcome {
    pub source_url: String,
    pub normalized_source_key: String,
    pub outcome: String,
    pub video: VideoRecord,
    pub queue_item: Option<QueueItem>,
}

pub struct WorkflowState {
    pub database: Arc<Mutex<Connection>>,
    pub task_data_dir: Arc<Mutex<PathBuf>>,
    pub app: Mutex<Option<AppHandle>>,
    scheduler_running: AtomicBool,
    heavy: Arc<(Mutex<bool>, Condvar)>,
    cancellations: Mutex<std::collections::HashMap<String, Arc<AtomicBool>>>,
}

impl WorkflowState {
    pub fn new(database: Arc<Mutex<Connection>>, task_data_dir: Arc<Mutex<PathBuf>>) -> Self {
        Self {
            database,
            task_data_dir,
            app: Mutex::new(None),
            scheduler_running: AtomicBool::new(false),
            heavy: Arc::new((Mutex::new(false), Condvar::new())),
            cancellations: Mutex::new(std::collections::HashMap::new()),
        }
    }

    pub fn set_app(self: &Arc<Self>, app: AppHandle) {
        let media_state = Arc::downgrade(self);
        app.listen("media-progress", move |event| {
            let Some(state) = media_state.upgrade() else {
                return;
            };
            if let Ok(progress) = serde_json::from_str::<MediaProgressPayload>(event.payload()) {
                let _ = state.record_media_progress(progress);
            }
        });

        let asr_state = Arc::downgrade(self);
        app.listen("asr-phase-progress", move |event| {
            let Some(state) = asr_state.upgrade() else {
                return;
            };
            if let Ok(progress) = serde_json::from_str::<AsrProgressPayload>(event.payload()) {
                let _ = state.record_asr_progress(progress);
            }
        });

        let asr_phase_state = Arc::downgrade(self);
        app.listen("asr-phase", move |event| {
            let Some(state) = asr_phase_state.upgrade() else {
                return;
            };
            if let Ok(phase) = serde_json::from_str::<AsrPhasePayload>(event.payload()) {
                let _ = state.record_asr_phase(phase);
            }
        });

        if let Ok(mut slot) = self.app.lock() {
            *slot = Some(app);
        }
    }

    fn app(&self) -> Option<AppHandle> {
        self.app.lock().ok().and_then(|v| v.clone())
    }

    pub fn emit(&self, name: &str) {
        if let Some(app) = self.app() {
            let _ = app.emit(name, ());
        }
    }

    pub fn acquire_heavy(&self, id: &str, cancel: &Arc<AtomicBool>) -> Result<HeavyPermit, String> {
        let (lock, cvar) = &*self.heavy;
        let mut active = lock.lock().map_err(|_| "推理资源不可用".to_string())?;
        while *active {
            if cancel.load(Ordering::Relaxed) {
                return Err("任务已取消".into());
            }
            active = cvar
                .wait_timeout(active, std::time::Duration::from_millis(100))
                .map_err(|_| "推理资源不可用".to_string())?
                .0;
        }
        *active = true;
        Ok(HeavyPermit {
            state: self.heavy.clone(),
            id: id.to_string(),
        })
    }

    pub fn cancel(&self, id: &str) -> bool {
        self.cancellations
            .lock()
            .ok()
            .and_then(|m| m.get(id).cloned())
            .map(|v| {
                v.store(true, Ordering::Relaxed);
                true
            })
            .unwrap_or(false)
    }
    pub fn is_active(&self, id: &str) -> bool {
        self.cancellations
            .lock()
            .map(|m| m.contains_key(id))
            .unwrap_or(false)
    }
    pub fn has_active(&self) -> bool {
        self.cancellations
            .lock()
            .map(|m| !m.is_empty())
            .unwrap_or(true)
    }

    pub fn enqueue_cancel(&self, id: &str) -> Result<Arc<AtomicBool>, String> {
        let mut map = self
            .cancellations
            .lock()
            .map_err(|_| "任务控制器当前不可用".to_string())?;
        if map.contains_key(id) {
            return Err("该视频已经在处理中".into());
        }
        let value = Arc::new(AtomicBool::new(false));
        map.insert(id.to_string(), value.clone());
        Ok(value)
    }

    pub fn release_cancel(&self, id: &str) {
        if let Ok(mut map) = self.cancellations.lock() {
            map.remove(id);
        }
    }

    pub fn start_scheduler(self: &Arc<Self>) {
        if self.scheduler_running.swap(true, Ordering::SeqCst) {
            return;
        }
        let this = self.clone();
        tauri::async_runtime::spawn_blocking(move || {
            loop {
                match this.process_next() {
                    Ok(true) => continue,
                    Ok(false) | Err(_) => break,
                }
            }
            this.scheduler_running.store(false, Ordering::SeqCst);
            // A concurrent enqueue can race with the final empty read.
            if this.has_queued() {
                this.start_scheduler();
            }
        });
    }

    fn has_queued(&self) -> bool {
        self.database
            .lock()
            .ok()
            .and_then(|db| {
                db.query_row(
                    "SELECT 1 FROM queue_items WHERE state='queued' LIMIT 1",
                    [],
                    |_| Ok(1),
                )
                .optional()
                .ok()
            })
            .flatten()
            .is_some()
    }

    fn process_next(&self) -> Result<bool, String> {
        let item = {
            let db = self
                .database
                .lock()
                .map_err(|_| "数据库当前不可用".to_string())?;
            db.query_row("SELECT id, video_id, position, state, stage, progress, phase_completed, phase_total, phase_unit, attempt_count, asr_backend, asr_config_json, error_code, error_message, created_at, started_at, updated_at, finished_at, status_message FROM queue_items WHERE state='queued' ORDER BY position, created_at LIMIT 1", [], row_queue).optional().map_err(|e| e.to_string())?
        };
        let Some(item) = item else {
            return Ok(false);
        };
        let cancel = self.enqueue_cancel(&item.id)?;
        self.mark_running(&item.id)?;
        let result = self.run_item(&item, &cancel);
        self.release_cancel(&item.id);
        match result {
            Ok(()) => self.finish_item(&item.id, true, None),
            Err(error) if error == "__BLOCKED__" => self.finish_item_state(
                &item.id,
                "blocked",
                Some("MODEL_NOT_INSTALLED"),
                Some("转录模型尚未安装"),
            ),
            Err(_error) if cancel.load(Ordering::Relaxed) => self.finish_item_state(
                &item.id,
                "paused",
                Some("USER_CANCELLED"),
                Some("已暂停，媒体和已生成的断点将被保留"),
            ),
            Err(error) => self.finish_item(&item.id, false, Some(error)),
        }?;
        Ok(true)
    }

    fn run_item(&self, item: &QueueItem, cancel: &Arc<AtomicBool>) -> Result<(), String> {
        let app = self.app().ok_or("应用尚未完成初始化")?;
        let task_root = self
            .task_data_dir
            .lock()
            .map_err(|_| "数据目录当前不可用".to_string())?
            .clone();
        let (source_url, backend, config) = {
            let db = self
                .database
                .lock()
                .map_err(|_| "数据库当前不可用".to_string())?;
            db.query_row("SELECT source_url, (SELECT asr_backend FROM queue_items WHERE id=?1), (SELECT asr_config_json FROM queue_items WHERE id=?1) FROM videos WHERE id=?2", params![item.id, item.video_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, Option<String>>(2)?))).map_err(|e| e.to_string())?
        };
        let backend = backend.unwrap_or_else(|| "funasr-nano".into());
        let task_id = &item.video_id;
        let media_ready = valid_media_manifest(&task_root.join("tasks").join(task_id));
        if !media_ready {
            self.update_stage(&item.id, "download", 0, None)?;
            let tools = media::resolve_media_tools(&app).map_err(|e| e.to_string())?;
            media::prepare_media(
                &app,
                &tools,
                &task_root,
                task_id,
                &source_url,
                cancel.clone(),
            )
            .map_err(|e| e.to_string())?;
        }
        if cancel.load(Ordering::Relaxed) {
            return Err("任务已取消".into());
        }
        self.update_stage(&item.id, "transcribe", 0, None)?;
        let installed = if backend.starts_with("openasr") || backend.starts_with("moss") {
            openasr::model_status(&app).installed
        } else {
            asr::model_status(&app, &self.app_data_dir()).installed
        };
        if !installed {
            return Err("__BLOCKED__".into());
        }
        let _permit = self.acquire_heavy(&item.id, cancel)?;
        tauri::async_runtime::block_on(asr::transcribe_job(
            &app,
            &self.app_data_dir(),
            &task_root,
            task_id,
            &backend,
            config.as_deref(),
            backend.starts_with("openasr") || backend.starts_with("moss"),
            cancel.clone(),
        ))
        .map_err(|e| e.to_string())?;
        if cancel.load(Ordering::Relaxed) {
            return Err("任务已取消".into());
        }
        let mut db = self
            .database
            .lock()
            .map_err(|_| "数据库当前不可用".to_string())?;
        let now = now();
        let tx = db.transaction().map_err(|e| e.to_string())?;
        tx.execute(
            "UPDATE videos SET library_available_at=?1, updated_at=?1 WHERE id=?2",
            params![now, task_id],
        )
        .map_err(|e| e.to_string())?;
        tx.execute("INSERT INTO artifacts(video_id,artifact_type,state,relative_path,revision,updated_at) VALUES(?1,'standard_transcript','ready','transcript/transcript.json',1,?2) ON CONFLICT(video_id,artifact_type) DO UPDATE SET state='ready',revision=artifacts.revision+1,updated_at=excluded.updated_at", params![task_id, now]).map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    }

    fn app_data_dir(&self) -> PathBuf {
        self.app()
            .and_then(|a| a.path().app_data_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."))
    }
    fn mark_running(&self, id: &str) -> Result<(), String> {
        let db = self
            .database
            .lock()
            .map_err(|_| "数据库当前不可用".to_string())?;
        db.execute("UPDATE queue_items SET state='running',attempt_count=attempt_count+1,started_at=?1,updated_at=?1,error_code=NULL,error_message=NULL,status_message='正在准备处理',phase_completed=0,phase_total=100 WHERE id=?2", params![now(),id]).map_err(|e| e.to_string())?;
        self.emit("queue-updated");
        Ok(())
    }
    fn update_stage(
        &self,
        id: &str,
        stage: &str,
        progress: u8,
        message: Option<&str>,
    ) -> Result<(), String> {
        let db = self
            .database
            .lock()
            .map_err(|_| "数据库当前不可用".to_string())?;
        db.execute("UPDATE queue_items SET state='running',stage=?1,progress=?2,phase_completed=?2,phase_total=100,status_message=?3,updated_at=?4 WHERE id=?5", params![stage, progress, message, now(), id]).map_err(|e| e.to_string())?;
        self.emit("queue-updated");
        Ok(())
    }
    fn finish_item(&self, id: &str, success: bool, error: Option<String>) -> Result<(), String> {
        self.finish_item_state(
            id,
            if success { "completed" } else { "failed" },
            if success {
                None
            } else {
                Some("PIPELINE_FAILED")
            },
            error.as_deref(),
        )
    }
    fn finish_item_state(
        &self,
        id: &str,
        state: &str,
        code: Option<&str>,
        error: Option<&str>,
    ) -> Result<(), String> {
        let db = self
            .database
            .lock()
            .map_err(|_| "数据库当前不可用".to_string())?;
        let success = state == "completed";
        db.execute(
            "UPDATE queue_items SET state=?1,progress=CASE WHEN ?2 THEN 100 ELSE progress END,error_code=?3,error_message=?4,status_message=?5,finished_at=?6,updated_at=?6 WHERE id=?7",
            params![state, success, code, error, if success { Some("处理完成") } else { None }, now(), id],
        )
        .map_err(|error| error.to_string())?;
        self.emit("queue-updated");
        if success {
            self.emit("library-updated");
        }
        Ok(())
    }

    fn record_media_progress(&self, payload: MediaProgressPayload) -> Result<(), String> {
        let stage = match payload.stage.as_str() {
            "download" => "download",
            "normalize" | "ready" => "normalize",
            _ => return Ok(()),
        };
        let db = self
            .database
            .lock()
            .map_err(|_| "数据库当前不可用".to_string())?;
        db.execute(
            "UPDATE queue_items SET stage=?1,progress=?2,phase_completed=?2,phase_total=100,phase_unit='percent',status_message=?3,updated_at=?4 WHERE video_id=?5 AND state='running'",
            params![stage, payload.progress.min(100), payload.message, now(), payload.job_id],
        )
        .map_err(|error| error.to_string())?;
        drop(db);
        self.emit("queue-updated");
        Ok(())
    }

    fn record_asr_progress(&self, payload: AsrProgressPayload) -> Result<(), String> {
        let total = payload.total.unwrap_or_else(|| payload.completed.max(1));
        let percent = if total == 0 {
            0
        } else {
            ((payload.completed.saturating_mul(100) / total).min(99)) as u8
        };
        let db = self
            .database
            .lock()
            .map_err(|_| "数据库当前不可用".to_string())?;
        db.execute(
            "UPDATE queue_items SET stage='transcribe',progress=?1,phase_completed=?2,phase_total=?3,phase_unit=?4,status_message=?5,updated_at=?6 WHERE video_id=?7 AND state='running'",
            params![percent, payload.completed, total, payload.unit, payload.message, now(), payload.job_id],
        )
        .map_err(|error| error.to_string())?;
        drop(db);
        self.emit("queue-updated");
        Ok(())
    }

    fn record_asr_phase(&self, payload: AsrPhasePayload) -> Result<(), String> {
        let db = self
            .database
            .lock()
            .map_err(|_| "数据库当前不可用".to_string())?;
        db.execute(
            "UPDATE queue_items SET stage='transcribe',status_message=?1,updated_at=?2 WHERE video_id=?3 AND state='running'",
            params![payload.message, now(), payload.job_id],
        )
        .map_err(|error| error.to_string())?;
        drop(db);
        self.emit("queue-updated");
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MediaProgressPayload {
    job_id: String,
    stage: String,
    progress: u8,
    message: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AsrProgressPayload {
    job_id: String,
    #[allow(dead_code)]
    phase: String,
    completed: u64,
    total: Option<u64>,
    unit: String,
    message: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AsrPhasePayload {
    job_id: String,
    #[allow(dead_code)]
    phase: String,
    #[allow(dead_code)]
    state: String,
    message: String,
}

pub struct HeavyPermit {
    state: Arc<(Mutex<bool>, Condvar)>,
    #[allow(dead_code)]
    id: String,
}
impl Drop for HeavyPermit {
    fn drop(&mut self) {
        if let Ok(mut active) = self.state.0.lock() {
            *active = false;
            self.state.1.notify_one();
        }
    }
}

fn now() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .to_string()
}
fn id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("video-{nanos}-{}", std::process::id())
}

pub fn normalize_source_key(platform: &str, source: &str) -> String {
    let lower = source.trim().to_ascii_lowercase();
    if platform.eq_ignore_ascii_case("bilibili") {
        let upper = source.to_ascii_uppercase();
        if let Some(pos) = upper.find("BV") {
            let tail = upper[pos + 2..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .take(10)
                .collect::<String>();
            if tail.len() >= 8 {
                return format!("bilibili:bv{}", tail.to_ascii_lowercase());
            }
        }
    }
    url::Url::parse(&lower)
        .map(|mut u| {
            u.set_fragment(None);
            u.set_query(None);
            u.to_string().trim_end_matches('/').to_string()
        })
        .unwrap_or(lower)
}

fn row_video(r: &rusqlite::Row<'_>) -> rusqlite::Result<VideoRecord> {
    Ok(VideoRecord {
        id: r.get(0)?,
        title: r.get(1)?,
        platform: r.get(2)?,
        duration: r.get(3)?,
        source_url: r.get(4)?,
        normalized_source_key: r.get(5)?,
        author: r.get(6)?,
        thumbnail_url: r.get(7)?,
        created_at: r.get(8)?,
        updated_at: r.get(9)?,
        library_available_at: r.get(10)?,
        deleted_at: r.get(11)?,
        transcript_status: if r.get::<_, Option<String>>(10)?.is_some() {
            "ready".into()
        } else {
            "missing".into()
        },
        translation_status: "missing".into(),
        note_status: "missing".into(),
        media_status: "available".into(),
        transcript_language: None,
        queue_item_id: None,
    })
}
fn row_queue(r: &rusqlite::Row<'_>) -> rusqlite::Result<QueueItem> {
    let completed = r.get(6)?;
    let total = r.get(7)?;
    Ok(QueueItem {
        id: r.get(0)?,
        video_id: r.get(1)?,
        position: r.get(2)?,
        state: r.get(3)?,
        stage: r.get(4)?,
        progress: r.get(5)?,
        phase_completed: completed,
        phase_total: total,
        phase_unit: r.get(8)?,
        attempt_count: r.get(9)?,
        asr_backend: r.get(10)?,
        asr_config_json: r.get(11)?,
        error_code: r.get(12)?,
        error_message: r.get(13)?,
        created_at: r.get(14)?,
        started_at: r.get(15)?,
        updated_at: r.get(16)?,
        finished_at: r.get(17)?,
        progress_completed: completed,
        progress_total: total,
        status_message: r.get(18)?,
        title: String::new(),
        platform: String::new(),
        duration: String::new(),
        source_url: String::new(),
        author: None,
        thumbnail_url: None,
    })
}
fn column_exists(db: &Connection, table: &str, column: &str) -> rusqlite::Result<bool> {
    let mut statement = db.prepare(&format!("PRAGMA table_info({table})"))?;
    let names = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(names.iter().any(|name| name == column))
}

pub fn initialize_database(db: &Connection, task_root: &Path) -> rusqlite::Result<()> {
    db.execute_batch("PRAGMA foreign_keys=ON; BEGIN IMMEDIATE; CREATE TABLE IF NOT EXISTS videos(id TEXT PRIMARY KEY,title TEXT NOT NULL,platform TEXT NOT NULL,duration TEXT NOT NULL,source_url TEXT NOT NULL,normalized_source_key TEXT NOT NULL UNIQUE,author TEXT,thumbnail_url TEXT,created_at TEXT NOT NULL,updated_at TEXT NOT NULL,library_available_at TEXT,deleted_at TEXT); DROP INDEX IF EXISTS videos_library_order; DROP INDEX IF EXISTS videos_library_platform_order; CREATE INDEX videos_library_order ON videos(updated_at DESC,id DESC) WHERE library_available_at IS NOT NULL AND deleted_at IS NULL; CREATE INDEX videos_library_platform_order ON videos(platform,updated_at DESC,id DESC) WHERE library_available_at IS NOT NULL AND deleted_at IS NULL; CREATE TABLE IF NOT EXISTS queue_items(id TEXT PRIMARY KEY,video_id TEXT NOT NULL REFERENCES videos(id) ON DELETE CASCADE,position INTEGER NOT NULL,state TEXT NOT NULL,stage TEXT NOT NULL,progress INTEGER NOT NULL DEFAULT 0,phase_completed INTEGER,phase_total INTEGER,phase_unit TEXT,attempt_count INTEGER NOT NULL DEFAULT 0,asr_backend TEXT,asr_config_json TEXT,error_code TEXT,error_message TEXT,status_message TEXT,created_at TEXT NOT NULL,started_at TEXT,updated_at TEXT NOT NULL,finished_at TEXT); CREATE INDEX IF NOT EXISTS queue_items_order ON queue_items(state,position,created_at); CREATE INDEX IF NOT EXISTS queue_items_video_state ON queue_items(video_id,state,position); CREATE TABLE IF NOT EXISTS artifacts(video_id TEXT NOT NULL REFERENCES videos(id) ON DELETE CASCADE,artifact_type TEXT NOT NULL,state TEXT NOT NULL,relative_path TEXT,revision INTEGER NOT NULL DEFAULT 1,content_hash TEXT,input_revision INTEGER,updated_at TEXT NOT NULL,PRIMARY KEY(video_id,artifact_type));")?;
    if !column_exists(db, "queue_items", "status_message")? {
        db.execute("ALTER TABLE queue_items ADD COLUMN status_message TEXT", [])?;
    }
    let has_jobs: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='jobs')",
        [],
        |r| r.get(0),
    )?;
    if has_jobs {
        migrate_jobs(db, task_root)?;
        db.execute_batch("DROP TABLE jobs;")?;
    }
    recover_interrupted_queue(db, task_root)?;
    db.execute_batch(&format!("PRAGMA user_version={SCHEMA_VERSION}; COMMIT;"))
}

fn recover_interrupted_queue(db: &Connection, task_root: &Path) -> rusqlite::Result<()> {
    let mut statement = db.prepare("SELECT id,video_id FROM queue_items WHERE state='running'")?;
    let items = statement
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    for (queue_id, video_id) in items {
        let dir = task_root.join("tasks").join(&video_id);
        if valid_transcript(&dir) {
            db.execute(
                "UPDATE videos SET library_available_at=?1,updated_at=?1 WHERE id=?2",
                params![now(), video_id],
            )?;
            db.execute("UPDATE queue_items SET state='completed',stage='transcribe',progress=100,started_at=NULL,finished_at=?1,updated_at=?1 WHERE id=?2", params![now(),queue_id])?;
            db.execute("INSERT INTO artifacts(video_id,artifact_type,state,relative_path,revision,updated_at) VALUES(?1,'standard_transcript','ready','transcript/transcript.json',1,?2) ON CONFLICT(video_id,artifact_type) DO UPDATE SET state='ready',relative_path=excluded.relative_path,updated_at=excluded.updated_at", params![video_id,now()])?;
        } else {
            let stage = if valid_media_manifest(&dir) {
                "transcribe"
            } else {
                "download"
            };
            db.execute("UPDATE queue_items SET state='queued',stage=?1,progress=0,started_at=NULL,error_code='INTERRUPTED',error_message='应用上次退出时任务未完成，已恢复到可复用阶段',updated_at=?2 WHERE id=?3", params![stage,now(),queue_id])?;
        }
    }
    Ok(())
}

fn migrate_jobs(db: &Connection, task_root: &Path) -> rusqlite::Result<()> {
    let mut stmt = db.prepare("SELECT id,title,platform,duration,updated_at,status,progress,source_url,thumbnail_url,error_message,asr_backend,asr_config_json FROM jobs")?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, u8>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, Option<String>>(8)?,
                r.get::<_, Option<String>>(9)?,
                r.get::<_, Option<String>>(10)?,
                r.get::<_, Option<String>>(11)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (
        legacy_id,
        title,
        platform,
        duration,
        updated,
        status,
        progress,
        source,
        thumb,
        error,
        backend,
        config,
    ) in rows
    {
        if is_demo(&legacy_id, &source) {
            continue;
        }
        let key = normalize_source_key(&platform, &source);
        let created = if updated.chars().all(|c| c.is_ascii_digit()) {
            updated.clone()
        } else {
            now()
        };
        db.execute("INSERT OR IGNORE INTO videos(id,title,platform,duration,source_url,normalized_source_key,thumbnail_url,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?8)",params![legacy_id,title,platform,duration,source,key,thumb,created])?;
        let dir = task_root.join("tasks").join(&legacy_id);
        let media_ok = valid_media_manifest(&dir);
        let transcript_ok = valid_transcript(&dir);
        let library = transcript_ok;
        if library {
            db.execute(
                "UPDATE videos SET library_available_at=?1 WHERE id=?2",
                params![now(), legacy_id],
            )?;
            db.execute("INSERT OR REPLACE INTO artifacts(video_id,artifact_type,state,relative_path,revision,updated_at) VALUES(?1,'standard_transcript','ready','transcript/transcript.json',1,?2)",params![legacy_id,now()])?;
            if transcript_has_translation(&dir)
                && write_translation_artifact(task_root, &legacy_id).is_ok()
            {
                db.execute("INSERT OR REPLACE INTO artifacts(video_id,artifact_type,state,relative_path,revision,updated_at) VALUES(?1,'translation','ready','translation/translation.json',1,?2)",params![legacy_id,now()])?;
            }
            if valid_json_file(&dir.join("note/note.json")) {
                db.execute("INSERT OR REPLACE INTO artifacts(video_id,artifact_type,state,relative_path,revision,updated_at) VALUES(?1,'note','ready','note/note.json',1,?2)",params![legacy_id,now()])?;
            }
        } else {
            let state = match status.as_str() {
                "paused" => "paused",
                "failed" => "failed",
                "waiting" => "queued",
                _ => "queued",
            };
            let stage = if media_ok { "transcribe" } else { "download" };
            let legacy_code = error.as_ref().map(|_| "LEGACY_ERROR");
            db.execute("INSERT OR IGNORE INTO queue_items(id,video_id,position,state,stage,progress,attempt_count,asr_backend,asr_config_json,error_code,error_message,created_at,updated_at) VALUES(?1,?1,(SELECT COALESCE(MAX(position),0)+1 FROM queue_items),?2,?3,?4,0,?5,?6,?7,?8,?9,?9)",params![legacy_id,state,stage,progress,backend,config,legacy_code,error,created])?;
        }
    }
    Ok(())
}
fn is_demo(id: &str, url: &str) -> bool {
    (id == "rag-overview" && url == "https://www.bilibili.com/video/BV1RAGDEMO")
        || (id == "rust-async" && url == "https://v.douyin.com/rust-demo/")
        || (id == "user-interview" && url == "https://www.bilibili.com/video/BV1USERDEMO")
}
pub fn valid_media_manifest(dir: &Path) -> bool {
    let p = dir.join("media.json");
    let Ok(raw) = fs::read_to_string(p) else {
        return false;
    };
    let Ok(m) = serde_json::from_str::<media::MediaPreparationResult>(&raw) else {
        return false;
    };
    m.video_file
        .as_deref()
        .is_some_and(|p| Path::new(p).is_file())
        && !m.chunks.is_empty()
        && m.chunks.iter().all(|c| Path::new(&c.path).is_file())
}
pub fn valid_transcript(dir: &Path) -> bool {
    let Ok(raw) = fs::read_to_string(dir.join("transcript/transcript.json")) else {
        return false;
    };
    serde_json::from_str::<Value>(&raw)
        .ok()
        .and_then(|v| {
            v.get("segments")
                .and_then(|s| s.as_array())
                .map(|a| !a.is_empty())
        })
        .unwrap_or(false)
}

fn valid_json_file(path: &Path) -> bool {
    fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .is_some()
}

fn transcript_has_translation(dir: &Path) -> bool {
    fs::read_to_string(dir.join("transcript/transcript.json"))
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .and_then(|value| value.get("segments").and_then(Value::as_array).cloned())
        .is_some_and(|segments| {
            segments.iter().any(|segment| {
                segment
                    .get("translatedText")
                    .and_then(Value::as_str)
                    .is_some_and(|text| !text.trim().is_empty())
            })
        })
}

use serde_json::Value;

const VIDEO_COLUMNS: &str = "id,title,platform,duration,source_url,normalized_source_key,author,thumbnail_url,created_at,updated_at,library_available_at,deleted_at";

fn hydrate_video(
    db: &Connection,
    task_root: &Path,
    mut video: VideoRecord,
) -> rusqlite::Result<VideoRecord> {
    let mut artifacts = db.prepare("SELECT artifact_type,state FROM artifacts WHERE video_id=?1")?;
    for record in artifacts.query_map([&video.id], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })? {
        let (kind, state) = record?;
        match kind.as_str() {
            "standard_transcript" => video.transcript_status = state,
            "translation" => video.translation_status = state,
            "note" => video.note_status = state,
            _ => {}
        }
    }
    video.queue_item_id = db
        .query_row(
            "SELECT id FROM queue_items WHERE video_id=?1 AND state NOT IN ('cancelled','completed') ORDER BY position LIMIT 1",
            [&video.id],
            |r| r.get(0),
        )
        .optional()?;
    let dir = task_root.join("tasks").join(&video.id);
    video.media_status = if valid_media_manifest(&dir) {
        "available".into()
    } else {
        "missing".into()
    };
    if video.transcript_status == "ready" && !valid_transcript(&dir) {
        video.transcript_status = "failed".into();
    }
    if video.translation_status == "ready"
        && !valid_json_file(&dir.join("translation/translation.json"))
    {
        video.translation_status = "missing".into();
    }
    if video.note_status == "ready" && !valid_json_file(&dir.join("note/note.json")) {
        video.note_status = "missing".into();
    }
    if let Ok(raw) = fs::read_to_string(dir.join("transcript/transcript.json")) {
        if let Ok(value) = serde_json::from_str::<Value>(&raw) {
            video.transcript_language = value
                .get("language")
                .and_then(|v| v.as_str())
                .map(str::to_string);
        }
    }
    Ok(video)
}

fn video_filter_clause() -> &'static str {
    "library_available_at IS NOT NULL AND deleted_at IS NULL AND (?1 = '' OR instr(lower(title), lower(?1)) > 0 OR instr(lower(COALESCE(author, '')), lower(?1)) > 0 OR instr(lower(source_url), lower(?1)) > 0) AND (?2 = '' OR lower(platform) = lower(?2))"
}

pub fn list_videos_page(
    db: &Connection,
    task_root: &Path,
    query: Option<&str>,
    platform: Option<&str>,
    page: Option<u32>,
    page_size: Option<u32>,
) -> rusqlite::Result<VideoPage> {
    let query = query.unwrap_or_default().trim();
    let platform = platform.unwrap_or_default().trim();
    let page = page.unwrap_or(1).max(1);
    let page_size = page_size.unwrap_or(20).clamp(1, 100);
    let total: u64 = db.query_row(
        &format!("SELECT COUNT(*) FROM videos WHERE {}", video_filter_clause()),
        params![query, platform],
        |r| r.get(0),
    )?;
    let offset = u64::from(page - 1).saturating_mul(u64::from(page_size));
    let mut statement = db.prepare(&format!(
        "SELECT {} FROM videos WHERE {} ORDER BY updated_at DESC, id DESC LIMIT ?3 OFFSET ?4",
        VIDEO_COLUMNS,
        video_filter_clause(),
    ))?;
    let rows = statement
        .query_map(params![query, platform, page_size, offset], row_video)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let items = rows
        .into_iter()
        .map(|video| hydrate_video(db, task_root, video))
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(VideoPage {
        items,
        total,
        page,
        page_size,
    })
}

pub fn get_video(
    db: &Connection,
    task_root: &Path,
    video_id: &str,
) -> rusqlite::Result<Option<VideoRecord>> {
    let video = db
        .query_row(
            &format!(
                "SELECT {} FROM videos WHERE id=?1 AND library_available_at IS NOT NULL AND deleted_at IS NULL",
                VIDEO_COLUMNS
            ),
            [video_id],
            row_video,
        )
        .optional()?;
    video
        .map(|record| hydrate_video(db, task_root, record))
        .transpose()
}

pub fn lookup_videos_by_sources(
    db: &Connection,
    task_root: &Path,
    sources: &[VideoSourceLookupInput],
) -> rusqlite::Result<Vec<VideoSourceLookup>> {
    if sources.len() > 100 {
        return Err(rusqlite::Error::InvalidParameterName(
            "sources must contain at most 100 items".into(),
        ));
    }
    sources
        .iter()
        .map(|source| {
            let key = normalize_source_key(&source.platform, &source.source_url);
            let video = db
                .query_row(
                    &format!("SELECT {} FROM videos WHERE normalized_source_key=?1 AND library_available_at IS NOT NULL AND deleted_at IS NULL", VIDEO_COLUMNS),
                    [&key],
                    row_video,
                )
                .optional()?;
            Ok(VideoSourceLookup {
                platform: source.platform.clone(),
                source_url: source.source_url.clone(),
                video: video
                    .map(|record| hydrate_video(db, task_root, record))
                    .transpose()?,
            })
        })
        .collect()
}
pub fn list_queue(db: &Connection) -> rusqlite::Result<Vec<QueueItem>> {
    let mut s=db.prepare("SELECT q.id,q.video_id,q.position,q.state,q.stage,q.progress,q.phase_completed,q.phase_total,q.phase_unit,q.attempt_count,q.asr_backend,q.asr_config_json,q.error_code,q.error_message,q.created_at,q.started_at,q.updated_at,q.finished_at,q.status_message FROM queue_items q WHERE q.state NOT IN ('cancelled') ORDER BY q.position,q.created_at")?;
    let mut items = s
        .query_map([], row_queue)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for item in &mut items {
        if let Ok(video)=db.query_row("SELECT title,platform,duration,source_url,author,thumbnail_url FROM videos WHERE id=?1",[&item.video_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get(4)?,r.get(5)?))) { item.title=video.0; item.platform=video.1; item.duration=video.2; item.source_url=video.3; item.author=video.4; item.thumbnail_url=video.5; }
    }
    Ok(items)
}
pub fn enqueue(
    db: &Connection,
    inputs: Vec<EnqueueInput>,
) -> rusqlite::Result<Vec<EnqueueOutcome>> {
    let tx = db.unchecked_transaction()?;
    let mut outcomes = Vec::new();
    for input in inputs {
        let key = normalize_source_key(&input.platform, &input.source_url);
        if let Some(video) = tx
            .query_row(
                "SELECT id,title,platform,duration,source_url,normalized_source_key,author,thumbnail_url,created_at,updated_at,library_available_at,deleted_at FROM videos WHERE normalized_source_key=?1",
                [&key],
                row_video,
            )
            .optional()?
        {
            let active = tx
                .query_row(
                    "SELECT id,video_id,position,state,stage,progress,phase_completed,phase_total,phase_unit,attempt_count,asr_backend,asr_config_json,error_code,error_message,created_at,started_at,updated_at,finished_at,status_message FROM queue_items WHERE video_id=?1 AND state NOT IN ('cancelled','completed') LIMIT 1",
                    [&video.id],
                    row_queue,
                )
                .optional()?;
            let (outcome, queue_item) = if video.library_available_at.is_some() {
                ("alreadyInLibrary", None)
            } else if let Some(queue_item) = active {
                ("alreadyQueued", Some(queue_item))
            } else {
                ("queueItem", Some(create_queue_item(&tx, &video.id, "download", "等待处理", input.asr_backend.as_deref(), input.asr_config_json.as_deref())?))
            };
            outcomes.push(EnqueueOutcome {
                source_url: input.source_url,
                normalized_source_key: key,
                outcome: outcome.into(),
                video,
                queue_item,
            });
            continue;
        }

        let video_id = id();
        let stamp = now();
        tx.execute(
            "INSERT INTO videos(id,title,platform,duration,source_url,normalized_source_key,author,thumbnail_url,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?9)",
            params![video_id, input.title, input.platform, input.duration, input.source_url, key, input.author.as_deref(), input.thumbnail_url.as_deref(), stamp],
        )?;
        let video = tx.query_row(
            "SELECT id,title,platform,duration,source_url,normalized_source_key,author,thumbnail_url,created_at,updated_at,library_available_at,deleted_at FROM videos WHERE id=?1",
            [&video_id],
            row_video,
        )?;
        let queue_item = create_queue_item(
            &tx,
            &video_id,
            "download",
            "等待处理",
            input.asr_backend.as_deref(),
            input.asr_config_json.as_deref(),
        )?;
        outcomes.push(EnqueueOutcome {
            source_url: input.source_url,
            normalized_source_key: key,
            outcome: "queueItem".into(),
            video,
            queue_item: Some(queue_item),
        });
    }
    tx.commit()?;
    Ok(outcomes)
}

fn create_queue_item(
    db: &Connection,
    video_id: &str,
    stage: &str,
    message: &str,
    asr_backend: Option<&str>,
    asr_config_json: Option<&str>,
) -> rusqlite::Result<QueueItem> {
    let position: i64 = db.query_row(
        "SELECT COALESCE(MAX(position),0)+1 FROM queue_items",
        [],
        |row| row.get(0),
    )?;
    let stamp = now();
    let queue_id = format!("queue-{video_id}-{}", id());
    db.execute(
        "INSERT INTO queue_items(id,video_id,position,state,stage,progress,attempt_count,status_message,created_at,updated_at,asr_backend,asr_config_json) VALUES(?1,?2,?3,'queued',?4,0,0,?5,?6,?6,?7,?8)",
        params![queue_id, video_id, position, stage, message, stamp, asr_backend, asr_config_json],
    )?;
    db.query_row(
        "SELECT id,video_id,position,state,stage,progress,phase_completed,phase_total,phase_unit,attempt_count,asr_backend,asr_config_json,error_code,error_message,created_at,started_at,updated_at,finished_at,status_message FROM queue_items WHERE id=?1",
        [queue_id],
        row_queue,
    )
}

pub fn queue_command(db: &Connection, id: &str, action: &str) -> rusqlite::Result<()> {
    match action {
        "pause" => {
            db.execute("UPDATE queue_items SET state='paused',status_message='已暂停',updated_at=?1 WHERE id=?2 AND state IN ('queued','running')",params![now(),id])?;
        }
        "resume" => {
            db.execute("UPDATE queue_items SET state='queued',error_code=NULL,error_message=NULL,status_message='等待继续处理',updated_at=?1 WHERE id=?2 AND state IN ('paused','blocked','failed')",params![now(),id])?;
        }
        "remove" => {
            db.execute("UPDATE queue_items SET state='cancelled',status_message='已从队列移除',updated_at=?1 WHERE id=?2 AND state NOT IN ('running')",params![now(),id])?;
        }
        _ => {
            return Err(rusqlite::Error::InvalidParameterName(
                "unknown queue action".into(),
            ))
        }
    }
    Ok(())
}

pub fn retry_queue(db: &Connection, task_dir: &Path, id: &str) -> Result<(), String> {
    let (video_id, state) = db
        .query_row(
            "SELECT video_id,state FROM queue_items WHERE id=?1",
            [id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .map_err(|error| error.to_string())?;
    if !matches!(state.as_str(), "failed" | "blocked" | "cancelled") {
        return Err("只有失败、受阻或已移除的队列项可以重试".into());
    }
    let stage = if valid_media_manifest(&task_dir.join("tasks").join(video_id)) {
        "transcribe"
    } else {
        "download"
    };
    db.execute(
        "UPDATE queue_items SET state='queued',stage=?1,progress=0,phase_completed=0,phase_total=100,error_code=NULL,error_message=NULL,status_message='等待重试',updated_at=?2,finished_at=NULL WHERE id=?3",
        params![stage, now(), id],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

pub fn has_live_queue(db: &Connection, video_id: &str) -> rusqlite::Result<bool> {
    db.query_row(
        "SELECT EXISTS(SELECT 1 FROM queue_items WHERE video_id=?1 AND state NOT IN ('cancelled','completed'))",
        [video_id],
        |row| row.get(0),
    )
}

pub fn requeue_video(
    db: &Connection,
    task_dir: &Path,
    video_id: &str,
    asr_backend: Option<&str>,
    asr_config_json: Option<&str>,
) -> Result<QueueItem, String> {
    if has_live_queue(db, video_id).map_err(|error| error.to_string())? {
        return Err("该视频已经在队列中".into());
    }
    let stamp = now();
    let tx = db
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    let changed = tx
        .execute(
            "UPDATE videos SET library_available_at=NULL,updated_at=?1 WHERE id=?2 AND library_available_at IS NOT NULL",
            params![stamp, video_id],
        )
        .map_err(|error| error.to_string())?;
    if changed == 0 {
        return Err("视频库中找不到该视频".into());
    }
    tx.execute(
        "UPDATE artifacts SET state=CASE WHEN artifact_type='standard_transcript' THEN 'processing' ELSE 'stale' END,updated_at=?1 WHERE video_id=?2 AND artifact_type IN ('standard_transcript','translation','note')",
        params![stamp, video_id],
    )
    .map_err(|error| error.to_string())?;
    let stage = if valid_media_manifest(&task_dir.join("tasks").join(video_id)) {
        "transcribe"
    } else {
        "download"
    };
    let queue_item = create_queue_item(
        &tx,
        video_id,
        stage,
        "等待重新转录",
        asr_backend,
        asr_config_json,
    )
    .map_err(|error| error.to_string())?;
    tx.commit().map_err(|error| error.to_string())?;
    Ok(queue_item)
}
pub fn move_queue(db: &Connection, id: &str, direction: &str) -> rusqlite::Result<()> {
    if !matches!(direction, "up" | "down" | "top") {
        return Err(rusqlite::Error::InvalidParameterName(
            "unknown queue direction".into(),
        ));
    }
    let Some((pos, state)) = db
        .query_row(
            "SELECT position,state FROM queue_items WHERE id=?1",
            [id],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
        )
        .optional()?
    else {
        return Ok(());
    };
    if state != "queued" {
        return Ok(());
    }
    if direction == "top" {
        let min: i64 = db.query_row("SELECT COALESCE(MIN(position),0) FROM queue_items WHERE state NOT IN ('cancelled','completed')", [], |r| r.get(0))?;
        db.execute(
            "UPDATE queue_items SET position=?1,updated_at=?2 WHERE id=?3",
            params![min - 1, now(), id],
        )?;
        return Ok(());
    }
    let target = if direction == "up" {
        db.query_row("SELECT id,position FROM queue_items WHERE state NOT IN ('cancelled','completed') AND position<?1 ORDER BY position DESC LIMIT 1", [pos], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))).optional()?
    } else {
        db.query_row("SELECT id,position FROM queue_items WHERE state NOT IN ('cancelled','completed') AND position>?1 ORDER BY position ASC LIMIT 1", [pos], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))).optional()?
    };
    if let Some((other, target_position)) = target {
        let tx = db.unchecked_transaction()?;
        tx.execute("UPDATE queue_items SET position=-1 WHERE id=?1", [id])?;
        tx.execute(
            "UPDATE queue_items SET position=?1,updated_at=?2 WHERE id=?3",
            params![pos, now(), other],
        )?;
        tx.execute(
            "UPDATE queue_items SET position=?1,updated_at=?2 WHERE id=?3",
            params![target_position, now(), id],
        )?;
        tx.commit()?;
    }
    Ok(())
}

pub fn mark_translation(
    task_dir: &Path,
    video_id: &str,
    segment_id: &str,
    text: &str,
) -> Result<(), String> {
    let mut transcript = asr::load_transcript(task_dir, video_id).map_err(|e| e.to_string())?;
    let segment = transcript
        .segments
        .iter_mut()
        .find(|s| s.id == segment_id)
        .ok_or("找不到该转录段")?;
    segment.translated_text = Some(translation::clean_milmmt_translation_output(text));
    asr::save_transcript(task_dir, video_id, &transcript).map_err(|e| e.to_string())?;
    let dir = task_dir.join("tasks").join(video_id).join("translation");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let data = serde_json::json!({"videoId":video_id,"sourceRevision":transcript_hash(&transcript),"segments":transcript.segments.iter().filter_map(|s|s.translated_text.as_ref().map(|t|serde_json::json!({"id":s.id,"text":translation::clean_milmmt_translation_output(t)}))).collect::<Vec<_>>()});
    fs::write(
        dir.join("translation.json"),
        serde_json::to_vec_pretty(&data).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
pub fn write_translation_artifact(task_dir: &Path, video_id: &str) -> Result<(), String> {
    let transcript = asr::load_transcript(task_dir, video_id).map_err(|e| e.to_string())?;
    let dir = task_dir.join("tasks").join(video_id).join("translation");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let data = serde_json::json!({"videoId":video_id,"sourceRevision":transcript_hash(&transcript),"segments":transcript.segments.iter().filter_map(|s|s.translated_text.as_ref().map(|t|serde_json::json!({"id":s.id,"text":translation::clean_milmmt_translation_output(t)}))).collect::<Vec<_>>()});
    fs::write(
        dir.join("translation.json"),
        serde_json::to_vec_pretty(&data).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
pub fn mark_derived_stale(db: &Connection, video_id: &str) -> Result<(), String> {
    let stamp = now();
    db.execute("UPDATE artifacts SET state='stale',input_revision=revision,updated_at=?1 WHERE video_id=?2 AND artifact_type IN ('translation','note') AND state='ready'",params![stamp,video_id]).map_err(|e|e.to_string())?;
    Ok(())
}
pub fn mark_note_stale(db: &Connection, video_id: &str) -> Result<(), String> {
    db.execute("UPDATE artifacts SET state='stale',input_revision=revision,updated_at=?1 WHERE video_id=?2 AND artifact_type='note' AND state='ready'",params![now(),video_id]).map_err(|e|e.to_string())?;
    Ok(())
}
pub fn mark_artifact_ready(
    db: &Connection,
    video_id: &str,
    artifact_type: &str,
    path: &str,
) -> Result<(), String> {
    let stamp = now();
    db.execute("INSERT INTO artifacts(video_id,artifact_type,state,relative_path,revision,updated_at) VALUES(?1,?2,'ready',?3,1,?4) ON CONFLICT(video_id,artifact_type) DO UPDATE SET state='ready',relative_path=excluded.relative_path,revision=artifacts.revision+1,updated_at=excluded.updated_at",params![video_id,artifact_type,path,stamp]).map_err(|e|e.to_string())?;
    Ok(())
}
fn transcript_hash(t: &asr::TranscriptResult) -> String {
    let source = t
        .segments
        .iter()
        .map(|s| (&s.id, &s.start_ms, &s.end_ms, &s.text))
        .collect::<Vec<_>>();
    let bytes = serde_json::to_vec(&source).unwrap_or_default();
    format!("{:x}", Sha256::digest(bytes))
}

pub fn delete_results(db: &mut Connection, task_dir: &Path, video_id: &str) -> Result<(), String> {
    let video_dir = task_dir.join("tasks").join(video_id);
    let trash_dir = task_dir
        .join(".trash")
        .join(format!("results-{video_id}-{}", id()));
    fs::create_dir_all(&trash_dir).map_err(|error| error.to_string())?;

    let mut moved = Vec::new();
    for relative in ["transcript", "translation", "note", "moss_checkpoint.json"] {
        let source = video_dir.join(relative);
        if !source.exists() {
            continue;
        }
        let target = trash_dir.join(relative);
        if let Err(error) = fs::rename(&source, &target) {
            for (original, trashed) in moved.iter().rev() {
                let _ = fs::rename(trashed, original);
            }
            let _ = fs::remove_dir_all(&trash_dir);
            return Err(error.to_string());
        }
        moved.push((source, target));
    }

    let transaction = db.transaction().map_err(|error| error.to_string())?;
    let database_result = (|| {
        transaction
            .execute(
                "UPDATE videos SET library_available_at=NULL,updated_at=?1 WHERE id=?2",
                params![now(), video_id],
            )
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "DELETE FROM artifacts WHERE video_id=?1 AND artifact_type NOT IN ('source_media','prepared_audio')",
                [video_id],
            )
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())
    })();

    if database_result.is_err() {
        for (original, trashed) in moved.iter().rev() {
            let _ = fs::rename(trashed, original);
        }
    } else {
        let _ = fs::remove_dir_all(&trash_dir);
    }
    database_result
}

pub fn delete_completely(
    db: &mut Connection,
    task_dir: &Path,
    video_id: &str,
) -> Result<(), String> {
    let video_dir = task_dir.join("tasks").join(video_id);
    if !video_dir.exists() {
        db.execute("DELETE FROM videos WHERE id=?1", [video_id])
            .map_err(|error| error.to_string())?;
        return Ok(());
    }

    let trash_root = task_dir.join(".trash");
    fs::create_dir_all(&trash_root).map_err(|error| error.to_string())?;
    let trash_dir = trash_root.join(format!("video-{video_id}-{}", id()));
    fs::rename(&video_dir, &trash_dir).map_err(|error| error.to_string())?;

    let transaction = db.transaction().map_err(|error| error.to_string())?;
    let database_result = (|| {
        transaction
            .execute("DELETE FROM videos WHERE id=?1", [video_id])
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())
    })();
    if database_result.is_err() {
        let _ = fs::rename(&trash_dir, &video_dir);
    } else {
        let _ = fs::remove_dir_all(&trash_dir);
    }
    database_result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalizes_bv() {
        assert_eq!(
            normalize_source_key(
                "bilibili",
                "https://www.bilibili.com/video/BV1ABCDEF12/?p=1"
            ),
            "bilibili:bv1abcdef12"
        );
    }
    #[test]
    fn migration_does_not_seed_demo() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE jobs(id TEXT PRIMARY KEY,title TEXT,platform TEXT,duration TEXT,updated_at TEXT,status TEXT,progress INTEGER,source_url TEXT,thumbnail_url TEXT,error_message TEXT,asr_backend TEXT,asr_config_json TEXT);").unwrap();
        db.execute("INSERT INTO jobs VALUES('rag-overview','demo','bilibili','1:00','x','completed',100,'https://www.bilibili.com/video/BV1RAGDEMO',NULL,NULL,NULL,NULL)",[]).unwrap();
        initialize_database(&db, Path::new(".")).unwrap();
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM videos", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    #[test]
    fn queue_order_skips_paused() {
        let db = Connection::open_in_memory().unwrap();
        initialize_database(&db, Path::new(".")).unwrap();
        let rows = enqueue(
            &db,
            vec![
                EnqueueInput {
                    title: "a".into(),
                    platform: "bilibili".into(),
                    duration: "1".into(),
                    source_url: "https://x/a".into(),
                    author: None,
                    thumbnail_url: None,
                    asr_backend: None,
                    asr_config_json: None,
                },
                EnqueueInput {
                    title: "b".into(),
                    platform: "bilibili".into(),
                    duration: "1".into(),
                    source_url: "https://x/b".into(),
                    author: None,
                    thumbnail_url: None,
                    asr_backend: None,
                    asr_config_json: None,
                },
            ],
        )
        .unwrap();
        queue_command(
            &db,
            rows[0].queue_item.as_ref().unwrap().id.as_str(),
            "pause",
        )
        .unwrap();
        assert_eq!(
            db.query_row(
                "SELECT state FROM queue_items ORDER BY position LIMIT 1",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "paused"
        );
    }

    #[test]
    fn migration_keeps_valid_transcript_without_media() {
        let temp = tempfile::tempdir().unwrap();
        let transcript_dir = temp.path().join("tasks/legacy/transcript");
        fs::create_dir_all(&transcript_dir).unwrap();
        fs::write(
            transcript_dir.join("transcript.json"),
            r#"{"segments":[{"id":"s1","text":"kept"}]}"#,
        )
        .unwrap();
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE jobs(id TEXT PRIMARY KEY,title TEXT,platform TEXT,duration TEXT,updated_at TEXT,status TEXT,progress INTEGER,source_url TEXT,thumbnail_url TEXT,error_message TEXT,asr_backend TEXT,asr_config_json TEXT);").unwrap();
        db.execute("INSERT INTO jobs VALUES('legacy','kept','bilibili','1:00','123','completed',100,'https://www.bilibili.com/video/BV1REAL12345',NULL,NULL,NULL,NULL)", []).unwrap();

        initialize_database(&db, temp.path()).unwrap();

        let available: Option<String> = db
            .query_row(
                "SELECT library_available_at FROM videos WHERE id='legacy'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(available.is_some());
        let page = list_videos_page(&db, temp.path(), None, None, None, None).unwrap();
        assert_eq!(page.items[0].media_status, "missing");
        assert_eq!(page.items[0].transcript_status, "ready");
    }

    #[test]
    fn move_queue_skips_completed_position_gaps() {
        let db = Connection::open_in_memory().unwrap();
        initialize_database(&db, Path::new(".")).unwrap();
        let rows = enqueue(
            &db,
            (0..3)
                .map(|index| EnqueueInput {
                    title: format!("video-{index}"),
                    platform: "bilibili".into(),
                    duration: "1".into(),
                    source_url: format!("https://example.com/{index}"),
                    author: None,
                    thumbnail_url: None,
                    asr_backend: None,
                    asr_config_json: None,
                })
                .collect(),
        )
        .unwrap();
        let first = rows[0].queue_item.as_ref().unwrap().id.clone();
        let middle = rows[1].queue_item.as_ref().unwrap().id.clone();
        let last = rows[2].queue_item.as_ref().unwrap().id.clone();
        db.execute(
            "UPDATE queue_items SET state='completed' WHERE id=?1",
            [&middle],
        )
        .unwrap();

        move_queue(&db, &last, "up").unwrap();

        let ordered = db
            .prepare("SELECT id FROM queue_items WHERE state!='completed' ORDER BY position")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(ordered, vec![last, first]);
    }

    #[test]
    fn retry_does_not_increment_attempt_until_scheduler_starts() {
        let temp = tempfile::tempdir().unwrap();
        let db = Connection::open_in_memory().unwrap();
        initialize_database(&db, temp.path()).unwrap();
        let rows = enqueue(
            &db,
            vec![EnqueueInput {
                title: "video".into(),
                platform: "bilibili".into(),
                duration: "1".into(),
                source_url: "https://example.com/retry".into(),
                author: None,
                thumbnail_url: None,
                asr_backend: None,
                asr_config_json: None,
            }],
        )
        .unwrap();
        let queue_id = &rows[0].queue_item.as_ref().unwrap().id;
        db.execute(
            "UPDATE queue_items SET state='failed' WHERE id=?1",
            [queue_id],
        )
        .unwrap();

        retry_queue(&db, temp.path(), queue_id).unwrap();

        let attempt: u32 = db
            .query_row(
                "SELECT attempt_count FROM queue_items WHERE id=?1",
                [queue_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(attempt, 0);
    }

    #[test]
    fn enqueue_again_after_result_deletion_creates_a_new_queue_item() {
        let temp = tempfile::tempdir().unwrap();
        let mut db = Connection::open_in_memory().unwrap();
        initialize_database(&db, temp.path()).unwrap();
        let input = EnqueueInput {
            title: "video".into(),
            platform: "bilibili".into(),
            duration: "1".into(),
            source_url: "https://example.com/re-add".into(),
            author: None,
            thumbnail_url: None,
            asr_backend: Some("openasr-moss-q4".into()),
            asr_config_json: Some("{\"chunkSeconds\":30,\"overlapSeconds\":1}".into()),
        };
        let first = enqueue(&db, vec![input.clone()]).unwrap();
        let video_id = first[0].video.id.clone();
        let queue_id = first[0].queue_item.as_ref().unwrap().id.clone();
        db.execute(
            "UPDATE queue_items SET state='completed' WHERE id=?1",
            [&queue_id],
        )
        .unwrap();
        db.execute(
            "UPDATE videos SET library_available_at='1' WHERE id=?1",
            [&video_id],
        )
        .unwrap();

        delete_results(&mut db, temp.path(), &video_id).unwrap();
        let second = enqueue(&db, vec![input]).unwrap();

        assert_eq!(second[0].outcome, "queueItem");
        assert!(second[0].queue_item.is_some());
        assert_ne!(second[0].queue_item.as_ref().unwrap().id, queue_id);
        assert_eq!(
            second[0]
                .queue_item
                .as_ref()
                .unwrap()
                .asr_backend
                .as_deref(),
            Some("openasr-moss-q4")
        );
    }

    fn seed_library_videos(db: &Connection, count: usize) -> Vec<String> {
        let inputs = (0..count)
            .map(|index| EnqueueInput {
                title: format!("Library video {index}"),
                platform: if index % 2 == 0 {
                    "bilibili".into()
                } else {
                    "douyin".into()
                },
                duration: "1:00".into(),
                source_url: format!("https://example.com/library/{index}"),
                author: Some(format!("Author {index}")),
                thumbnail_url: None,
                asr_backend: None,
                asr_config_json: None,
            })
            .collect::<Vec<_>>();
        let rows = enqueue(db, inputs).unwrap();
        for (index, row) in rows.iter().enumerate() {
            db.execute(
                "UPDATE videos SET library_available_at='ready',updated_at=?1 WHERE id=?2",
                params![format!("{index:04}"), row.video.id],
            )
            .unwrap();
        }
        rows.into_iter().map(|row| row.video.id).collect()
    }

    #[test]
    fn video_page_reports_total_and_respects_page_boundaries() {
        let temp = tempfile::tempdir().unwrap();
        let db = Connection::open_in_memory().unwrap();
        initialize_database(&db, temp.path()).unwrap();
        let index_sql: String = db
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='index' AND name='videos_library_order'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(index_sql.contains("WHERE library_available_at IS NOT NULL"));
        seed_library_videos(&db, 5);

        let first = list_videos_page(&db, temp.path(), None, None, Some(1), Some(2)).unwrap();
        assert_eq!(first.total, 5);
        assert_eq!(first.page, 1);
        assert_eq!(first.page_size, 2);
        assert_eq!(first.items.len(), 2);
        assert_eq!(first.items[0].title, "Library video 4");

        let last = list_videos_page(&db, temp.path(), None, None, Some(3), Some(2)).unwrap();
        assert_eq!(last.items.len(), 1);
        let beyond = list_videos_page(&db, temp.path(), None, None, Some(4), Some(2)).unwrap();
        assert!(beyond.items.is_empty());
        assert_eq!(beyond.total, 5);
        let normalized = list_videos_page(&db, temp.path(), None, None, Some(0), Some(500)).unwrap();
        assert_eq!(normalized.page, 1);
        assert_eq!(normalized.page_size, 100);
    }

    #[test]
    fn video_page_searches_title_author_and_source_and_filters_platform() {
        let temp = tempfile::tempdir().unwrap();
        let db = Connection::open_in_memory().unwrap();
        initialize_database(&db, temp.path()).unwrap();
        seed_library_videos(&db, 4);

        let title = list_videos_page(&db, temp.path(), Some("video 2"), None, None, None).unwrap();
        assert_eq!(title.total, 1);
        let author = list_videos_page(&db, temp.path(), Some("AUTHOR 3"), None, None, None).unwrap();
        assert_eq!(author.total, 1);
        let source = list_videos_page(&db, temp.path(), Some("library/1"), None, None, None).unwrap();
        assert_eq!(source.total, 1);
        let platform = list_videos_page(&db, temp.path(), None, Some("DOUYIN"), None, None).unwrap();
        assert_eq!(platform.total, 2);
        assert!(platform.items.iter().all(|video| video.platform == "douyin"));
    }

    #[test]
    fn source_lookup_matches_only_library_videos_and_preserves_input_order() {
        let temp = tempfile::tempdir().unwrap();
        let db = Connection::open_in_memory().unwrap();
        initialize_database(&db, temp.path()).unwrap();
        seed_library_videos(&db, 2);
        let sources = vec![
            VideoSourceLookupInput {
                platform: "bilibili".into(),
                source_url: "https://example.com/library/0?from=search".into(),
            },
            VideoSourceLookupInput {
                platform: "bilibili".into(),
                source_url: "https://example.com/not-in-library".into(),
            },
        ];
        let found = lookup_videos_by_sources(&db, temp.path(), &sources).unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].source_url, sources[0].source_url);
        assert_eq!(found[0].video.as_ref().unwrap().title, "Library video 0");
        assert!(found[1].video.is_none());
    }

    #[test]
    fn source_lookup_rejects_more_than_one_hundred_items() {
        let db = Connection::open_in_memory().unwrap();
        initialize_database(&db, Path::new(".")).unwrap();
        let sources = (0..101)
            .map(|index| VideoSourceLookupInput {
                platform: "bilibili".into(),
                source_url: format!("https://example.com/{index}"),
            })
            .collect::<Vec<_>>();
        assert!(lookup_videos_by_sources(&db, Path::new("."), &sources).is_err());
    }

    #[test]
    fn get_video_is_limited_to_library_records() {
        let temp = tempfile::tempdir().unwrap();
        let db = Connection::open_in_memory().unwrap();
        initialize_database(&db, temp.path()).unwrap();
        let queued = enqueue(
            &db,
            vec![EnqueueInput {
                title: "Not yet in library".into(),
                platform: "bilibili".into(),
                duration: "1:00".into(),
                source_url: "https://example.com/queued".into(),
                author: None,
                thumbnail_url: None,
                asr_backend: None,
                asr_config_json: None,
            }],
        )
        .unwrap();
        assert!(get_video(&db, temp.path(), &queued[0].video.id)
            .unwrap()
            .is_none());
        db.execute(
            "UPDATE videos SET library_available_at='ready' WHERE id=?1",
            [&queued[0].video.id],
        )
        .unwrap();
        assert_eq!(
            get_video(&db, temp.path(), &queued[0].video.id)
                .unwrap()
                .unwrap()
                .title,
            "Not yet in library"
        );
    }
}

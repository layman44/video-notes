use futures_util::StreamExt;
use reqwest::Client;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::AsyncWriteExt;

pub const MODEL_ID: &str = "moss-transcribe-diarize:q4";
pub const MODEL_NAME: &str = "MOSS-Transcribe-Diarize 0.9B q4（OpenASR）";
pub const MODEL_SIZE_LABEL: &str = "约 860 MiB";
const MODEL_URL: &str = "https://huggingface.co/OpenASR/moss-transcribe-diarize/resolve/196b6d4939c334ff41559db2549f1432899f8822/moss-transcribe-diarize-q4_k.oasr";
const MODEL_SHA256: &str = "0044546efb95d4d08e85f5574da2b042a5a4fb2490678c666b65404f1ac94c04";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenAsrModelStatus {
    pub id: String,
    pub name: String,
    pub installed: bool,
    pub file_size: Option<u64>,
    pub size_label: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadProgress {
    model_id: String,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
    progress: u8,
    message: String,
}

fn emit_progress(app: &AppHandle, downloaded: u64, total: Option<u64>, progress: u8, message: impl Into<String>) {
    let _ = app.emit(
        "model-download-progress",
        DownloadProgress {
            model_id: MODEL_ID.into(),
            downloaded_bytes: downloaded,
            total_bytes: total,
            progress: progress.min(100),
            message: message.into(),
        },
    );
}

pub fn model_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_local_data_dir()
        .map(|dir| dir.join("openasr").join("moss-transcribe-diarize-q4_k.oasr"))
        .map_err(|error| format!("无法获取 OpenASR 模型目录：{error}"))
}

pub fn runtime_path(app: &AppHandle) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(path) = std::env::var("VIDEO_NOTES_OPENASR_PATH") {
        candidates.push(PathBuf::from(path));
    }
    if let Ok(resource) = app.path().resource_dir() {
        candidates.push(resource.join("resources").join("tools").join("openasr").join("openasr.exe"));
        candidates.push(resource.join("tools").join("openasr").join("openasr.exe"));
        candidates.push(resource.join("tools").join("openasr").join("openasr-0.1.30-windows-x86_64").join("openasr.exe"));
        candidates.push(resource.join("tools").join("openasr.exe"));
        candidates.push(resource.join("openasr.exe"));
    }
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join("tools")
            .join("openasr")
            .join("openasr.exe"),
    );
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources").join("tools").join("openasr.exe"));
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join("tools")
            .join("openasr")
            .join("openasr-0.1.30-windows-x86_64")
            .join("openasr.exe"),
    );
    candidates.into_iter().find(|path| path.is_file())
}

pub fn model_status(app: &AppHandle) -> OpenAsrModelStatus {
    let path = model_path(app).unwrap_or_else(|_| PathBuf::from("models/moss-transcribe-diarize-q4_k.oasr"));
    let file_size = fs::metadata(&path).map(|meta| meta.len()).ok();
    OpenAsrModelStatus {
        id: MODEL_ID.into(),
        name: MODEL_NAME.into(),
        installed: runtime_path(app).is_some() && file_size.is_some(),
        file_size,
        size_label: MODEL_SIZE_LABEL.into(),
        path: path.to_string_lossy().into_owned(),
    }
}

pub async fn download_model(app: &AppHandle) -> Result<OpenAsrModelStatus, String> {
    if runtime_path(app).is_none() {
        return Err("OpenASR Runtime 不存在，请先运行 tools:fetch-openasr 安装本地运行时".into());
    }
    let destination = model_path(app)?;
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("无法创建 OpenASR 模型目录：{error}"))?;
    }
    let temporary = destination.with_extension("oasr.download");
    let client = Client::new();
    emit_progress(app, 0, None, 0, "正在下载 MOSS q4 模型……");
    let response = client
        .get(MODEL_URL)
        .send()
        .await
        .map_err(|error| format!("下载 MOSS q4 失败：{error}"))?
        .error_for_status()
        .map_err(|error| format!("下载 MOSS q4 失败：{error}"))?;
    let total = response.content_length();
    let mut stream = response.bytes_stream();
    let mut file = tokio::fs::File::create(&temporary)
        .await
        .map_err(|error| format!("无法创建模型临时文件：{error}"))?;
    let mut downloaded = 0u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| format!("读取模型下载流失败：{error}"))?;
        file.write_all(&chunk).await.map_err(|error| format!("保存模型失败：{error}"))?;
        downloaded += chunk.len() as u64;
        let percent = total.map(|size| ((downloaded as f64 / size.max(1) as f64) * 100.0).round() as u8).unwrap_or(0);
        emit_progress(app, downloaded, total, percent, "正在下载 MOSS q4 模型……");
    }
    file.flush().await.map_err(|error| format!("刷新模型文件失败：{error}"))?;
    verify_sha256(&temporary)?;
    fs::rename(&temporary, &destination).map_err(|error| format!("写入 MOSS q4 模型失败：{error}"))?;
    emit_progress(app, downloaded, total, 100, "MOSS q4 模型安装完成");
    Ok(model_status(app))
}

pub fn delete_model(app: &AppHandle) -> Result<(), String> {
    let path = model_path(app)?;
    if path.is_file() {
        fs::remove_file(path).map_err(|error| format!("无法删除 MOSS q4 模型：{error}"))?;
    }
    Ok(())
}

fn verify_sha256(path: &Path) -> Result<(), String> {
    let mut file = File::open(path).map_err(|error| format!("无法校验模型文件：{error}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| format!("读取模型校验失败：{error}"))?;
        if read == 0 { break; }
        hasher.update(&buffer[..read]);
    }
    let actual = format!("{:x}", hasher.finalize());
    if actual != MODEL_SHA256 {
        return Err(format!("MOSS q4 校验失败：期望 {MODEL_SHA256}，实际 {actual}"));
    }
    Ok(())
}

pub fn model_is_ready(app: &AppHandle) -> Result<(PathBuf, PathBuf), String> {
    let runtime = runtime_path(app).ok_or_else(|| "OpenASR Runtime 不存在，请先安装 OpenASR Runtime".to_string())?;
    let model = model_path(app)?;
    if !model.is_file() {
        return Err(format!("MODEL_NOT_INSTALLED:请先到“模型”页面下载 {MODEL_NAME}"));
    }
    Ok((runtime, model))
}

use bzip2::read::BzDecoder;
use futures_util::StreamExt;
use reqwest::{header, Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
};
use tauri::{ipc::Channel, AppHandle, Manager};
use tar::Archive;
use tokio::io::AsyncWriteExt;
use walkdir::WalkDir;
use zip::ZipArchive;

const RUNTIME_VERSION: &str = "runtime-llamacpp-v0.2.0";
const RUNTIME_X64_FILE: &str = "funasr-llamacpp-windows-x64.zip";
const RUNTIME_X64_SHA256: &str = "297c962346d7e30d7a7c2c860dfaab3ff07d01fddf15e6fc5212ca9545441a51";
const RUNTIME_AVX2_FILE: &str = "funasr-llamacpp-windows-x64-avx2.zip";
const RUNTIME_AVX2_SHA256: &str = "4db0f11f603c324a63545cd7009cdd45bb45576efe282cec22796b5fd42d8ea1";

const NANO_REPO: &str = "FunAudioLLM/Fun-ASR-Nano-GGUF";
const NANO_MODEL_FILE: &str = "qwen3-0.6b-q8_0.gguf";
const NANO_ENCODER_FILE: &str = "funasr-encoder-f16.gguf";
const NANO_EXE: &str = "llama-funasr-cli.exe";

const PARA_REPO: &str = "FunAudioLLM/Paraformer-GGUF";
const PARA_FILE: &str = "paraformer-q8.gguf";
const PARA_EXE: &str = "llama-funasr-paraformer.exe";

const VAD_FILE: &str = "fsmn-vad.gguf";
const VAD_HF_URL: &str = "https://huggingface.co/FunAudioLLM/fsmn-vad-GGUF/resolve/main/fsmn-vad.gguf?download=true";
const VAD_HF_MIRROR_URL: &str = "https://hf-mirror.com/FunAudioLLM/fsmn-vad-GGUF/resolve/main/fsmn-vad.gguf";



const SHERPA_VERSION: &str = "v1.13.6";
const SHERPA_ARCHIVE: &str = "sherpa-onnx-v1.13.6-win-x64-shared-MT-Release-no-tts.tar.bz2";
const SHERPA_C_API_DLL: &str = "sherpa-onnx-c-api.dll";

const PUNCT_ARCHIVE: &str = "sherpa-onnx-punct-ct-transformer-zh-en-vocab272727-2024-04-12-int8.tar.bz2";
const PUNCT_MODEL_FILE: &str = "model.int8.onnx";
const PUNCT_URL: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/punctuation-models/sherpa-onnx-punct-ct-transformer-zh-en-vocab272727-2024-04-12-int8.tar.bz2";
const PUNCT_MIRROR_URL: &str = "https://gh-proxy.com/https://github.com/k2-fsa/sherpa-onnx/releases/download/punctuation-models/sherpa-onnx-punct-ct-transformer-zh-en-vocab272727-2024-04-12-int8.tar.bz2";

const ALIGNMENT_ARCHIVE: &str = "sherpa-onnx-nemo-ctc-en-conformer-small.tar.bz2";
const ALIGNMENT_MODEL_FILE: &str = "model.int8.onnx";
const ALIGNMENT_TOKENS_FILE: &str = "tokens.txt";
const ALIGNMENT_URL: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-nemo-ctc-en-conformer-small.tar.bz2";
const ALIGNMENT_MIRROR_URL: &str = "https://gh-proxy.com/https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-nemo-ctc-en-conformer-small.tar.bz2";

const FFMPEG_FILE: &str = "ffmpeg-master-latest-win64-gpl-shared.zip";
const FFMPEG_URL: &str = "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl-shared.zip";
const FFMPEG_MIRROR_URL: &str = "https://gh-proxy.com/https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl-shared.zip";
const REVISION: &str = "master";

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeModelRequest {
    pub model_kind: String,
}

#[derive(Debug, Clone, Copy)]
struct ModelSpec {
    kind: &'static str,
    label: &'static str,
    repo: &'static str,
    file: &'static str,
    exe: &'static str,
    encoder_file: Option<&'static str>,
}

fn model_spec(kind: &str) -> Result<ModelSpec, String> {
    match kind {
        "nano" | "" => Ok(ModelSpec {
            kind: "nano",
            label: "Fun-ASR-Nano Q8_0 + Encoder",
            repo: NANO_REPO,
            file: NANO_MODEL_FILE,
            exe: NANO_EXE,
            encoder_file: Some(NANO_ENCODER_FILE),
        }),
        "paraformer" => Ok(ModelSpec {
            kind: "paraformer",
            label: "Paraformer Q8 GGUF",
            repo: PARA_REPO,
            file: PARA_FILE,
            exe: PARA_EXE,
            encoder_file: None,
        }),
        other => Err(format!("不支持的 FunASR 模式：{other}")),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct NativeFunAsrPaths {
    pub runtime_path: String,
    pub model_path: String,
    pub encoder_model_path: String,
    pub vad_model_path: String,
    pub punctuation_runtime_path: String,
    pub punctuation_model_path: String,
    pub alignment_model_path: String,
    pub alignment_tokens_path: String,
    pub ffmpeg_path: String,
    pub ffprobe_path: String,
    pub install_root: String,
    pub runtime_version: String,
    pub runtime_variant: String,
    pub model_kind: String,
    pub model_label: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeFunAsrStatus {
    pub ready: bool,
    pub runtime_ready: bool,
    pub model_ready: bool,
    pub vad_ready: bool,
    pub punctuation_ready: bool,
    pub alignment_ready: bool,
    pub ffmpeg_ready: bool,
    pub paths: NativeFunAsrPaths,
    pub missing: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "event",
    content = "data"
)]
pub enum NativeInstallEvent {
    Started { total_items: usize },
    ItemStarted {
        label: String,
        repo: String,
        index: usize,
        total_items: usize,
    },
    Progress {
        label: String,
        current_file: String,
        downloaded_bytes: u64,
        total_bytes: u64,
        index: usize,
        total_items: usize,
    },
    ItemFinished {
        label: String,
        path: String,
        index: usize,
        total_items: usize,
    },
    Finished { paths: NativeFunAsrPaths },
    Error { message: String },
}

#[derive(Debug, Clone)]
struct RepoFile {
    path: String,
    size: u64,
}

fn send(channel: &Channel<NativeInstallEvent>, event: NativeInstallEvent) -> Result<(), String> {
    channel
        .send(event)
        .map_err(|e| format!("发送原生运行时安装事件失败：{e}"))
}

fn install_root(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_local_data_dir()
        .map(|p| p.join("native-funasr-gguf"))
        .map_err(|e| format!("无法获取应用本地数据目录：{e}"))
}

fn legacy_root(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_local_data_dir()
        .map(|p| p.join("native-funasr"))
        .map_err(|e| format!("无法获取应用本地数据目录：{e}"))
}

fn runtime_choice() -> (&'static str, &'static str, &'static str) {
    #[cfg(target_arch = "x86_64")]
    {
        if std::is_x86_feature_detected!("avx2")
            && std::is_x86_feature_detected!("fma")
            && std::is_x86_feature_detected!("f16c")
            && std::is_x86_feature_detected!("bmi2")
        {
            return (RUNTIME_AVX2_FILE, RUNTIME_AVX2_SHA256, "windows-x64-avx2");
        }
    }
    (RUNTIME_X64_FILE, RUNTIME_X64_SHA256, "windows-x64")
}

fn github_release_url(file: &str) -> String {
    format!("https://github.com/modelscope/FunASR/releases/download/{RUNTIME_VERSION}/{file}")
}

fn github_proxy_url(file: &str) -> String {
    format!("https://gh-proxy.com/https://github.com/modelscope/FunASR/releases/download/{RUNTIME_VERSION}/{file}")
}

fn find_named_file(root: &Path, file_name: &str) -> Option<PathBuf> {
    if !root.exists() {
        return None;
    }
    WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .find(|entry| {
            entry.file_type().is_file()
                && entry
                    .file_name()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(file_name)
        })
        .map(|entry| entry.path().to_path_buf())
}

fn value_u64(value: Option<&Value>) -> u64 {
    match value {
        Some(Value::Number(n)) => n.as_u64().unwrap_or(0),
        Some(Value::String(s)) => s.parse::<u64>().unwrap_or(0),
        _ => 0,
    }
}

fn find_files_array(value: &Value) -> Option<&Vec<Value>> {
    match value {
        Value::Object(map) => {
            for key in ["Files", "files"] {
                if let Some(Value::Array(items)) = map.get(key) {
                    return Some(items);
                }
            }
            for child in map.values() {
                if let Some(found) = find_files_array(child) {
                    return Some(found);
                }
            }
            None
        }
        _ => None,
    }
}

async fn list_repo_files(client: &Client, repo: &str) -> Result<Vec<RepoFile>, String> {
    let url = format!("https://modelscope.cn/api/v1/models/{repo}/repo/files");
    let response = client
        .get(url)
        .query(&[("Revision", REVISION), ("Recursive", "True")])
        .send()
        .await
        .map_err(|e| format!("读取 ModelScope 仓库 {repo} 文件列表失败：{e}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "读取 ModelScope 仓库 {repo} 文件列表失败：HTTP {}",
            response.status()
        ));
    }
    let value: Value = response
        .json()
        .await
        .map_err(|e| format!("解析 ModelScope 仓库 {repo} 文件列表失败：{e}"))?;
    let items = find_files_array(&value)
        .ok_or_else(|| format!("ModelScope 仓库 {repo} 返回格式中没有 Files 数组"))?;
    let mut files = Vec::new();
    for item in items {
        let Some(obj) = item.as_object() else { continue };
        let file_type = obj
            .get("Type")
            .or_else(|| obj.get("type"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_lowercase();
        if matches!(file_type.as_str(), "tree" | "dir" | "directory" | "folder") {
            continue;
        }
        let path = obj
            .get("Path")
            .or_else(|| obj.get("path"))
            .or_else(|| obj.get("Name"))
            .or_else(|| obj.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .replace('\\', "/");
        if path.is_empty() || path.ends_with('/') || path.starts_with(".git/") {
            continue;
        }
        let size = value_u64(
            obj.get("Size")
                .or_else(|| obj.get("size"))
                .or_else(|| obj.get("FileSize"))
                .or_else(|| obj.get("fileSize")),
        );
        files.push(RepoFile { path, size });
    }
    if files.is_empty() {
        return Err(format!("ModelScope 仓库 {repo} 没有返回可下载文件"));
    }
    Ok(files)
}

async fn download_modelscope_file(
    client: &Client,
    repo: &str,
    wanted_file: &str,
    dest: &Path,
    label: &str,
    channel: &Channel<NativeInstallEvent>,
    index: usize,
    total_items: usize,
) -> Result<(), String> {
    let files = list_repo_files(client, repo).await?;
    let file = files
        .into_iter()
        .find(|f| {
            Path::new(&f.path)
                .file_name()
                .and_then(|v| v.to_str())
                .map(|v| v.eq_ignore_ascii_case(wanted_file))
                .unwrap_or(false)
        })
        .ok_or_else(|| format!("ModelScope 仓库 {repo} 中没有找到 {wanted_file}"))?;

    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("创建模型目录失败：{e}"))?;
    }
    if let Ok(meta) = tokio::fs::metadata(dest).await {
        if meta.is_file() && (file.size == 0 || meta.len() == file.size) {
            let _ = send(
                channel,
                NativeInstallEvent::Progress {
                    label: label.into(),
                    current_file: wanted_file.into(),
                    downloaded_bytes: meta.len(),
                    total_bytes: file.size.max(meta.len()),
                    index,
                    total_items,
                },
            );
            return Ok(());
        }
        if meta.is_file() {
            let _ = tokio::fs::remove_file(dest).await;
        }
    }

    let part = dest.with_extension("gguf.part");
    let mut existing = tokio::fs::metadata(&part).await.map(|m| m.len()).unwrap_or(0);
    if file.size > 0 && existing > file.size {
        let _ = tokio::fs::remove_file(&part).await;
        existing = 0;
    }

    let url = format!("https://modelscope.cn/api/v1/models/{repo}/repo");
    let mut request = client
        .get(url)
        .query(&[("Revision", REVISION), ("FilePath", file.path.as_str())]);
    if existing > 0 {
        request = request.header(header::RANGE, format!("bytes={existing}-"));
    }
    let response = request
        .send()
        .await
        .map_err(|e| format!("下载 {repo}/{wanted_file} 失败：{e}"))?;
    let append = existing > 0 && response.status() == StatusCode::PARTIAL_CONTENT;
    if !response.status().is_success() {
        return Err(format!(
            "下载 {repo}/{wanted_file} 失败：HTTP {}",
            response.status()
        ));
    }
    if existing > 0 && !append {
        existing = 0;
    }

    let mut output = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(append)
        .truncate(!append)
        .open(&part)
        .await
        .map_err(|e| format!("打开模型临时文件失败：{e}"))?;
    let mut stream = response.bytes_stream();
    let mut current = existing;
    let mut last_report = current;
    while let Some(next) = stream.next().await {
        let bytes = next.map_err(|e| format!("下载 {wanted_file} 中断：{e}"))?;
        output
            .write_all(&bytes)
            .await
            .map_err(|e| format!("写入 {wanted_file} 失败：{e}"))?;
        current = current.saturating_add(bytes.len() as u64);
        if current.saturating_sub(last_report) >= 512 * 1024 {
            let _ = send(
                channel,
                NativeInstallEvent::Progress {
                    label: label.into(),
                    current_file: wanted_file.into(),
                    downloaded_bytes: current,
                    total_bytes: file.size,
                    index,
                    total_items,
                },
            );
            last_report = current;
        }
    }
    output.flush().await.map_err(|e| format!("刷新模型文件失败：{e}"))?;
    drop(output);
    if file.size > 0 && current != file.size {
        return Err(format!(
            "{wanted_file} 下载大小不匹配：期望 {} bytes，实际 {} bytes",
            file.size, current
        ));
    }
    tokio::fs::rename(&part, dest)
        .await
        .map_err(|e| format!("完成模型下载失败：{e}"))?;
    Ok(())
}

async fn download_url_with_fallback(
    client: &Client,
    urls: &[String],
    dest: &Path,
    label: &str,
    display_file: &str,
    channel: &Channel<NativeInstallEvent>,
    index: usize,
    total_items: usize,
) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("创建下载目录失败：{e}"))?;
    }
    let part = dest.with_extension(format!(
        "{}part",
        dest.extension()
            .and_then(|v| v.to_str())
            .map(|v| format!("{v}."))
            .unwrap_or_default()
    ));
    let mut errors = Vec::new();
    for url in urls {
        let _ = tokio::fs::remove_file(&part).await;
        let response = match client.get(url).send().await {
            Ok(v) => v,
            Err(e) => {
                errors.push(format!("{url}: {e}"));
                continue;
            }
        };
        if !response.status().is_success() {
            errors.push(format!("{url}: HTTP {}", response.status()));
            continue;
        }
        let total_bytes = response.content_length().unwrap_or(0);
        let mut output = tokio::fs::File::create(&part)
            .await
            .map_err(|e| format!("创建下载临时文件失败：{e}"))?;
        let mut stream = response.bytes_stream();
        let mut downloaded = 0u64;
        let mut last_report = 0u64;
        let mut failed = None;
        while let Some(next) = stream.next().await {
            match next {
                Ok(bytes) => {
                    if let Err(e) = output.write_all(&bytes).await {
                        failed = Some(format!("写入下载文件失败：{e}"));
                        break;
                    }
                    downloaded = downloaded.saturating_add(bytes.len() as u64);
                    if downloaded.saturating_sub(last_report) >= 512 * 1024 {
                        let _ = send(
                            channel,
                            NativeInstallEvent::Progress {
                                label: label.into(),
                                current_file: display_file.into(),
                                downloaded_bytes: downloaded,
                                total_bytes,
                                index,
                                total_items,
                            },
                        );
                        last_report = downloaded;
                    }
                }
                Err(e) => {
                    failed = Some(format!("下载中断：{e}"));
                    break;
                }
            }
        }
        if let Some(e) = failed {
            errors.push(format!("{url}: {e}"));
            continue;
        }
        output.flush().await.map_err(|e| format!("刷新下载文件失败：{e}"))?;
        drop(output);
        if total_bytes > 0 && downloaded != total_bytes {
            errors.push(format!("{url}: 大小不匹配 {downloaded}/{total_bytes}"));
            continue;
        }
        if dest.is_file() {
            let _ = tokio::fs::remove_file(dest).await;
        }
        tokio::fs::rename(&part, dest)
            .await
            .map_err(|e| format!("完成下载文件失败：{e}"))?;
        return Ok(());
    }
    Err(format!("所有下载源均失败：{}", errors.join(" | ")))
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|e| format!("打开校验文件失败：{e}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buffer).map_err(|e| format!("计算 SHA-256 失败：{e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn verify_sha256(path: &Path, expected: &str) -> Result<(), String> {
    let actual = sha256_file(path)?;
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(format!(
            "{} SHA-256 校验失败：期望 {expected}，实际 {actual}",
            path.display()
        ))
    }
}

fn verify_gguf(path: &Path) -> Result<(), String> {
    let mut file = File::open(path).map_err(|e| format!("打开 GGUF 文件失败：{e}"))?;
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic)
        .map_err(|e| format!("读取 GGUF 文件头失败：{e}"))?;
    if &magic == b"GGUF" {
        Ok(())
    } else {
        Err(format!("{} 不是有效的 GGUF 文件", path.display()))
    }
}

fn verify_punctuation_model(path: &Path) -> Result<(), String> {
    let meta = std::fs::metadata(path)
        .map_err(|e| format!("读取标点模型失败 {}：{e}", path.display()))?;
    if meta.is_file() && meta.len() > 50 * 1024 * 1024 {
        Ok(())
    } else {
        Err(format!("{} 不是有效的中英标点 INT8 模型", path.display()))
    }
}

fn verify_alignment_model(model: &Path, tokens: &Path) -> Result<(), String> {
    let model_meta = std::fs::metadata(model)
        .map_err(|e| format!("读取 English CTC 模型失败 {}：{e}", model.display()))?;
    let tokens_meta = std::fs::metadata(tokens)
        .map_err(|e| format!("读取 English CTC tokens 失败 {}：{e}", tokens.display()))?;
    if model_meta.is_file() && model_meta.len() > 35 * 1024 * 1024 && tokens_meta.is_file() && tokens_meta.len() > 20 {
        Ok(())
    } else {
        Err("English CTC INT8 模型或 tokens.txt 不完整".into())
    }
}

fn verify_zip(path: &Path) -> Result<(), String> {
    let file = File::open(path).map_err(|e| format!("打开 ZIP 校验失败：{e}"))?;
    let archive = ZipArchive::new(file)
        .map_err(|e| format!("{} 不是有效的 ZIP：{e}", path.display()))?;
    if archive.len() == 0 {
        Err(format!("{} ZIP 内容为空", path.display()))
    } else {
        Ok(())
    }
}

fn extract_zip_file(zip_path: &Path, dest_root: &Path) -> Result<(), String> {
    let file = File::open(zip_path)
        .map_err(|e| format!("打开 ZIP {} 失败：{e}", zip_path.display()))?;
    let mut archive = ZipArchive::new(file)
        .map_err(|e| format!("解析 ZIP {} 失败：{e}", zip_path.display()))?;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("读取 ZIP 第 {i} 项失败：{e}"))?;
        let Some(enclosed) = entry.enclosed_name().map(Path::to_path_buf) else {
            continue;
        };
        let out = dest_root.join(enclosed);
        if entry.is_dir() {
            std::fs::create_dir_all(&out)
                .map_err(|e| format!("创建解压目录 {} 失败：{e}", out.display()))?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("创建解压目录 {} 失败：{e}", parent.display()))?;
        }
        let mut output = File::create(&out)
            .map_err(|e| format!("创建解压文件 {} 失败：{e}", out.display()))?;
        io::copy(&mut entry, &mut output)
            .map_err(|e| format!("解压 {} 失败：{e}", out.display()))?;
    }
    Ok(())
}

fn extract_tar_bz2(archive_path: &Path, dest_root: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dest_root)
        .map_err(|e| format!("创建解压目录 {} 失败：{e}", dest_root.display()))?;
    let file = File::open(archive_path)
        .map_err(|e| format!("打开 tar.bz2 {} 失败：{e}", archive_path.display()))?;
    let decoder = BzDecoder::new(file);
    let mut archive = Archive::new(decoder);
    archive.unpack(dest_root)
        .map_err(|e| format!("解压 tar.bz2 {} 失败：{e}", archive_path.display()))
}

fn extract_punctuation_model(archive_path: &Path, dest: &Path) -> Result<(), String> {
    let file = File::open(archive_path)
        .map_err(|e| format!("打开标点模型压缩包失败：{e}"))?;
    let decoder = BzDecoder::new(file);
    let mut archive = Archive::new(decoder);
    let entries = archive
        .entries()
        .map_err(|e| format!("读取标点模型 tar.bz2 失败：{e}"))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| format!("读取标点模型压缩项失败：{e}"))?;
        let path = entry
            .path()
            .map_err(|e| format!("读取标点模型压缩路径失败：{e}"))?;
        let is_model = path
            .file_name()
            .and_then(|v| v.to_str())
            .map(|v| v.eq_ignore_ascii_case(PUNCT_MODEL_FILE))
            .unwrap_or(false);
        if !is_model {
            continue;
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("创建标点模型目录失败：{e}"))?;
        }
        let mut output = File::create(dest)
            .map_err(|e| format!("创建标点模型文件失败：{e}"))?;
        io::copy(&mut entry, &mut output)
            .map_err(|e| format!("解压标点模型失败：{e}"))?;
        return verify_punctuation_model(dest);
    }
    Err(format!("标点模型压缩包中没有找到 {PUNCT_MODEL_FILE}"))
}

async fn ensure_runtime(
    client: &Client,
    root: &Path,
    spec: ModelSpec,
    channel: &Channel<NativeInstallEvent>,
    index: usize,
    total_items: usize,
) -> Result<PathBuf, String> {
    let runtime_dir = root.join("runtime").join("v0.2.0");
    if let Some(exe) = find_named_file(&runtime_dir, spec.exe) {
        return Ok(exe);
    }
    tokio::fs::create_dir_all(&runtime_dir)
        .await
        .map_err(|e| format!("创建 Runtime 目录失败：{e}"))?;
    let (file, sha, _) = runtime_choice();
    let zip_path = runtime_dir.join(file);
    if !zip_path.is_file() || verify_sha256(&zip_path, sha).is_err() {
        let urls = [github_proxy_url(file), github_release_url(file)];
        let mut errors = Vec::new();
        let mut verified = false;
        for url in urls {
            match download_url_with_fallback(
                client,
                &[url.clone()],
                &zip_path,
                "FunASR llama.cpp Runtime v0.2.0",
                file,
                channel,
                index,
                total_items,
            )
            .await
            {
                Ok(()) => match verify_sha256(&zip_path, sha) {
                    Ok(()) => {
                        verified = true;
                        break;
                    }
                    Err(e) => {
                        errors.push(format!("{url}: {e}"));
                        let _ = tokio::fs::remove_file(&zip_path).await;
                    }
                },
                Err(e) => errors.push(format!("{url}: {e}")),
            }
        }
        if !verified {
            return Err(format!("FunASR Runtime 下载或校验失败：{}", errors.join(" | ")));
        }
    }
    let zip_clone = zip_path.clone();
    let root_clone = runtime_dir.clone();
    tokio::task::spawn_blocking(move || extract_zip_file(&zip_clone, &root_clone))
        .await
        .map_err(|e| format!("解压 Runtime 任务失败：{e}"))??;
    find_named_file(&runtime_dir, spec.exe)
        .ok_or_else(|| format!("Runtime 已解压，但没有找到 {}", spec.exe))
}

async fn ensure_selected_model(
    client: &Client,
    root: &Path,
    spec: ModelSpec,
    channel: &Channel<NativeInstallEvent>,
    index: usize,
    total_items: usize,
) -> Result<PathBuf, String> {
    let dest = root.join("models").join(spec.file);
    if dest.is_file() && verify_gguf(&dest).is_ok() {
        return Ok(dest);
    }
    let _ = tokio::fs::remove_file(&dest).await;
    download_modelscope_file(
        client,
        spec.repo,
        spec.file,
        &dest,
        spec.label,
        channel,
        index,
        total_items,
    )
    .await?;
    verify_gguf(&dest)?;
    Ok(dest)
}

async fn ensure_encoder_model(
    client: &Client,
    root: &Path,
    spec: ModelSpec,
    channel: &Channel<NativeInstallEvent>,
    index: usize,
    total_items: usize,
) -> Result<Option<PathBuf>, String> {
    let Some(file) = spec.encoder_file else {
        return Ok(None);
    };
    let dest = root.join("models").join(file);
    if dest.is_file() && verify_gguf(&dest).is_ok() {
        return Ok(Some(dest));
    }
    let _ = tokio::fs::remove_file(&dest).await;
    download_modelscope_file(
        client,
        spec.repo,
        file,
        &dest,
        "Fun-ASR-Nano Encoder F16",
        channel,
        index,
        total_items,
    )
    .await?;
    verify_gguf(&dest)?;
    Ok(Some(dest))
}

async fn ensure_vad(
    client: &Client,
    root: &Path,
    channel: &Channel<NativeInstallEvent>,
    index: usize,
    total_items: usize,
) -> Result<PathBuf, String> {
    let dest = root.join("models").join(VAD_FILE);
    if dest.is_file()
        && tokio::fs::metadata(&dest)
            .await
            .map(|m| m.len() > 100_000)
            .unwrap_or(false)
        && verify_gguf(&dest).is_ok()
    {
        return Ok(dest);
    }
    let urls = [VAD_HF_MIRROR_URL.to_string(), VAD_HF_URL.to_string()];
    let mut errors = Vec::new();
    for url in urls {
        match download_url_with_fallback(
            client,
            &[url.clone()],
            &dest,
            "FSMN-VAD GGUF",
            VAD_FILE,
            channel,
            index,
            total_items,
        )
        .await
        {
            Ok(()) => match verify_gguf(&dest) {
                Ok(()) => return Ok(dest),
                Err(e) => {
                    errors.push(format!("{url}: {e}"));
                    let _ = tokio::fs::remove_file(&dest).await;
                }
            },
            Err(e) => errors.push(format!("{url}: {e}")),
        }
    }
    Err(format!("FSMN-VAD 下载或校验失败：{}", errors.join(" | ")))
}

async fn ensure_punctuation_runtime(
    client: &Client,
    root: &Path,
    channel: &Channel<NativeInstallEvent>,
    index: usize,
    total_items: usize,
) -> Result<PathBuf, String> {
    let dir = root.join("tools").join("sherpa-onnx").join(SHERPA_VERSION);
    if let Some(exe) = find_named_file(&dir, SHERPA_C_API_DLL) {
        return Ok(exe);
    }
    tokio::fs::create_dir_all(&dir).await
        .map_err(|e| format!("创建 sherpa-onnx Runtime 目录失败：{e}"))?;
    let archive_path = dir.join(SHERPA_ARCHIVE);
    let official = format!("https://github.com/k2-fsa/sherpa-onnx/releases/download/{SHERPA_VERSION}/{SHERPA_ARCHIVE}");
    let mirror = format!("https://gh-proxy.com/{official}");
    let mut errors = Vec::new();
    for url in [mirror, official] {
        let _ = tokio::fs::remove_file(&archive_path).await;
        match download_url_with_fallback(
            client, &[url.clone()], &archive_path,
            "sherpa-onnx C API Runtime v1.13.6", SHERPA_ARCHIVE, channel, index, total_items
        ).await {
            Ok(()) => {
                let archive_clone = archive_path.clone();
                let dir_clone = dir.clone();
                match tokio::task::spawn_blocking(move || extract_tar_bz2(&archive_clone, &dir_clone)).await {
                    Ok(Ok(())) => {
                        if let Some(exe) = find_named_file(&dir, SHERPA_C_API_DLL) {
                            return Ok(exe);
                        }
                        errors.push(format!("{url}: 解压成功但没有找到 {SHERPA_C_API_DLL}"));
                    }
                    Ok(Err(e)) => errors.push(format!("{url}: {e}")),
                    Err(e) => errors.push(format!("{url}: 解压任务失败：{e}")),
                }
            }
            Err(e) => errors.push(format!("{url}: {e}")),
        }
    }
    Err(format!("sherpa-onnx Runtime 下载或解压失败：{}", errors.join(" | ")))
}

async fn ensure_punctuation(
    client: &Client,
    root: &Path,
    channel: &Channel<NativeInstallEvent>,
    index: usize,
    total_items: usize,
) -> Result<PathBuf, String> {
    let dir = root.join("models").join("punctuation");
    let dest = dir.join(PUNCT_MODEL_FILE);
    if dest.is_file() && verify_punctuation_model(&dest).is_ok() {
        return Ok(dest);
    }
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| format!("创建标点模型目录失败：{e}"))?;
    let archive_path = dir.join(PUNCT_ARCHIVE);
    let urls = [PUNCT_MIRROR_URL.to_string(), PUNCT_URL.to_string()];
    let mut errors = Vec::new();
    for url in urls {
        let _ = tokio::fs::remove_file(&archive_path).await;
        let _ = tokio::fs::remove_file(&dest).await;
        match download_url_with_fallback(
            client,
            &[url.clone()],
            &archive_path,
            "中英标点恢复 INT8",
            PUNCT_ARCHIVE,
            channel,
            index,
            total_items,
        )
        .await
        {
            Ok(()) => {
                let archive_clone = archive_path.clone();
                let dest_clone = dest.clone();
                let result = tokio::task::spawn_blocking(move || {
                    extract_punctuation_model(&archive_clone, &dest_clone)
                })
                .await
                .map_err(|e| format!("解压标点模型任务失败：{e}"))?;
                match result.and_then(|_| verify_punctuation_model(&dest)) {
                    Ok(()) => return Ok(dest),
                    Err(e) => errors.push(format!("{url}: {e}")),
                }
            }
            Err(e) => errors.push(format!("{url}: {e}")),
        }
    }
    let _ = tokio::fs::remove_file(&archive_path).await;
    let _ = tokio::fs::remove_file(&dest).await;
    Err(format!("中英标点模型下载或解压失败：{}", errors.join(" | ")))
}

async fn ensure_alignment_model(
    client: &Client,
    root: &Path,
    channel: &Channel<NativeInstallEvent>,
    index: usize,
    total_items: usize,
) -> Result<(PathBuf, PathBuf), String> {
    let dir = root.join("models").join("alignment").join("en-conformer-small");
    if let (Some(model), Some(tokens)) = (
        find_named_file(&dir, ALIGNMENT_MODEL_FILE),
        find_named_file(&dir, ALIGNMENT_TOKENS_FILE),
    ) {
        if verify_alignment_model(&model, &tokens).is_ok() { return Ok((model, tokens)); }
    }
    tokio::fs::create_dir_all(&dir).await
        .map_err(|e| format!("创建 English CTC 对齐模型目录失败：{e}"))?;
    let archive_path = dir.join(ALIGNMENT_ARCHIVE);
    let urls = [ALIGNMENT_MIRROR_URL.to_string(), ALIGNMENT_URL.to_string()];
    let mut errors = Vec::new();
    for url in urls {
        let _ = tokio::fs::remove_file(&archive_path).await;
        match download_url_with_fallback(
            client, &[url.clone()], &archive_path,
            "English CTC Small INT8 · 选择性真实停顿修复", ALIGNMENT_ARCHIVE, channel, index, total_items
        ).await {
            Ok(()) => {
                let archive_clone = archive_path.clone();
                let dir_clone = dir.clone();
                match tokio::task::spawn_blocking(move || extract_tar_bz2(&archive_clone, &dir_clone)).await {
                    Ok(Ok(())) => {
                        let model = find_named_file(&dir, ALIGNMENT_MODEL_FILE);
                        let tokens = find_named_file(&dir, ALIGNMENT_TOKENS_FILE);
                        if let (Some(model), Some(tokens)) = (model, tokens) {
                            match verify_alignment_model(&model, &tokens) {
                                Ok(()) => {
                                    let _ = tokio::fs::remove_file(&archive_path).await;
                                    return Ok((model, tokens));
                                }
                                Err(e) => errors.push(format!("{url}: {e}")),
                            }
                        } else {
                            errors.push(format!("{url}: 解压后未找到 {ALIGNMENT_MODEL_FILE} / {ALIGNMENT_TOKENS_FILE}"));
                        }
                    }
                    Ok(Err(e)) => errors.push(format!("{url}: {e}")),
                    Err(e) => errors.push(format!("{url}: 解压任务失败：{e}")),
                }
            }
            Err(e) => errors.push(format!("{url}: {e}")),
        }
    }
    let _ = tokio::fs::remove_file(&archive_path).await;
    Err(format!("English CTC Small INT8 下载或解压失败：{}", errors.join(" | ")))
}

async fn ensure_ffmpeg(
    client: &Client,
    root: &Path,
    legacy: &Path,
    channel: &Channel<NativeInstallEvent>,
    index: usize,
    total_items: usize,
) -> Result<(PathBuf, PathBuf), String> {
    let new_dir = root.join("tools").join("ffmpeg");
    let legacy_dir = legacy.join("tools").join("ffmpeg");
    for dir in [&new_dir, &legacy_dir] {
        if let (Some(ffmpeg), Some(ffprobe)) = (
            find_named_file(dir, "ffmpeg.exe"),
            find_named_file(dir, "ffprobe.exe"),
        ) {
            return Ok((ffmpeg, ffprobe));
        }
    }
    tokio::fs::create_dir_all(&new_dir)
        .await
        .map_err(|e| format!("创建 FFmpeg 目录失败：{e}"))?;
    let zip_path = new_dir.join(FFMPEG_FILE);
    let urls = [FFMPEG_MIRROR_URL.to_string(), FFMPEG_URL.to_string()];
    let mut errors = Vec::new();
    let mut downloaded = false;
    for url in urls {
        match download_url_with_fallback(
            client,
            &[url.clone()],
            &zip_path,
            "FFmpeg / FFprobe",
            FFMPEG_FILE,
            channel,
            index,
            total_items,
        )
        .await
        {
            Ok(()) => match verify_zip(&zip_path) {
                Ok(()) => {
                    downloaded = true;
                    break;
                }
                Err(e) => {
                    errors.push(format!("{url}: {e}"));
                    let _ = tokio::fs::remove_file(&zip_path).await;
                }
            },
            Err(e) => errors.push(format!("{url}: {e}")),
        }
    }
    if !downloaded {
        return Err(format!("FFmpeg 下载或 ZIP 校验失败：{}", errors.join(" | ")));
    }
    let zip_clone = zip_path.clone();
    let dir_clone = new_dir.clone();
    tokio::task::spawn_blocking(move || extract_zip_file(&zip_clone, &dir_clone))
        .await
        .map_err(|e| format!("解压 FFmpeg 任务失败：{e}"))??;
    let ffmpeg = find_named_file(&new_dir, "ffmpeg.exe")
        .ok_or_else(|| "FFmpeg 已解压，但没有找到 ffmpeg.exe".to_string())?;
    let ffprobe = find_named_file(&new_dir, "ffprobe.exe")
        .ok_or_else(|| "FFmpeg 已解压，但没有找到 ffprobe.exe".to_string())?;
    Ok((ffmpeg, ffprobe))
}

fn auto_paths(root: &Path, legacy: &Path, spec: ModelSpec) -> NativeFunAsrPaths {
    let runtime_dir = root.join("runtime").join("v0.2.0");
    let models_dir = root.join("models");
    let ffmpeg_new = root.join("tools").join("ffmpeg");
    let ffmpeg_legacy = legacy.join("tools").join("ffmpeg");
    let ffmpeg = find_named_file(&ffmpeg_new, "ffmpeg.exe")
        .or_else(|| find_named_file(&ffmpeg_legacy, "ffmpeg.exe"));
    let ffprobe = find_named_file(&ffmpeg_new, "ffprobe.exe")
        .or_else(|| find_named_file(&ffmpeg_legacy, "ffprobe.exe"));
    let (_, _, variant) = runtime_choice();
    NativeFunAsrPaths {
        runtime_path: find_named_file(&runtime_dir, spec.exe)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default(),
        model_path: models_dir.join(spec.file).to_string_lossy().into_owned(),
        encoder_model_path: spec.encoder_file
            .map(|file| models_dir.join(file).to_string_lossy().into_owned())
            .unwrap_or_default(),
        vad_model_path: models_dir.join(VAD_FILE).to_string_lossy().into_owned(),
        punctuation_runtime_path: find_named_file(&root.join("tools").join("sherpa-onnx").join(SHERPA_VERSION), SHERPA_C_API_DLL)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default(),
        punctuation_model_path: models_dir
            .join("punctuation")
            .join(PUNCT_MODEL_FILE)
            .to_string_lossy()
            .into_owned(),
        alignment_model_path: find_named_file(&models_dir.join("alignment").join("en-conformer-small"), ALIGNMENT_MODEL_FILE)
            .map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
        alignment_tokens_path: find_named_file(&models_dir.join("alignment").join("en-conformer-small"), ALIGNMENT_TOKENS_FILE)
            .map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
        ffmpeg_path: ffmpeg.map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
        ffprobe_path: ffprobe.map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
        install_root: root.to_string_lossy().into_owned(),
        runtime_version: "v0.2.0".into(),
        runtime_variant: variant.into(),
        model_kind: spec.kind.into(),
        model_label: spec.label.into(),
    }
}

pub fn status(app: &AppHandle, request: &NativeModelRequest) -> Result<NativeFunAsrStatus, String> {
    let spec = model_spec(&request.model_kind)?;
    let root = install_root(app)?;
    let legacy = legacy_root(app)?;
    let paths = auto_paths(&root, &legacy, spec);
    let runtime_ready = !paths.runtime_path.is_empty() && Path::new(&paths.runtime_path).is_file();
    let llm_ready = Path::new(&paths.model_path).is_file()
        && verify_gguf(Path::new(&paths.model_path)).is_ok();
    let encoder_ready = spec.encoder_file.is_none()
        || (!paths.encoder_model_path.is_empty()
            && Path::new(&paths.encoder_model_path).is_file()
            && verify_gguf(Path::new(&paths.encoder_model_path)).is_ok());
    let model_ready = llm_ready && encoder_ready;
    let vad_ready = Path::new(&paths.vad_model_path).is_file()
        && verify_gguf(Path::new(&paths.vad_model_path)).is_ok();
    let punctuation_ready = !paths.punctuation_runtime_path.is_empty()
        && Path::new(&paths.punctuation_runtime_path).is_file()
        && Path::new(&paths.punctuation_model_path).is_file()
        && verify_punctuation_model(Path::new(&paths.punctuation_model_path)).is_ok();
    let alignment_ready = spec.kind != "nano" || (!paths.alignment_model_path.is_empty()
        && !paths.alignment_tokens_path.is_empty()
        && verify_alignment_model(Path::new(&paths.alignment_model_path), Path::new(&paths.alignment_tokens_path)).is_ok());
    let ffmpeg_ready = !paths.ffmpeg_path.is_empty()
        && !paths.ffprobe_path.is_empty()
        && Path::new(&paths.ffmpeg_path).is_file()
        && Path::new(&paths.ffprobe_path).is_file();
    let mut missing = Vec::new();
    if !runtime_ready {
        missing.push("FunASR llama.cpp Runtime v0.2.0".into());
    }
    if !model_ready {
        missing.push(spec.label.into());
    }
    if !vad_ready {
        missing.push("FSMN-VAD GGUF".into());
    }
    if !punctuation_ready {
        missing.push("sherpa-onnx v1.13.6 + 中英标点恢复 INT8".into());
    }
    if !alignment_ready {
        missing.push("English CTC Small INT8 · 选择性真实停顿修复（可选增强）".into());
    }
    if !ffmpeg_ready {
        missing.push("FFmpeg / FFprobe".into());
    }
    Ok(NativeFunAsrStatus {
        // The alignment model is an optional selective-repair enhancement. Nano must remain
        // usable when it is missing or when its download is unavailable.
        ready: runtime_ready && model_ready && vad_ready && punctuation_ready && ffmpeg_ready,
        runtime_ready,
        model_ready,
        vad_ready,
        punctuation_ready,
        alignment_ready,
        ffmpeg_ready,
        paths,
        missing,
    })
}

pub async fn install(
    app: AppHandle,
    request: NativeModelRequest,
    on_event: Channel<NativeInstallEvent>,
) -> Result<NativeFunAsrPaths, String> {
    let spec = model_spec(&request.model_kind)?;
    let root = install_root(&app)?;
    let legacy = legacy_root(&app)?;
    tokio::fs::create_dir_all(&root)
        .await
        .map_err(|e| format!("创建 FunASR GGUF 安装目录失败：{e}"))?;
    let client = Client::builder()
        .user_agent("Local-ASR-Studio/0.9.2")
        .redirect(reqwest::redirect::Policy::limited(12))
        .connect_timeout(std::time::Duration::from_secs(12))
        .build()
        .map_err(|e| format!("初始化下载客户端失败：{e}"))?;

    let total_items = if spec.kind == "nano" { 7usize } else { 6usize };
    send(&on_event, NativeInstallEvent::Started { total_items })?;

    let (runtime_file, _, runtime_variant) = runtime_choice();
    send(
        &on_event,
        NativeInstallEvent::ItemStarted {
            label: "FunASR llama.cpp Runtime v0.2.0".into(),
            repo: format!("GitHub Release · {runtime_variant}"),
            index: 1,
            total_items,
        },
    )?;
    let runtime = ensure_runtime(&client, &root, spec, &on_event, 1, total_items).await?;
    send(
        &on_event,
        NativeInstallEvent::ItemFinished {
            label: format!("Runtime v0.2.0 · {runtime_file}"),
            path: runtime.to_string_lossy().into_owned(),
            index: 1,
            total_items,
        },
    )?;

    send(
        &on_event,
        NativeInstallEvent::ItemStarted {
            label: spec.label.into(),
            repo: spec.repo.into(),
            index: 2,
            total_items,
        },
    )?;
    let encoder = match ensure_encoder_model(&client, &root, spec, &on_event, 2, total_items).await {
        Ok(v) => v,
        Err(error) => {
            let _ = send(&on_event, NativeInstallEvent::Error { message: error.clone() });
            return Err(error);
        }
    };
    let model = match ensure_selected_model(&client, &root, spec, &on_event, 2, total_items).await {
        Ok(v) => v,
        Err(error) => {
            let _ = send(&on_event, NativeInstallEvent::Error { message: error.clone() });
            return Err(error);
        }
    };
    let model_summary = encoder
        .as_ref()
        .map(|enc| format!("{} + {}", enc.display(), model.display()))
        .unwrap_or_else(|| model.to_string_lossy().into_owned());
    send(
        &on_event,
        NativeInstallEvent::ItemFinished {
            label: spec.label.into(),
            path: model_summary,
            index: 2,
            total_items,
        },
    )?;

    send(
        &on_event,
        NativeInstallEvent::ItemStarted {
            label: "FSMN-VAD GGUF".into(),
            repo: "FunAudioLLM/fsmn-vad-GGUF".into(),
            index: 3,
            total_items,
        },
    )?;
    let vad = match ensure_vad(&client, &root, &on_event, 3, total_items).await {
        Ok(v) => v,
        Err(error) => {
            let _ = send(&on_event, NativeInstallEvent::Error { message: error.clone() });
            return Err(error);
        }
    };
    send(
        &on_event,
        NativeInstallEvent::ItemFinished {
            label: "FSMN-VAD GGUF".into(),
            path: vad.to_string_lossy().into_owned(),
            index: 3,
            total_items,
        },
    )?;

    let mut next_index = 4usize;

    send(
        &on_event,
        NativeInstallEvent::ItemStarted {
            label: "sherpa-onnx C API Runtime v1.13.6".into(),
            repo: "k2-fsa/sherpa-onnx · Windows x64 prebuilt".into(),
            index: next_index,
            total_items,
        },
    )?;
    let punctuation_runtime = match ensure_punctuation_runtime(
        &client,
        &root,
        &on_event,
        next_index,
        total_items,
    )
    .await
    {
        Ok(v) => v,
        Err(error) => {
            let _ = send(&on_event, NativeInstallEvent::Error { message: error.clone() });
            return Err(error);
        }
    };
    send(
        &on_event,
        NativeInstallEvent::ItemFinished {
            label: "sherpa-onnx C API Runtime v1.13.6".into(),
            path: punctuation_runtime.to_string_lossy().into_owned(),
            index: next_index,
            total_items,
        },
    )?;
    next_index += 1;

    send(
        &on_event,
        NativeInstallEvent::ItemStarted {
            label: "中英标点恢复 INT8".into(),
            repo: "k2-fsa/sherpa-onnx punctuation-models".into(),
            index: next_index,
            total_items,
        },
    )?;
    let punctuation = match ensure_punctuation(
        &client,
        &root,
        &on_event,
        next_index,
        total_items,
    )
    .await
    {
        Ok(v) => v,
        Err(error) => {
            let _ = send(&on_event, NativeInstallEvent::Error { message: error.clone() });
            return Err(error);
        }
    };
    send(
        &on_event,
        NativeInstallEvent::ItemFinished {
            label: "中英标点恢复 INT8".into(),
            path: punctuation.to_string_lossy().into_owned(),
            index: next_index,
            total_items,
        },
    )?;
    next_index += 1;

    if spec.kind == "nano" {
        send(
            &on_event,
            NativeInstallEvent::ItemStarted {
                label: "English CTC Small INT8 · 选择性真实停顿修复".into(),
                repo: "k2-fsa/sherpa-onnx asr-models".into(),
                index: next_index,
                total_items,
            },
        )?;
        match ensure_alignment_model(
            &client, &root, &on_event, next_index, total_items,
        ).await {
            Ok((alignment_model, alignment_tokens)) => {
                send(
                    &on_event,
                    NativeInstallEvent::ItemFinished {
                        label: "English CTC Small INT8 · 选择性真实停顿修复".into(),
                        path: format!("{} + {}", alignment_model.display(), alignment_tokens.display()),
                        index: next_index,
                        total_items,
                    },
                )?;
            }
            Err(error) => {
                // Alignment is an optional selective-repair enhancement; never make the main
                // Nano environment unusable merely because this extra model could
                // not be downloaded. The transcriber will fall back to Stable Prefix
                // presentation regrouping when these paths remain empty.
                send(
                    &on_event,
                    NativeInstallEvent::ItemFinished {
                        label: "English CTC 可选修复模型未安装（已跳过，可稍后重试）".into(),
                        path: error,
                        index: next_index,
                        total_items,
                    },
                )?;
            }
        }
        next_index += 1;

    }

    send(
        &on_event,
        NativeInstallEvent::ItemStarted {
            label: "FFmpeg / FFprobe".into(),
            repo: "BtbN FFmpeg Builds · latest".into(),
            index: next_index,
            total_items,
        },
    )?;
    let (ffmpeg, ffprobe) = match ensure_ffmpeg(
        &client,
        &root,
        &legacy,
        &on_event,
        next_index,
        total_items,
    )
    .await
    {
        Ok(v) => v,
        Err(error) => {
            let _ = send(&on_event, NativeInstallEvent::Error { message: error.clone() });
            return Err(error);
        }
    };
    send(
        &on_event,
        NativeInstallEvent::ItemFinished {
            label: "FFmpeg / FFprobe".into(),
            path: ffmpeg.to_string_lossy().into_owned(),
            index: next_index,
            total_items,
        },
    )?;

    let final_status = status(&app, &request)?;
    if !final_status.ready {
        let message = format!("安装结束但仍缺少：{}", final_status.missing.join("、"));
        let _ = send(&on_event, NativeInstallEvent::Error { message: message.clone() });
        return Err(message);
    }
    let mut paths = final_status.paths.clone();
    paths.ffmpeg_path = ffmpeg.to_string_lossy().into_owned();
    paths.ffprobe_path = ffprobe.to_string_lossy().into_owned();
    send(&on_event, NativeInstallEvent::Finished { paths: paths.clone() })?;
    Ok(paths)
}

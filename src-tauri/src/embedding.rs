//! Embedding model management and CPU vector inference for local semantic search.
//!
//! Uses native ONNX Runtime via `ort` + `tokenizers` (Qwen3-Embedding-0.6B),
//! running locally on CPU in tens of milliseconds without external GPU or Python dependencies.

use crate::asr;
use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;
use ort::value::Tensor;
use tokenizers::Tokenizer;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

pub const DEFAULT_EMBEDDING_MODEL_ID: &str = "qwen3-embedding-0.6b";
pub const MODEL_NAME: &str = "Qwen3 Embedding 0.6B (通义千问语义大模型)";
pub const MODEL_SIZE_LABEL: &str = "约 595 MiB";
#[allow(dead_code)]
pub const VECTOR_DIM: usize = 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingModelStatus {
    pub id: &'static str,
    pub name: &'static str,
    pub installed: bool,
    pub file_size: Option<u64>,
    pub size_label: &'static str,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelDownloadProgress {
    model_id: &'static str,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
    progress: u8,
    message: String,
}

struct DownloadFileTarget {
    filename: &'static str,
    mirror_url: &'static str,
    official_url: &'static str,
    expected_size: u64,
}

const MODEL_TARGET_FILES: &[DownloadFileTarget] = &[
    DownloadFileTarget {
        filename: "model_quantized.onnx",
        mirror_url: "https://hf-mirror.com/onnx-community/Qwen3-Embedding-0.6B-ONNX/resolve/main/onnx/model_quantized.onnx",
        official_url: "https://huggingface.co/onnx-community/Qwen3-Embedding-0.6B-ONNX/resolve/main/onnx/model_quantized.onnx",
        expected_size: 613_527_631,
    },
    DownloadFileTarget {
        filename: "tokenizer.json",
        mirror_url: "https://hf-mirror.com/onnx-community/Qwen3-Embedding-0.6B-ONNX/resolve/main/tokenizer.json",
        official_url: "https://huggingface.co/onnx-community/Qwen3-Embedding-0.6B-ONNX/resolve/main/tokenizer.json",
        expected_size: 11_423_705,
    },
    DownloadFileTarget {
        filename: "config.json",
        mirror_url: "https://hf-mirror.com/onnx-community/Qwen3-Embedding-0.6B-ONNX/resolve/main/config.json",
        official_url: "https://huggingface.co/onnx-community/Qwen3-Embedding-0.6B-ONNX/resolve/main/config.json",
        expected_size: 1_576,
    },
    DownloadFileTarget {
        filename: "special_tokens_map.json",
        mirror_url: "https://hf-mirror.com/onnx-community/Qwen3-Embedding-0.6B-ONNX/resolve/main/special_tokens_map.json",
        official_url: "https://huggingface.co/onnx-community/Qwen3-Embedding-0.6B-ONNX/resolve/main/special_tokens_map.json",
        expected_size: 613,
    },
    DownloadFileTarget {
        filename: "tokenizer_config.json",
        mirror_url: "https://hf-mirror.com/onnx-community/Qwen3-Embedding-0.6B-ONNX/resolve/main/tokenizer_config.json",
        official_url: "https://huggingface.co/onnx-community/Qwen3-Embedding-0.6B-ONNX/resolve/main/tokenizer_config.json",
        expected_size: 9_731,
    },
];

pub fn total_model_bytes() -> u64 {
    MODEL_TARGET_FILES.iter().map(|f| f.expected_size).sum()
}

pub fn models_dir(app_data_dir: &Path) -> PathBuf {
    asr::models_dir(app_data_dir)
}

pub fn model_cache_dir(app_data_dir: &Path) -> PathBuf {
    models_dir(app_data_dir).join("qwen3-embedding-0.6b")
}

pub fn model_status(app_data_dir: &Path) -> EmbeddingModelStatus {
    let cache_dir = model_cache_dir(app_data_dir);
    let is_installed = is_model_cached(&cache_dir);
    EmbeddingModelStatus {
        id: DEFAULT_EMBEDDING_MODEL_ID,
        name: MODEL_NAME,
        installed: is_installed,
        file_size: if is_installed { Some(total_model_bytes()) } else { None },
        size_label: MODEL_SIZE_LABEL,
        path: cache_dir.to_string_lossy().into_owned(),
    }
}

fn is_model_cached(cache_dir: &Path) -> bool {
    if !cache_dir.is_dir() {
        return false;
    }
    MODEL_TARGET_FILES.iter().all(|target| {
        let path = cache_dir.join(target.filename);
        path.is_file() && fs::metadata(&path).map(|m| m.len() > 0).unwrap_or(false)
    })
}

fn emit_download_progress(
    app: &AppHandle,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
    message: impl Into<String>,
) {
    let progress = total_bytes
        .filter(|total| *total > 0)
        .map(|total| ((downloaded_bytes.saturating_mul(100) / total).min(100)) as u8)
        .unwrap_or(0);
    let _ = app.emit(
        "model-download-progress",
        ModelDownloadProgress {
            model_id: DEFAULT_EMBEDDING_MODEL_ID,
            downloaded_bytes,
            total_bytes,
            progress,
            message: message.into(),
        },
    );
}

fn download_single_file(
    app: &AppHandle,
    dest_path: &Path,
    target: &DownloadFileTarget,
    base_downloaded: u64,
    total_all_bytes: u64,
) -> Result<(), String> {
    if dest_path.is_file() {
        if let Ok(meta) = fs::metadata(dest_path) {
            if meta.len() > 0 {
                println!("[embedding] 文件 {} 已存在且非空，跳过下载", target.filename);
                return Ok(());
            }
        }
    }

    let partial = dest_path.with_extension("download");

    let sources = [
        ("国内镜像 (hf-mirror.com)", target.mirror_url),
        ("官方源 (huggingface.co)", target.official_url),
    ];

    let mut last_error = String::new();
    for (source_name, url) in sources {
        println!("[embedding] 正在通过 {source_name} 下载文件: {}", target.filename);
        let mut existing = fs::metadata(&partial).map(|m| m.len()).unwrap_or(0);
        let is_large_file = target.expected_size >= 1_000_000;
        let use_resume = is_large_file && existing > 0 && existing < target.expected_size;

        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(12))
            .timeout_read(Duration::from_secs(60))
            .build();

        let mut request = agent.get(url).set("User-Agent", "VideoNotes/0.1");
        if use_resume {
            request = request.set("Range", &format!("bytes={existing}-"));
        }

        emit_download_progress(
            app,
            base_downloaded + (if use_resume { existing } else { 0 }),
            Some(total_all_bytes),
            format!("正在通过{}下载 {}……", source_name, target.filename),
        );

        let response = match request.call() {
            Ok(resp) => resp,
            Err(ureq::Error::Status(416, _)) if use_resume => {
                println!("[embedding] 收到 416，重置断点从头下载 {}", target.filename);
                let _ = fs::remove_file(&partial);
                existing = 0;
                let retry_request = agent.get(url).set("User-Agent", "VideoNotes/0.1");
                match retry_request.call() {
                    Ok(resp) => resp,
                    Err(e) => {
                        last_error = format!("{source_name} 重试失败: {e}");
                        eprintln!("[embedding] {last_error}");
                        continue;
                    }
                }
            }
            Err(e) => {
                last_error = format!("{source_name} 请求失败: {e}");
                eprintln!("[embedding] {last_error}");
                continue;
            }
        };

        let is_206 = response.status() == 206;
        let is_append = use_resume && is_206;
        let start = if is_append { existing } else { 0 };

        let mut output = match OpenOptions::new()
            .create(true)
            .write(true)
            .append(is_append)
            .truncate(!is_append)
            .open(&partial)
        {
            Ok(f) => f,
            Err(e) => return Err(format!("无法写入文件 {}: {e}", target.filename)),
        };

        let mut reader = response.into_reader();
        let mut downloaded_current = start;
        let mut buffer = vec![0_u8; 128 * 1024];
        let mut download_ok = true;

        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => {
                    if let Err(e) = output.write_all(&buffer[..count]) {
                        last_error = format!("写入中断: {e}");
                        download_ok = false;
                        break;
                    }
                    downloaded_current += count as u64;
                    emit_download_progress(
                        app,
                        base_downloaded + downloaded_current,
                        Some(total_all_bytes),
                        format!("正在通过{}下载 {}……", source_name, target.filename),
                    );
                }
                Err(e) => {
                    last_error = format!("读取中断: {e}");
                    download_ok = false;
                    break;
                }
            }
        }

        if download_ok && downloaded_current > 0 {
            let _ = output.flush();
            drop(output);
            if fs::rename(&partial, dest_path).is_err() {
                let _ = fs::copy(&partial, dest_path);
                let _ = fs::remove_file(&partial);
            }
            println!("[embedding] 文件 {} 下载成功 (大小: {downloaded_current} 字节)", target.filename);
            return Ok(());
        }
    }

    Err(format!("未能下载文件 {}: {last_error}", target.filename))
}

pub async fn download_model(app: &AppHandle, model_data_dir: &Path) -> Result<(), String> {
    let cache_dir = model_cache_dir(model_data_dir);
    fs::create_dir_all(&cache_dir).map_err(|error| format!("无法创建向量模型目录：{error}"))?;

    let total_all = total_model_bytes();
    println!(
        "[embedding] 开始下载 BGE 语义检索模型, 目标目录: {}, 总大小: {} 字节",
        cache_dir.display(),
        total_all
    );

    emit_download_progress(app, 0, Some(total_all), "正在连接镜像源并准备下载……");

    let app_handle = app.clone();
    let cache_dir_clone = cache_dir.clone();
    let model_data_dir_buf = model_data_dir.to_path_buf();

    tauri::async_runtime::spawn_blocking(move || {
        let mut accumulated_bytes = 0;
        for target in MODEL_TARGET_FILES {
            let dest_path = cache_dir_clone.join(target.filename);
            download_single_file(&app_handle, &dest_path, target, accumulated_bytes, total_all)?;
            accumulated_bytes += target.expected_size;
        }

        println!("[embedding] 所有模型文件下载完成，正在预热加载引擎...");
        emit_download_progress(&app_handle, total_all, Some(total_all), "正在初始化语义向量引擎……");

        let engine = get_or_load_engine(&model_data_dir_buf)?;
        let lock = EMBEDDING_ENGINE.get_or_init(|| Mutex::new(None));
        if let Ok(mut guard) = lock.lock() {
            *guard = Some(engine);
        }

        emit_download_progress(&app_handle, total_all, Some(total_all), "下载完成");
        println!("[embedding] BGE 语义模型安装与预热成功！");
        Ok::<(), String>(())
    })
    .await
    .map_err(|error| format!("下载任务线程异常：{error}"))?
}

pub struct QwenEmbeddingEngine {
    session: Session,
    tokenizer: Tokenizer,
}

static EMBEDDING_ENGINE: OnceLock<Mutex<Option<QwenEmbeddingEngine>>> = OnceLock::new();

impl QwenEmbeddingEngine {
    pub fn load(model_data_dir: &Path) -> Result<Self, String> {
        let cache_dir = model_cache_dir(model_data_dir);
        let onnx_path = cache_dir.join("model_quantized.onnx");
        let tokenizer_path = cache_dir.join("tokenizer.json");

        if !onnx_path.is_file() || !tokenizer_path.is_file() {
            return Err("Qwen3 语义模型文件未就绪，请先下载模型".to_string());
        }

        println!("[embedding] >>> 正在加载 Qwen3 ONNX 语义模型 (路径: {:?})...", onnx_path);
        let load_start = std::time::Instant::now();

        let tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| format!("加载 tokenizer.json 失败：{e}"))?;

        let session = Session::builder()
            .map_err(|e| format!("创建 ONNX 会话构建器失败：{e}"))?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|e| format!("设置 ONNX 优化级别失败：{e}"))?
            .with_intra_threads(4)
            .map_err(|e| format!("设置 ONNX 线程数失败：{e}"))?
            .commit_from_file(&onnx_path)
            .map_err(|e| format!("加载 Qwen3 ONNX 模型失败：{e}"))?;

        println!("[embedding] >>> Qwen3 ONNX 引擎与 Tokenizer 加载成功！耗时: {:?}", load_start.elapsed());
        Ok(Self { session, tokenizer })
    }

    pub fn embed_batch(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let total_start = std::time::Instant::now();
        let mut results = Vec::with_capacity(texts.len());

        for text in texts {
            let encoding = self
                .tokenizer
                .encode(text.as_str(), true)
                .map_err(|e| format!("分词编码失败：{e}"))?;

            let input_ids_u32 = encoding.get_ids();
            let seq_len = input_ids_u32.len();
            if seq_len == 0 {
                results.push(vec![0.0_f32; VECTOR_DIM]);
                continue;
            }

            let input_ids: Vec<i64> = input_ids_u32.iter().map(|&id| id as i64).collect();
            let attention_mask: Vec<i64> = encoding
                .get_attention_mask()
                .iter()
                .map(|&m| m as i64)
                .collect();
            let position_ids: Vec<i64> = (0..seq_len as i64).collect();

            let shape = [1, seq_len];
            let input_ids_tensor = Tensor::from_array((shape, input_ids))
                .map_err(|e| format!("创建 input_ids 张量失败：{e}"))?;
            let attention_mask_tensor = Tensor::from_array((shape, attention_mask))
                .map_err(|e| format!("创建 attention_mask 张量失败：{e}"))?;
            let position_ids_tensor = Tensor::from_array((shape, position_ids))
                .map_err(|e| format!("创建 position_ids 张量失败：{e}"))?;

            let mut inputs: Vec<(std::borrow::Cow<'_, str>, ort::session::SessionInputValue<'_>)> =
                Vec::with_capacity(3 + 56);
            inputs.push((std::borrow::Cow::Borrowed("input_ids"), input_ids_tensor.into()));
            inputs.push((std::borrow::Cow::Borrowed("attention_mask"), attention_mask_tensor.into()));
            inputs.push((std::borrow::Cow::Borrowed("position_ids"), position_ids_tensor.into()));

            for i in 0..28 {
                let key_name = format!("past_key_values.{i}.key");
                let val_name = format!("past_key_values.{i}.value");
                let k_tensor = Tensor::from_array(([1, 8, 0, 128], Vec::<f32>::new()))
                    .map_err(|e| format!("创建 {key_name} 张量失败：{e}"))?;
                let v_tensor = Tensor::from_array(([1, 8, 0, 128], Vec::<f32>::new()))
                    .map_err(|e| format!("创建 {val_name} 张量失败：{e}"))?;
                inputs.push((std::borrow::Cow::Owned(key_name), k_tensor.into()));
                inputs.push((std::borrow::Cow::Owned(val_name), v_tensor.into()));
            }

            let outputs = self
                .session
                .run(inputs)
                .map_err(|e| format!("Qwen3 ONNX 推理计算失败：{e}"))?;

            let (extracted_shape, data) = outputs["last_hidden_state"]
                .try_extract_tensor::<f32>()
                .map_err(|e| format!("解析模型输出张量失败：{e}"))?;

            let hidden_dim = if extracted_shape.len() >= 3 {
                extracted_shape[2] as usize
            } else {
                VECTOR_DIM
            };

            // 取有效序列的最后一个 token 向量 (LastToken pooling)
            let last_token_idx = seq_len.saturating_sub(1);
            let start = last_token_idx * hidden_dim;
            let end = start + hidden_dim;

            if end <= data.len() {
                let mut vec = data[start..end].to_vec();
                let norm: f32 = vec.iter().map(|v| v * v).sum::<f32>().sqrt();
                if norm > 0.0 {
                    for v in &mut vec {
                        *v /= norm;
                    }
                }
                results.push(vec);
            } else {
                results.push(vec![0.0_f32; VECTOR_DIM]);
            }
        }

        if let Some(first) = results.first() {
            println!(
                "[embedding] Qwen3 成功提取 {} 条文本语义向量 | 耗时: {:?} | 维度: {} | 样例首项: [{:.4}, {:.4}, {:.4}, ...]",
                texts.len(),
                total_start.elapsed(),
                first.len(),
                first.first().copied().unwrap_or(0.0),
                first.get(1).copied().unwrap_or(0.0),
                first.get(2).copied().unwrap_or(0.0)
            );
        }

        Ok(results)
    }
}

pub fn delete_model(model_data_dir: &Path) -> Result<(), String> {
    if let Some(lock) = EMBEDDING_ENGINE.get() {
        if let Ok(mut guard) = lock.lock() {
            *guard = None;
        }
    }
    let cache_dir = model_cache_dir(model_data_dir);
    if cache_dir.is_dir() {
        fs::remove_dir_all(&cache_dir).map_err(|error| format!("无法删除向量模型：{error}"))?;
    }
    Ok(())
}

fn get_or_load_engine(model_data_dir: &Path) -> Result<QwenEmbeddingEngine, String> {
    QwenEmbeddingEngine::load(model_data_dir)
}

/// 批量提取文本的稠密语义向量
pub fn embed_texts(model_data_dir: &Path, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
    if texts.is_empty() {
        return Ok(Vec::new());
    }

    let lock = EMBEDDING_ENGINE.get_or_init(|| Mutex::new(None));
    let mut guard = lock
        .lock()
        .map_err(|_| "向量引擎锁不可用".to_string())?;

    if guard.is_none() {
        *guard = Some(get_or_load_engine(model_data_dir)?);
    }

    let engine = guard.as_mut().ok_or("向量引擎未就绪")?;
    engine.embed_batch(texts)
}

/// 提取单个查询词的语义向量
pub fn embed_query(model_data_dir: &Path, query: &str) -> Result<Vec<f32>, String> {
    let clean_query = query.trim().to_string();
    println!("[embedding] >>> 正在为搜索词计算 Qwen3 语义特征向量: \"{}\"", clean_query);
    let mut results = embed_texts(model_data_dir, &[clean_query])?;
    results
        .pop()
        .ok_or_else(|| "未能生成查询向量".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn reports_not_installed_for_empty_dir() {
        let dir = tempdir().unwrap();
        let status = model_status(dir.path());
        assert_eq!(status.id, DEFAULT_EMBEDDING_MODEL_ID);
        assert!(!status.installed);
    }
}

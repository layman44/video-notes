use crate::asr::{self, TranscriptResult};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};
use tauri::{AppHandle, Emitter};

pub const DEFAULT_TRANSLATION_MODEL_ID: &str = "milmmt-46-1b-q4_k_m";
pub const MODEL_NAME: &str = "MiLMMT 46 1B Q4_K_M (极速翻译)";
pub const MODEL_FILE: &str = "MiLMMT-46-1B-v1.0.Q4_K_M.gguf";
pub const MODEL_SIZE_BYTES: u64 = 806_057_408;
pub const MODEL_SIZE_LABEL: &str = "约 768 MiB";

#[derive(Debug, Clone, Copy)]
struct ModelSource {
    name: &'static str,
    url: &'static str,
}

const MODEL_SOURCES: &[ModelSource] = &[
    ModelSource {
        name: "国内镜像（HF-Mirror）",
        url: "https://hf-mirror.com/mradermacher/MiLMMT-46-1B-v1.0-GGUF/resolve/main/MiLMMT-46-1B-v1.0.Q4_K_M.gguf?download=true",
    },
    ModelSource {
        name: "官方源（Hugging Face）",
        url: "https://huggingface.co/mradermacher/MiLMMT-46-1B-v1.0-GGUF/resolve/main/MiLMMT-46-1B-v1.0.Q4_K_M.gguf?download=true",
    },
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationModelStatus {
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

pub fn models_dir(app_data_dir: &Path) -> PathBuf {
    asr::models_dir(app_data_dir)
}

pub fn model_path(app_data_dir: &Path) -> PathBuf {
    models_dir(app_data_dir).join(MODEL_FILE)
}

pub fn model_status(app_data_dir: &Path) -> TranslationModelStatus {
    let path = model_path(app_data_dir);
    let metadata = fs::metadata(&path)
        .ok()
        .filter(|value| value.is_file() && value.len() == MODEL_SIZE_BYTES);
    TranslationModelStatus {
        id: DEFAULT_TRANSLATION_MODEL_ID,
        name: MODEL_NAME,
        installed: metadata.is_some(),
        file_size: metadata.map(|value| value.len()),
        size_label: MODEL_SIZE_LABEL,
        path: path.to_string_lossy().into_owned(),
    }
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
            model_id: DEFAULT_TRANSLATION_MODEL_ID,
            downloaded_bytes,
            total_bytes,
            progress,
            message: message.into(),
        },
    );
}

fn content_range_total(value: &str) -> Option<u64> {
    value.rsplit('/').next()?.parse().ok()
}

fn download_from_source(
    app: &AppHandle,
    partial: &Path,
    source: ModelSource,
) -> Result<u64, String> {
    let existing = fs::metadata(partial).map(|value| value.len()).unwrap_or(0);
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(12))
        .timeout_read(Duration::from_secs(60))
        .build();
    let mut request = agent.get(source.url).set("User-Agent", "VideoNotes/0.1");
    if existing > 0 {
        request = request.set("Range", &format!("bytes={existing}-"));
    }
    emit_download_progress(
        app,
        existing,
        Some(MODEL_SIZE_BYTES),
        if existing > 0 {
            format!("正在通过{}继续下载……", source.name)
        } else {
            format!("正在通过{}下载……", source.name)
        },
    );
    let response = request
        .call()
        .map_err(|error| format!("{}连接失败：{error}", source.name))?;
    let is_resume = existing > 0 && response.status() == 206;
    let start = if is_resume { existing } else { 0 };
    let total = response
        .header("Content-Range")
        .and_then(content_range_total)
        .or_else(|| {
            response
                .header("Content-Length")
                .and_then(|value| value.parse::<u64>().ok())
                .map(|remaining| start + remaining)
        })
        .unwrap_or(MODEL_SIZE_BYTES);
    if total != MODEL_SIZE_BYTES {
        return Err(format!("{}返回了异常文件大小：{total}", source.name));
    }
    let mut output = OpenOptions::new()
        .create(true)
        .write(true)
        .append(is_resume)
        .truncate(!is_resume)
        .open(partial)
        .map_err(|error| format!("无法写入模型文件：{error}"))?;
    let mut reader = response.into_reader();
    let mut downloaded = start;
    let mut buffer = vec![0_u8; 256 * 1024];
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|error| format!("{}下载中断：{error}", source.name))?;
        if count == 0 {
            break;
        }
        output
            .write_all(&buffer[..count])
            .map_err(|error| format!("无法保存模型文件：{error}"))?;
        downloaded += count as u64;
        emit_download_progress(
            app,
            downloaded,
            Some(MODEL_SIZE_BYTES),
            format!("正在通过{}下载……", source.name),
        );
    }
    output
        .flush()
        .map_err(|error| format!("无法完成模型写入：{error}"))?;
    Ok(downloaded)
}

pub fn download_default_model(
    app: &AppHandle,
    app_data_dir: &Path,
) -> Result<TranslationModelStatus, String> {
    let directory = models_dir(app_data_dir);
    fs::create_dir_all(&directory).map_err(|error| format!("无法创建模型目录：{error}"))?;
    let target = model_path(app_data_dir);
    if model_status(app_data_dir).installed {
        return Ok(model_status(app_data_dir));
    }
    let partial = directory.join(format!("{MODEL_FILE}.part"));
    let partial_size = fs::metadata(&partial).map(|value| value.len()).unwrap_or(0);
    if partial_size > MODEL_SIZE_BYTES {
        fs::remove_file(&partial).map_err(|error| format!("无法清理异常模型文件：{error}"))?;
    } else if partial_size == MODEL_SIZE_BYTES {
        fs::rename(&partial, &target).map_err(|error| format!("无法安装模型文件：{error}"))?;
        return Ok(model_status(app_data_dir));
    }

    let mut failures = Vec::new();
    for source in MODEL_SOURCES.iter().copied() {
        match download_from_source(app, &partial, source) {
            Ok(downloaded) if downloaded == MODEL_SIZE_BYTES => {
                fs::rename(&partial, &target)
                    .map_err(|error| format!("无法安装模型文件：{error}"))?;
                emit_download_progress(
                    app,
                    downloaded,
                    Some(MODEL_SIZE_BYTES),
                    "专用翻译模型安装完成",
                );
                return Ok(model_status(app_data_dir));
            }
            Ok(_) => {
                failures.push(format!("{}下载未完成全部字节", source.name));
            }
            Err(error) => failures.push(error),
        }
    }
    Err(format!("翻译模型下载失败：{}", failures.join("；")))
}

pub fn remove_default_model(app_data_dir: &Path) -> Result<TranslationModelStatus, String> {
    let target = model_path(app_data_dir);
    let partial = models_dir(app_data_dir).join(format!("{MODEL_FILE}.part"));
    if target.is_file() {
        fs::remove_file(&target).map_err(|error| format!("无法删除模型文件：{error}"))?;
    }
    if partial.is_file() {
        let _ = fs::remove_file(&partial);
    }
    Ok(model_status(app_data_dir))
}

pub fn language_name(code: &str) -> &'static str {
    match code.to_ascii_lowercase().as_str() {
        "en" => "English",
        "ja" => "Japanese",
        "ko" => "Korean",
        "fr" => "French",
        "de" => "German",
        "ru" => "Russian",
        "es" => "Spanish",
        "it" => "Italian",
        "pt" => "Portuguese",
        "ar" => "Arabic",
        "th" => "Thai",
        "vi" => "Vietnamese",
        "id" => "Indonesian",
        _ => "English",
    }
}

pub fn translation_prompt_milmmt(
    transcript: &TranscriptResult,
    segment_index: usize,
) -> Result<String, String> {
    let segment = transcript
        .segments
        .get(segment_index)
        .ok_or_else(|| format!("翻译目标索引越界：{segment_index}"))?;
    let src_lang = language_name(&transcript.language);
    let source = clean_prompt_text(&segment.text);
    if source.is_empty() {
        return Err("翻译目标为空".to_string());
    }

    // MiLMMT-46's official prompt is intentionally simple. Keep segment identity
    // and alignment entirely in application code; the translation model only sees
    // one semantic Standard segment at a time.
    Ok(format!(
        "Translate this from {src_lang} to Chinese (Simplified):\n\
         {src_lang}: {source}\n\
         Chinese (Simplified):"
    ))
}

/// Remove role/template framing that some llama.cpp builds include around the
/// generated translation. Only markers at the beginning (and known stop
/// markers at the end) are removed; a normal `Assistant:` in the body is
/// deliberately left untouched.
pub fn clean_milmmt_translation_output(raw_output: &str) -> String {
    let mut text = raw_output.trim();

    // If output contains a conversation transcript starting with User: / <|user|>,
    // advance directly to the Assistant turn.
    let trimmed = text.trim_start_matches('\u{feff}').trim_start();
    if let Some(pos) = find_assistant_turn(trimmed) {
        text = &trimmed[pos..];
    }

    // A model can echo more than one nested role/template marker. Every
    // successful branch consumes input, so this loop always makes progress.
    loop {
        let before = text;
        text = text.trim_start();

        if let Some(rest) = strip_prefix_ascii_ci(text, "<|assistant|>") {
            text = rest;
        } else if let Some(rest) = strip_prefix_ascii_ci(text, "<|im_start|>") {
            text = strip_template_role(rest);
        } else if let Some(rest) = strip_prefix_ascii_ci(text, "<|start_header_id|>") {
            text = strip_template_role(rest);
        } else if let Some(rest) = strip_prefix_ascii_ci(text, "<|end_header_id|>") {
            text = rest;
        } else if let Some(rest) = strip_prefix_ascii_ci(text, "###") {
            // Common markdown chat template: `### Assistant:`.
            let candidate = rest.trim_start();
            if let Some(clean) = strip_assistant_prefix(candidate) {
                text = clean;
            } else {
                break;
            }
        } else if let Some(rest) = strip_assistant_prefix(text) {
            text = rest;
        } else if let Some(rest) = strip_prefix_ascii_ci(text, "Chinese (Simplified):") {
            text = rest;
        } else {
            break;
        }

        if text == before {
            break;
        }
    }

    // Do not strip arbitrary angle-bracket text. These are the termination
    // tokens emitted by the supported chat templates, and only at the end.
    loop {
        let trimmed = text.trim_end();
        let Some(rest) = ["<|eot_id|>", "<|end|>", "<|im_end|>", "</s>"]
            .iter()
            .find_map(|token| strip_suffix_ascii_ci(trimmed, token))
        else {
            text = trimmed;
            break;
        };
        text = rest.trim_end();
    }

    text.trim_matches('`')
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn find_assistant_turn(text: &str) -> Option<usize> {
    let trimmed = text.trim_start();
    if strip_prefix_ascii_ci(trimmed, "user:").is_some()
        || strip_prefix_ascii_ci(trimmed, "<|user|>").is_some()
        || strip_prefix_ascii_ci(trimmed, "<|im_start|>user").is_some()
        || strip_prefix_ascii_ci(trimmed, "### user").is_some()
    {
        for marker in &["assistant:", "assistant：", "<|assistant|>", "<|im_start|>assistant", "<|start_header_id|>assistant", "### assistant"] {
            if let Some(idx) = text.to_ascii_lowercase().find(marker) {
                return Some(idx);
            }
        }
    }
    None
}

fn strip_prefix_ascii_ci<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    text.get(..prefix.len())
        .filter(|value| value.eq_ignore_ascii_case(prefix))
        .map(|_| &text[prefix.len()..])
}

fn strip_suffix_ascii_ci<'a>(text: &'a str, suffix: &str) -> Option<&'a str> {
    let start = text.len().checked_sub(suffix.len())?;
    text.get(start..)
        .filter(|value| value.eq_ignore_ascii_case(suffix))
        .map(|_| &text[..start])
}

fn strip_assistant_prefix(text: &str) -> Option<&str> {
    let rest = strip_prefix_ascii_ci(text.trim_start(), "assistant")?;
    let rest = rest.trim_start_matches([' ', '\t', '\r', '\n']);
    let rest = rest.strip_prefix(':').or_else(|| rest.strip_prefix('：'))?;
    Some(rest)
}

fn strip_template_role(text: &str) -> &str {
    let trimmed = text.trim_start();
    let Some(rest) = strip_prefix_ascii_ci(trimmed, "assistant") else {
        return trimmed;
    };
    let rest = rest.trim_start_matches([' ', '\t', '\r', '\n']);
    strip_prefix_ascii_ci(rest, "<|end_header_id|>").unwrap_or(rest)
}

fn clean_prompt_text(text: &str) -> String {
    text.replace(|c: char| c == '\r' || c == '\n', " ").split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_language_codes_to_standard_names() {
        assert_eq!(language_name("en"), "English");
        assert_eq!(language_name("ja"), "Japanese");
        assert_eq!(language_name("ko"), "Korean");
        assert_eq!(language_name("fr"), "French");
        assert_eq!(language_name("unknown"), "English");
    }

    #[test]
    fn formats_milmmt_prompt() {
        let transcript = TranscriptResult {
            job_id: "test".to_string(),
            model_id: "test".to_string(),
            language: "en".to_string(),
            translation_language: None,
            text: "Hello world".to_string(),
            segments: vec![asr::TranscriptSegment {
                id: "0".to_string(),
                chunk_index: 0,
                start: 0.0,
                end: 1.0,
                start_ms: 0,
                end_ms: 1000,
                text: "Hello world".to_string(),
                translated_text: None,
                avg_confidence: None,
            }],
            pause_repairs: None,
        };
        let prompt = translation_prompt_milmmt(&transcript, 0).unwrap();
        assert_eq!(
            prompt,
            "Translate this from English to Chinese (Simplified):\nEnglish: Hello world\nChinese (Simplified):"
        );
        assert!(!prompt.contains("SEG:"));
        assert!(!prompt.contains("CONTEXT"));
    }

    #[test]
    fn cleans_assistant_prefix_and_optional_whitespace() {
        assert_eq!(
            clean_milmmt_translation_output("Assistant: 我是个丑陋的鸭子。"),
            "我是个丑陋的鸭子。"
        );
        assert_eq!(
            clean_milmmt_translation_output("assistant\n:\n我是个丑陋的鸭子。"),
            "我是个丑陋的鸭子。"
        );
    }

    #[test]
    fn cleans_nested_chat_template_markers_and_is_idempotent() {
        let raw = "<|im_start|>assistant\n<|assistant|> Chinese (Simplified):\n我是个丑陋的鸭子。<|im_end|>";
        let clean = clean_milmmt_translation_output(raw);
        assert_eq!(clean, "我是个丑陋的鸭子。");
        assert_eq!(clean_milmmt_translation_output(&clean), clean);
        assert_eq!(
            clean_milmmt_translation_output("<|start_header_id|>assistant<|end_header_id|>\n你好。<|eot_id|>"),
            "你好。"
        );
    }

    #[test]
    fn preserves_assistant_text_in_the_body() {
        let raw = "他说：Assistant: 这不是角色前缀。";
        assert_eq!(clean_milmmt_translation_output(raw), raw);
        assert_eq!(clean_milmmt_translation_output("assistant body"), "assistant body");
        assert_eq!(clean_milmmt_translation_output("### 正文标题"), "### 正文标题");
    }

    #[test]
    fn cleans_target_label_only_at_the_beginning() {
        assert_eq!(
            clean_milmmt_translation_output("Chinese (Simplified): 她冲到外面呼救。"),
            "她冲到外面呼救。"
        );
        let body = "译文提到 Chinese (Simplified): 作为正文。";
        assert_eq!(clean_milmmt_translation_output(body), body);
    }

    #[test]
    fn cleans_user_assistant_chat_output() {
        let raw = "User:\n\u{feff}Translate this from English to Chinese (Simplified):\nEnglish: An ugly duck thing.\nChinese (Simplified):\n\nAssistant:\n真是个丑陋的鸭子。\n";
        assert_eq!(clean_milmmt_translation_output(raw), "真是个丑陋的鸭子。");
    }
}

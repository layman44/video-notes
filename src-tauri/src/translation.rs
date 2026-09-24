use crate::{
    asr::{self, TranscriptResult},
    summary::{self, SummaryModelStatus},
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::AppHandle;

#[allow(dead_code)]
pub const DEFAULT_TRANSLATION_MODEL_ID: &str = summary::DEFAULT_MODEL_ID;
pub const MODEL_NAME: &str = "Qwen3.5 2B Q4_K_M (总结与翻译)";
#[allow(dead_code)]
pub const MODEL_FILE: &str = "Qwen3.5-2B-Q4_K_M.gguf";
#[allow(dead_code)]
pub const MODEL_SIZE_BYTES: u64 = 1_280_835_840;
#[allow(dead_code)]
pub const MODEL_SIZE_LABEL: &str = "约 1.19 GiB";

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

impl From<SummaryModelStatus> for TranslationModelStatus {
    fn from(s: SummaryModelStatus) -> Self {
        Self {
            id: s.id,
            name: MODEL_NAME,
            installed: s.installed,
            file_size: s.file_size,
            size_label: s.size_label,
            path: s.path,
        }
    }
}

#[allow(dead_code)]
pub fn models_dir(app_data_dir: &Path) -> PathBuf {
    asr::models_dir(app_data_dir)
}

#[allow(dead_code)]
pub fn model_path(app_data_dir: &Path) -> PathBuf {
    summary::model_status(app_data_dir).path.into()
}

pub fn model_status(app_data_dir: &Path) -> TranslationModelStatus {
    summary::model_status(app_data_dir).into()
}

pub fn download_default_model(
    app: &AppHandle,
    app_data_dir: &Path,
) -> Result<TranslationModelStatus, String> {
    summary::download_default_model(app, app_data_dir).map(Into::into)
}

pub fn remove_default_model(app_data_dir: &Path) -> Result<TranslationModelStatus, String> {
    summary::delete_default_model(app_data_dir)?;
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

pub fn translation_prompt_qwen(
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

    Ok(format!(
        "/no_think\n\
         你是一位专业的多语言翻译专家。请将以下文本准确、流畅地翻译为中文（简体）。\n\
         【翻译要求】\n\
         1. 忠实原文：准确传达原句含义、数值、专有名词与标点语气；\n\
         2. 通顺自然：符合中文表达习惯，语言凝练自然；\n\
         3. 纯净输出：仅输出最终的中文译文内容，严禁输出任何解释、说明、前缀（如“译文：”、“中文：”）或原文字符。\n\
         原文（{src_lang}）：{source}\n\
         中文译文："
    ))
}

#[allow(dead_code)]
pub fn translation_prompt_milmmt(
    transcript: &TranscriptResult,
    segment_index: usize,
) -> Result<String, String> {
    translation_prompt_qwen(transcript, segment_index)
}

pub fn clean_translation_output(raw_output: &str) -> String {
    let mut text = raw_output.trim();

    let trimmed = text.trim_start_matches('\u{feff}').trim_start();
    if let Some(pos) = find_assistant_turn(trimmed) {
        text = &trimmed[pos..];
    }

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
            let mut matched_custom = false;
            for prefix in &[
                "中文译文：", "中文译文:", "中文：", "中文:", "译文：", "译文:", "翻译：", "翻译:",
            ] {
                if let Some(rest) = text.strip_prefix(prefix) {
                    text = rest;
                    matched_custom = true;
                    break;
                }
            }
            if !matched_custom {
                break;
            }
        }

        if text == before {
            break;
        }
    }

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

pub fn clean_milmmt_translation_output(raw_output: &str) -> String {
    clean_translation_output(raw_output)
}

fn find_assistant_turn(text: &str) -> Option<usize> {
    let trimmed = text.trim_start();
    if strip_prefix_ascii_ci(trimmed, "user:").is_some()
        || strip_prefix_ascii_ci(trimmed, "<|user|>").is_some()
        || strip_prefix_ascii_ci(trimmed, "<|im_start|>user").is_some()
        || strip_prefix_ascii_ci(trimmed, "### user").is_some()
    {
        for marker in &[
            "assistant:",
            "assistant：",
            "<|assistant|>",
            "<|im_start|>assistant",
            "<|start_header_id|>assistant",
            "### assistant",
        ] {
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
    text.replace(|c: char| c == '\r' || c == '\n', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
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
    fn formats_qwen_translation_prompt() {
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
        let prompt = translation_prompt_qwen(&transcript, 0).unwrap();
        assert!(prompt.contains("原文（English）：Hello world"));
        assert!(prompt.contains("中文译文："));
    }

    #[test]
    fn cleans_assistant_prefix_and_optional_whitespace() {
        assert_eq!(
            clean_translation_output("Assistant: 我是个丑陋的鸭子。"),
            "我是个丑陋的鸭子。"
        );
        assert_eq!(
            clean_translation_output("assistant\n:\n我是个丑陋的鸭子。"),
            "我是个丑陋的鸭子。"
        );
    }

    #[test]
    fn cleans_nested_chat_template_markers_and_is_idempotent() {
        let raw = "<|im_start|>assistant\n<|assistant|> Chinese (Simplified):\n我是个丑陋的鸭子。<|im_end|>";
        let clean = clean_translation_output(raw);
        assert_eq!(clean, "我是个丑陋的鸭子。");
        assert_eq!(clean_translation_output(&clean), clean);
        assert_eq!(
            clean_translation_output("<|start_header_id|>assistant<|end_header_id|>\n你好。<|eot_id|>"),
            "你好。"
        );
    }

    #[test]
    fn cleans_target_label_only_at_the_beginning() {
        assert_eq!(
            clean_translation_output("中文译文：她冲到外面呼救。"),
            "她冲到外面呼救。"
        );
        assert_eq!(
            clean_translation_output("译文：她冲到外面呼救。"),
            "她冲到外面呼救。"
        );
    }
}

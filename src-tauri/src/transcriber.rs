use crate::{chunk_stitcher, pause_alignment};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    cmp::Ordering as CmpOrdering,
    path::Path,
    process::Stdio,
    net::TcpListener,
    sync::atomic::{AtomicBool, Ordering},
};
use tauri::ipc::Channel;
use tempfile::TempDir;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, BufReader},
    process::Command,
    time::{sleep, Duration},
};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn format_seconds_mmss(sec: f64) -> String {
    let total_sec = sec.max(0.0).round() as u64;
    let minutes = total_sec / 60;
    let seconds = total_sec % 60;
    format!("{minutes:02}:{seconds:02}")
}

pub(crate) fn hidden_command(program: impl AsRef<Path>) -> Command {
    #[cfg(windows)]
    {
        let mut cmd = Command::new(program.as_ref());
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd
    }
    #[cfg(not(windows))]
    {
        Command::new(program.as_ref())
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AsrConfig {
    /// Backend identifier captured when the task starts.  The frontend may change
    /// its default later, but an active task remains bound to this value.
    #[serde(default = "default_backend")]
    pub backend: String,
    pub funasr_mode: String,
    pub funasr_runtime_path: String,
    pub funasr_model_path: String,
    pub funasr_encoder_model_path: String,
    pub funasr_vad_model_path: String,
    pub punctuation_runtime_path: String,
    pub punctuation_model_path: String,
    pub alignment_model_path: String,
    pub alignment_tokens_path: String,
    /// Best-effort human-readable selective-verification diagnostics. Empty disables it.
    #[serde(default)]
    pub verification_debug_log_path: String,
    pub funasr_chunk_seconds: f64,

    #[serde(default)]
    pub openasr_runtime_path: String,
    #[serde(default)]
    pub openasr_model_path: String,
    #[serde(default = "default_moss_chunk_seconds")]
    pub moss_chunk_seconds: f64,
    #[serde(default = "default_moss_overlap_seconds")]
    pub moss_overlap_seconds: f64,

    pub ffmpeg_path: String,
    pub ffprobe_path: String,
    pub threads: usize,
}

fn default_backend() -> String { "funasr-nano".to_string() }
fn default_moss_chunk_seconds() -> f64 { 30.0 }
fn default_moss_overlap_seconds() -> f64 { 1.0 }

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartTranscriptionRequest {
    pub video_path: String,
    pub media_duration: Option<f64>,
    pub config: AsrConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptSegment {
    pub id: String,
    pub start: f64,
    pub end: f64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PauseBoundaryRepair {
    pub boundary_offset: usize,
    pub remove_punctuation_offset: Option<usize>,
    /// Segment-local evidence for the Canonical Boundary Resolver. Older saved
    /// results remain compatible because these fields are optional/defaulted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub segment_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub segment_char_offset: Option<usize>,
    /// Optional segment-local location of the punctuation that should be removed after
    /// relocation. Kept separate from the acoustic pause itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remove_segment_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remove_segment_char_offset: Option<usize>,
    /// True only when the punctuation model semantically corroborates moving a strong mark
    /// to this pause. Acoustic strength alone must never authorize punctuation relocation.
    #[serde(default)]
    pub punctuation_relocation_supported: bool,
    pub time: f64,
    pub gap: f64,
    pub confidence: f64,
    pub context: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CrossBoundaryBridgeRepair {
    pub left_segment_id: String,
    pub right_segment_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub right_text: Option<String>,
    /// True only when a bridge re-transcription verified that `right_segment_id` is not an
    /// independent utterance but a word-final / boundary acoustic fragment.
    #[serde(default)]
    pub drop_right: bool,
    pub confidence: f64,
    pub context: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VadSpeechSegment {
    pub start: f64,
    pub end: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "event", content = "data")]
pub enum TranscriptionEvent {
    Started { duration: f64 },
    PhaseStarted { phase: String, message: String },
    PhaseProgress {
        phase: String,
        completed: u64,
        total: Option<u64>,
        unit: String,
        message: String,
    },
    PhaseCompleted { phase: String, message: String },
    Snapshot { segments: Vec<TranscriptSegment>, language: Option<String>, processed_until: f64 },
    PauseRepairUpdate { repairs: Vec<PauseBoundaryRepair>, processed_until: f64 },
    Log { message: String },
    Finished {
        segments: Vec<TranscriptSegment>,
        language: Option<String>,
        pause_repairs: Vec<PauseBoundaryRepair>,
        bridge_repairs: Vec<CrossBoundaryBridgeRepair>,
        verification_results: Vec<crate::transcript::verification::VerificationResult>,
    },
    Cancelled {},
}

fn send(channel: &Channel<TranscriptionEvent>, event: TranscriptionEvent) -> Result<(), String> {
    channel.send(event).map_err(|e| format!("向前端发送识别事件失败：{e}"))
}

pub async fn run(
    request: StartTranscriptionRequest,
    on_event: Channel<TranscriptionEvent>,
    cancel: &AtomicBool,
) -> Result<(), String> {
    if request.config.backend == "openasr-moss-q4" {
        run_openasr_moss(request, on_event, cancel).await
    } else {
        run_funasr_native(request, on_event, cancel).await
    }
}

fn validate_funasr_config(config: &AsrConfig) -> Result<(), String> {
    let model_label = match config.funasr_mode.as_str() {
        "nano" => "Fun-ASR-Nano Q8_0 GGUF",
        "paraformer" => "Paraformer Q8 GGUF",
        _ => "FunASR GGUF",
    };
    for (label, path) in [
        ("FunASR llama.cpp Runtime", config.funasr_runtime_path.as_str()),
        (model_label, config.funasr_model_path.as_str()),
        ("FSMN-VAD GGUF", config.funasr_vad_model_path.as_str()),
        ("FFmpeg", config.ffmpeg_path.as_str()),
        ("FFprobe", config.ffprobe_path.as_str()),
    ] {
        if path.trim().is_empty() || !Path::new(path).is_file() {
            return Err(format!("{label} 不存在：{path}。请打开设置点击‘检查 / 修复安装’。"));
        }
    }
    if config.funasr_mode == "nano"
        && (config.funasr_encoder_model_path.trim().is_empty()
            || !Path::new(&config.funasr_encoder_model_path).is_file())
    {
        return Err(format!(
            "Fun-ASR-Nano Encoder F16 不存在：{}。请打开设置点击‘检查 / 修复安装’。",
            config.funasr_encoder_model_path
        ));
    }
    if config.funasr_mode != "nano" {
        for (label, path) in [
            ("sherpa-onnx C API Runtime", config.punctuation_runtime_path.as_str()),
            ("中英标点恢复 INT8", config.punctuation_model_path.as_str()),
        ] {
            if path.trim().is_empty() || !Path::new(path).is_file() {
                return Err(format!("{label} 不存在：{path}。请打开设置点击‘检查 / 修复安装’。"));
            }
        }
    }
    Ok(())
}

fn parse_srt_clock(value: &str) -> Option<f64> {
    let normalized = value.trim().replace(',', ".");
    if let Ok(seconds) = normalized.parse::<f64>() {
        return Some(seconds);
    }
    let parts = normalized
        .split(':')
        .map(|part| part.parse::<f64>().ok())
        .collect::<Option<Vec<_>>>()?;
    match parts.as_slice() {
        [minutes, seconds] => Some(minutes * 60.0 + seconds),
        [hours, minutes, seconds] => Some(hours * 3600.0 + minutes * 60.0 + seconds),
        _ => None,
    }
}

fn strip_model_tags(text: &str) -> String {
    let mut result = text.to_string();
    loop {
        let Some(start) = result.find("<|") else { break };
        let Some(relative_end) = result[start + 2..].find("|>") else { break };
        let end = start + 2 + relative_end + 2;
        result.replace_range(start..end, "");
    }
    result.trim().to_string()
}

fn script_token_counts(text: &str) -> (usize, usize, usize, usize) {
    let mut han = 0usize;
    let mut kana = 0usize;
    let mut hangul = 0usize;
    for ch in text.chars() {
        if matches!(ch as u32, 0x4E00..=0x9FFF | 0x3400..=0x4DBF) {
            han += 1;
        } else if matches!(ch as u32, 0x3040..=0x30FF | 0x31F0..=0x31FF) {
            kana += 1;
        } else if matches!(ch as u32, 0xAC00..=0xD7AF | 0x1100..=0x11FF) {
            hangul += 1;
        }
    }
    let latin_words = text
        .split(|c: char| c.is_whitespace() || c.is_ascii_punctuation())
        .filter(|w| w.chars().any(|c| c.is_ascii_alphabetic()))
        .count();
    (latin_words, han, kana, hangul)
}

pub(crate) fn is_chinese_text(text: &str) -> bool {
    let (latin_words, han, kana, hangul) = script_token_counts(text);
    if kana > 0 || hangul > 0 {
        return false;
    }
    if han > 0 && latin_words == 0 {
        return true;
    }
    if han >= 15 || (han >= 4 && han >= latin_words) {
        return true;
    }
    if let Some(info) = whatlang::detect(text) {
        if matches!(info.lang(), whatlang::Lang::Cmn) && han >= 2 {
            return true;
        }
    }
    false
}

pub(crate) fn dominant_language(text: &str) -> &'static str {
    let (latin_words, han, kana, hangul) = script_token_counts(text);
    if kana > 0 && kana >= hangul && kana >= han {
        return "Japanese";
    }
    if hangul > 0 && hangul >= han {
        return "Korean";
    }
    if is_chinese_text(text) {
        return "Chinese";
    }
    if let Some(info) = whatlang::detect(text) {
        match info.lang() {
            whatlang::Lang::Cmn => {
                if han >= 2 {
                    return "Chinese";
                }
            }
            whatlang::Lang::Jpn => return "Japanese",
            whatlang::Lang::Kor => return "Korean",
            whatlang::Lang::Eng => {
                if han < 5 {
                    return "English";
                }
            }
            _ => {}
        }
    }
    if latin_words >= 3 && latin_words >= han {
        "English"
    } else if han > 0 {
        "Chinese"
    } else {
        "English"
    }
}

fn is_han_char(ch: char) -> bool {
    matches!(ch as u32, 0x3400..=0x4DBF | 0x4E00..=0x9FFF)
}

fn parse_srt_segments(
    raw: &str,
    audio_start: f64,
    nominal_start: f64,
    nominal_end: f64,
    is_last: bool,
    chunk_index: usize,
) -> Vec<TranscriptSegment> {
    let normalized = raw.replace("\r\n", "\n").replace('\r', "\n");
    let mut result = Vec::new();
    for (cue_index, block) in normalized.split("\n\n").enumerate() {
        let lines: Vec<&str> = block.lines().map(str::trim).filter(|v| !v.is_empty()).collect();
        if lines.len() < 2 {
            continue;
        }
        let time_index = lines.iter().position(|line| line.contains("-->"));
        let Some(time_index) = time_index else { continue };
        let mut times = lines[time_index].split("-->");
        let Some(local_start) = times.next().and_then(parse_srt_clock) else { continue };
        let Some(local_end) = times.next().and_then(parse_srt_clock) else { continue };
        let start = audio_start + local_start;
        let end = audio_start + local_end.max(local_start + 0.05);
        let midpoint = (start + end) / 2.0;
        let owned = midpoint >= nominal_start - 0.0001
            && (midpoint < nominal_end || (is_last && midpoint <= nominal_end + 0.0001));
        if !owned {
            continue;
        }
        let text = strip_model_tags(
            &lines
                .iter()
                .skip(time_index + 1)
                .copied()
                .collect::<Vec<_>>()
                .join(" "),
        );
        if text.is_empty() {
            continue;
        }
        result.push(TranscriptSegment {
            id: format!("funasr-gguf-{chunk_index}-{cue_index}-{start:.3}"),
            start: start.max(0.0),
            end: end.max(start + 0.05),
            text,
        });
    }
    result
}

async fn run_funasr_srt(
    config: &AsrConfig,
    wav_path: &Path,
    cancel: &AtomicBool,
    use_vad: bool,
) -> Result<Option<(String, String)>, String> {
    run_funasr_srt_with_context(config, wav_path, cancel, use_vad, None, &[], None).await
}

#[derive(Debug, Clone, Copy, Default)]
struct NanoRuntimeCapabilities {
    hotwords_flag: Option<&'static str>,
    vad_maxseg: bool,
    chunk: bool,
}

async fn detect_nano_runtime_capabilities(runtime_path: &str) -> NanoRuntimeCapabilities {
    if runtime_path.trim().is_empty() || !Path::new(runtime_path).is_file() {
        return NanoRuntimeCapabilities::default();
    }
    let Ok(output) = hidden_command(runtime_path)
        .arg("--help")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
    else {
        return NanoRuntimeCapabilities::default();
    };
    let mut help = String::from_utf8_lossy(&output.stdout).into_owned();
    help.push_str(&String::from_utf8_lossy(&output.stderr));
    NanoRuntimeCapabilities {
        // Only enable document hotwords when the runtime explicitly advertises the safe plural form.
        hotwords_flag: help.contains("--hotwords").then_some("--hotwords"),
        vad_maxseg: help.contains("--vad-maxseg"),
        chunk: help.contains("--chunk"),
    }
}

async fn run_funasr_srt_with_context(
    config: &AsrConfig,
    wav_path: &Path,
    cancel: &AtomicBool,
    use_vad: bool,
    hotword_flag: Option<&str>,
    hotwords: &[String],
    safe_chunk_seconds: Option<f64>,
) -> Result<Option<(String, String)>, String> {
    let mut command = hidden_command(&config.funasr_runtime_path);
    command
        .env("OMP_NUM_THREADS", config.threads.clamp(1, 16).to_string())
        .env("GGML_NTHREADS", config.threads.clamp(1, 16).to_string());
    if config.funasr_mode == "nano" {
        command
            .arg("--enc")
            .arg(&config.funasr_encoder_model_path);
    }
    command
        .arg("-m")
        .arg(&config.funasr_model_path);
    if use_vad {
        command
            .arg("--vad")
            .arg(&config.funasr_vad_model_path);
    }
    if config.funasr_mode == "nano" {
        if let Some(flag) = hotword_flag {
            let joined = hotwords.iter()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .take(12)
                .collect::<Vec<_>>()
                .join(",");
            if !joined.is_empty() {
                command.arg(flag).arg(joined);
            }
        }
        if let Some(seconds) = safe_chunk_seconds.filter(|value| value.is_finite() && *value > 0.0) {
            command.arg("--chunk").arg(format!("{:.0}", seconds.clamp(5.0, 15.0)));
        }
    }
    command
        .arg("-a")
        .arg(wav_path)
        .arg("--srt")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command
        .spawn()
        .map_err(|e| format!(
            "无法启动 FunASR llama.cpp（{}）：{e}",
            config.funasr_runtime_path
        ))?;

    let mut stdout = child.stdout.take().ok_or_else(|| "无法读取 FunASR stdout".to_string())?;
    let mut stderr = child.stderr.take().ok_or_else(|| "无法读取 FunASR stderr".to_string())?;
    let stdout_task = tokio::spawn(async move {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).await.map(|_| bytes)
    });
    let stderr_task = tokio::spawn(async move {
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).await.map(|_| bytes)
    });

    let status = loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill().await;
            let _ = child.wait().await;
            let _ = stdout_task.await;
            let _ = stderr_task.await;
            return Ok(None);
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|e| format!("检查 FunASR 进程状态失败：{e}"))?
        {
            break status;
        }
        sleep(Duration::from_millis(120)).await;
    };

    let stdout_bytes = stdout_task
        .await
        .map_err(|e| format!("等待 FunASR stdout 失败：{e}"))?
        .map_err(|e| format!("读取 FunASR stdout 失败：{e}"))?;
    let stderr_bytes = stderr_task
        .await
        .map_err(|e| format!("等待 FunASR stderr 失败：{e}"))?
        .map_err(|e| format!("读取 FunASR stderr 失败：{e}"))?;
    let stdout_text = String::from_utf8_lossy(&stdout_bytes).into_owned();
    let stderr_text = String::from_utf8_lossy(&stderr_bytes).into_owned();
    if !status.success() {
        return Err(format!(
            "FunASR llama.cpp 识别失败：{status}\n{}",
            tail_text(&stderr_text, 5000)
        ));
    }
    Ok(Some((stdout_text, stderr_text)))
}

fn tail_text(text: &str, max_chars: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max_chars {
        return text.to_string();
    }
    chars[chars.len() - max_chars..].iter().collect()
}

async fn extract_full_audio_wav(ffmpeg: &str, video: &str, output: &Path) -> Result<(), String> {
    let out = hidden_command(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i", video, "-vn", "-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le"])
        .arg(output)
        .output()
        .await
        .map_err(|e| format!("无法启动 ffmpeg（{ffmpeg}）：{e}"))?;
    if !out.status.success() {
        return Err(format!(
            "FFmpeg 导出 VAD 音频失败：{}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(())
}


fn ends_with_strong_sentence_punctuation(text: &str) -> bool {
    text.trim_end()
        .chars()
        .rev()
        .find(|c| !matches!(c, '"' | '\'' | '”' | '’' | ')' | ']' | '】' | '》'))
        .is_some_and(|c| matches!(c, '.' | '!' | '?' | '。' | '！' | '？'))
}

fn cjk_suffix_prefix_overlap(left: &str, right: &str) -> usize {
    let l = left.chars().filter(|c| is_han_char(*c)).collect::<Vec<_>>();
    let r = right.chars().filter(|c| is_han_char(*c)).collect::<Vec<_>>();
    let max = l.len().min(r.len()).min(12);
    (1..=max).rev().find(|&n| l[l.len() - n..] == r[..n]).unwrap_or(0)
}

fn english_suffix_prefix_overlap(left: &str, right: &str) -> usize {
    let words = |text: &str| {
        text.split(|c: char| !c.is_ascii_alphanumeric() && c != '\'')
            .filter(|w| !w.is_empty())
            .map(|w| w.to_ascii_lowercase())
            .collect::<Vec<_>>()
    };
    let l = words(left);
    let r = words(right);
    let max = l.len().min(r.len()).min(8);
    (1..=max).rev().find(|&n| l[l.len() - n..] == r[..n]).unwrap_or(0)
}


fn bridge_candidate_indices(segments: &[TranscriptSegment]) -> Vec<usize> {
    let mut out = Vec::new();
    for i in 0..segments.len().saturating_sub(1) {
        let left = &segments[i];
        let right = &segments[i + 1];
        let gap = right.start - left.end;
        if gap.abs() > 0.85 || ends_with_strong_sentence_punctuation(&left.text) {
            continue;
        }
        let right_chars = right.text.chars().count();
        let right_words = right.text.split_whitespace().count();
        let right_has_han = right.text.chars().any(is_han_char);
        let right_short = if right_has_han {
            right.end - right.start <= 4.5 || right_chars <= 14
        } else {
            right.end - right.start <= 4.5 || right_words <= 5
        };
        let overlap = cjk_suffix_prefix_overlap(&left.text, &right.text) >= 2
            || english_suffix_prefix_overlap(&left.text, &right.text) >= 2;
        if right_short || overlap {
            out.push(i);
        }
    }
    // Bridge inference reloads the model. Keep this path selective and deterministic.
    out.truncate(4);
    out
}

async fn build_selective_bridge_repairs(
    config: &AsrConfig,
    full_wav: &Path,
    duration: f64,
    segments: &[TranscriptSegment],
    _language: &str,
    temp_dir: &Path,
    cancel: &AtomicBool,
    on_event: &Channel<TranscriptionEvent>,
) -> Result<Vec<CrossBoundaryBridgeRepair>, String> {
    let candidates = bridge_candidate_indices(segments);
    let candidate_count = candidates.len();
    let _ = send(on_event, TranscriptionEvent::PhaseProgress {
        phase: "boundary_review".into(),
        completed: 0,
        total: Some(candidate_count as u64),
        unit: "candidates".into(),
        message: if candidate_count == 0 {
            "没有发现需要复听的跨段异常边界".into()
        } else {
            format!("待检查跨段边界 0 / {candidate_count}")
        },
    });
    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    let mut repairs = Vec::new();
    let full_wav_path = full_wav.to_string_lossy().into_owned();

    // Micro/isolated fragments are intentionally NOT repaired here anymore. They are routed
    // through the Verification subsystem, where Expanded Nano plus the v26 Safety Gate owns
    // selective correction. This bridge path is reserved for ordinary
    // cross-Raw continuation conflicts such as a sentence broken across two larger ASR cues.

    for (ordinal, index) in candidates.into_iter().enumerate() {
        let _ = send(on_event, TranscriptionEvent::PhaseProgress {
            phase: "boundary_review".into(),
            completed: ordinal as u64,
            total: Some(candidate_count as u64),
            unit: "candidates".into(),
            message: format!("正在检查跨段边界 {} / {candidate_count}", ordinal + 1),
        });
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let left = &segments[index];
        let right = &segments[index + 1];
        let boundary = ((left.end + right.start) * 0.5).clamp(0.0, duration);
        let window_start = (boundary - 5.0).max(0.0);
        let window_end = (boundary + 6.0).min(duration);
        if window_end - window_start < 3.0 {
            continue;
        }

        let wav = temp_dir.join(format!("bridge-{ordinal}.wav"));
        if extract_chunk(
            &config.ffmpeg_path,
            &full_wav_path,
            window_start,
            window_end - window_start,
            &wav,
        ).await.is_err() {
            continue;
        }
        let Some((srt, _stderr)) = run_funasr_srt(config, &wav, cancel, false).await? else {
            continue;
        };
        let bridge = parse_srt_segments(
            &srt,
            window_start,
            window_start,
            window_end,
            true,
            10_000 + ordinal,
        );
        if bridge.is_empty() {
            continue;
        }

        let mut previous = vec![left.clone()];
        let mut current = vec![right.clone()];
        let before_left = previous[0].text.clone();
        let before_right = current[0].text.clone();
        let stats = chunk_stitcher::rewrite_with_authoritative_bridge(&mut previous, &mut current, &bridge);
        if stats.method != "authoritative-bridge" {
            continue;
        }
        let after_left = previous[0].text.clone();
        let after_right = current[0].text.clone();
        if after_left == before_left && after_right == before_right {
            continue;
        }
        repairs.push(CrossBoundaryBridgeRepair {
            left_segment_id: left.id.clone(),
            right_segment_id: right.id.clone(),
            left_text: (after_left != before_left).then_some(after_left),
            right_text: (after_right != before_right).then_some(after_right),
            drop_right: false,
            confidence: (0.90 + (stats.match_tokens.min(8) as f64) * 0.01).min(0.98),
            context: format!(
                "authoritative bridge {:.2}-{:.2}s · anchors {} · {}",
                window_start, window_end, stats.match_tokens, stats.method
            ),
        });
    }
    let _ = send(on_event, TranscriptionEvent::PhaseProgress {
        phase: "boundary_review".into(),
        completed: candidate_count as u64,
        total: Some(candidate_count as u64),
        unit: "candidates".into(),
        message: format!("跨段边界检查 {candidate_count} / {candidate_count}"),
    });
    Ok(repairs)
}


fn verification_debug_sanitize(value: &str) -> String {
    value.replace('\r', "\\r").replace('\n', "\\n")
}

fn verification_debug_log(config: &AsrConfig, stage: &str, message: impl AsRef<str>) {
    let path = config.verification_debug_log_path.trim();
    if path.is_empty() { return; }
    use std::io::Write as _;
    let timestamp_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|v| v.as_millis())
        .unwrap_or(0);
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "[{timestamp_ms}] [{stage}] {}", message.as_ref());
    }
}

fn verification_surface_segments(segments: &[TranscriptSegment]) -> Vec<crate::transcript::verification::VerificationSegment> {
    segments.iter().map(|s| crate::transcript::verification::VerificationSegment {
        id: s.id.clone(),
        start_ms: (s.start.max(0.0) * 1000.0).round() as u64,
        end_ms: (s.end.max(s.start) * 1000.0).round() as u64,
        text: s.text.clone(),
    }).collect()
}

fn join_verifier_srt(raw: &str) -> String {
    parse_srt_segments(raw, 0.0, 0.0, f64::MAX / 4.0, true, 90_000)
        .into_iter()
        .map(|s| s.text.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn join_segment_text<'a>(segments: impl Iterator<Item = &'a TranscriptSegment>) -> String {
    segments
        .map(|s| s.text.trim())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Partition an Expanded Nano SRT by absolute time. The target is selected by cue midpoint rather
/// than lexical anchors, so harmless wording drift in the outer context cannot veto a correction.
/// The returned coverage measures how much of RewriteSpan is covered by any Expanded SRT cue.
fn expanded_time_partition(
    expanded: &[TranscriptSegment],
    target_start: f64,
    target_end: f64,
) -> (String, String, String, f32) {
    let before = join_segment_text(expanded.iter().filter(|s| {
        let midpoint = (s.start + s.end) * 0.5;
        midpoint < target_start
    }));
    let target_rows = expanded.iter().filter(|s| {
        let midpoint = (s.start + s.end) * 0.5;
        midpoint >= target_start && midpoint <= target_end
    }).collect::<Vec<_>>();
    let target = join_segment_text(target_rows.iter().copied());
    let after = join_segment_text(expanded.iter().filter(|s| {
        let midpoint = (s.start + s.end) * 0.5;
        midpoint > target_end
    }));

    let target_duration = (target_end - target_start).max(0.001);
    let mut intervals = expanded
        .iter()
        .filter_map(|s| {
            let start = s.start.max(target_start);
            let end = s.end.min(target_end);
            (end > start).then_some((start, end))
        })
        .collect::<Vec<_>>();
    intervals.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut covered = 0.0_f64;
    let mut current: Option<(f64, f64)> = None;
    for (start, end) in intervals {
        match current {
            Some((cs, ce)) if start <= ce => current = Some((cs, ce.max(end))),
            Some((cs, ce)) => {
                covered += ce - cs;
                current = Some((start, end));
            }
            None => current = Some((start, end)),
        }
    }
    if let Some((cs, ce)) = current { covered += ce - cs; }
    let coverage = (covered / target_duration).clamp(0.0, 1.0) as f32;
    (before, target, after, coverage)
}

/// v2.6.2 selective re-listening. Suspicion rules only choose *where to listen again*; they never
/// authorize a text rewrite. A single Expanded Nano pass receives wider acoustic/semantic context.
/// Precise SRT timing and constrained text alignment are tracked separately by the Safety Gate.
async fn build_selective_verification_results(
    config: &AsrConfig,
    full_wav: &Path,
    duration: f64,
    segments: &[TranscriptSegment],
    temp_dir: &Path,
    cancel: &AtomicBool,
    on_event: &Channel<TranscriptionEvent>,
) -> Result<Vec<crate::transcript::verification::VerificationResult>, String> {
    use crate::transcript::verification::{
        assess_expanded_candidate, build_entity_memory, canonicalize_known_entities,
        context_head, context_preservation, context_tail, detect_suspicions,
        extract_local_rewrite_by_alignment,
        target_surface, CorrectionKind,
        VerificationDecision,
    };
    use crate::transcript::surface::{analyze_decoder_surface, lexical_surface_equivalent};

    verification_debug_log(
        config,
        "VERIFY-BEGIN",
        format!(
            "v2.6.2-expanded+surface-retry mode={} duration={:.3}s rawSegments={}",
            config.funasr_mode,
            duration,
            segments.len(),
        ),
    );
    if config.funasr_mode != "nano" || segments.len() < 3 {
        verification_debug_log(
            config,
            "VERIFY-SKIP",
            "v2.6.2 verification requires Nano mode and at least three Raw segments",
        );
        return Ok(Vec::new());
    }

    let surface = verification_surface_segments(segments);
    for (i, segment) in surface.iter().enumerate() {
        let left_gap = i
            .checked_sub(1)
            .map(|p| segment.start_ms.saturating_sub(surface[p].end_ms));
        let right_gap = (i + 1 < surface.len())
            .then(|| surface[i + 1].start_ms.saturating_sub(segment.end_ms));
        let duration_ms = segment.end_ms.saturating_sub(segment.start_ms);
        if duration_ms <= 2_500 || segment.text.chars().count() <= 18 {
            verification_debug_log(
                config,
                "SEGMENT-SCAN",
                format!(
                    "idx={} id={} startMs={} endMs={} durationMs={} leftGapMs={:?} rightGapMs={:?} text=\"{}\"",
                    i,
                    segment.id,
                    segment.start_ms,
                    segment.end_ms,
                    duration_ms,
                    left_gap,
                    right_gap,
                    verification_debug_sanitize(&segment.text),
                ),
            );
        }
    }

    // Phase 1: bootstrap memory is allowed to *find* possible entity variants, but it is not a
    // committed current-document memory. build_entity_memory() deliberately rejects plain
    // ALL-CAPS tokens from decoder-wide uppercase prose so surface degeneration cannot seed it.
    let memory = build_entity_memory(&surface);
    verification_debug_log(
        config,
        "ENTITY-MEMORY",
        format!(
            "phase=bootstrap stableCount={} stable={}",
            memory.stable.len(),
            memory
                .stable
                .iter()
                .map(|e| format!("{}x{} aliases={:?}", e.canonical, e.occurrences, e.aliases))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    );

    let candidates = detect_suspicions(&surface, &memory, 16);
    let candidate_count = candidates.len();
    verification_debug_log(
        config,
        "CANDIDATE-SCAN",
        format!("selectedCandidates={} maxCandidates=16", candidate_count),
    );
    let _ = send(on_event, TranscriptionEvent::PhaseProgress {
        phase: "verification".into(),
        completed: 0,
        total: Some(candidate_count as u64),
        unit: "candidates".into(),
        message: if candidate_count == 0 {
            "没有发现需要复听的可疑片段".into()
        } else {
            format!("待复听可疑片段 0 / {candidate_count}")
        },
    });
    if candidates.is_empty() {
        verification_debug_log(config, "VERIFY-END", "no suspicion candidates selected");
        return Ok(Vec::new());
    }

    let runtime_caps = detect_nano_runtime_capabilities(&config.funasr_runtime_path).await;
    let hotword_flag = runtime_caps.hotwords_flag;
    verification_debug_log(
        config,
        "CAPABILITIES",
        format!("nanoHotwordsFlag={:?} chunk={} vadMaxSeg={} verifier=expanded-nano+surface-retry", hotword_flag, runtime_caps.chunk, runtime_caps.vad_maxseg),
    );

    let full_wav_path = full_wav.to_string_lossy().into_owned();
    let mut results = Vec::new();

    for (ordinal, candidate) in candidates.into_iter().enumerate() {
        let _ = send(on_event, TranscriptionEvent::PhaseProgress {
            phase: "verification".into(),
            completed: ordinal as u64,
            total: Some(candidate_count as u64),
            unit: "candidates".into(),
            message: format!("正在复听可疑片段 {} / {candidate_count}", ordinal + 1),
        });
        if cancel.load(Ordering::Relaxed) { break; }
        let start_index = *candidate.target_indices.first().unwrap_or(&0);
        let end_index = *candidate.target_indices.last().unwrap_or(&start_index);

        if candidate.reasons.iter().any(|reason| matches!(reason, crate::transcript::verification::SuspicionReason::DecoderSurfaceDegeneration)) {
            let first_pass = target_surface(&surface, &candidate);
            let first_health = analyze_decoder_surface(&first_pass, candidate.end_ms.saturating_sub(candidate.start_ms));
            verification_debug_log(
                config,
                "SURFACE-RETRY",
                format!(
                    "ordinal={} targetIds={:?} durationMs={} words={} upperRatio={:.3} punctuation={} repetition={:.3} severe={} chunkCap={}",
                    ordinal,
                    candidate.target_segment_ids,
                    candidate.end_ms.saturating_sub(candidate.start_ms),
                    first_health.word_count,
                    first_health.uppercase_ratio,
                    first_health.strong_punctuation_count,
                    first_health.repetition_ratio,
                    first_health.severe,
                    runtime_caps.chunk,
                ),
            );

            if candidate.target_segment_ids.len() != 1 || !runtime_caps.chunk {
                results.push(crate::transcript::verification::VerificationResult {
                    target_segment_ids: candidate.target_segment_ids.clone(),
                    suspicious_segment_ids: candidate.suspicious_segment_ids.clone(),
                    suspicious_start_ms: candidate.suspicious_start_ms,
                    suspicious_end_ms: candidate.suspicious_end_ms,
                    start_ms: candidate.start_ms,
                    end_ms: candidate.end_ms,
                    context_start_ms: candidate.start_ms,
                    context_end_ms: candidate.end_ms,
                    reasons: candidate.reasons.clone(),
                    suspicion_score: candidate.score,
                    first_pass_text: first_pass,
                    expanded_nano_text: None,
                    expanded_target_text: None,
                    correction_kind: None,
                    left_context_similarity: 0.0,
                    right_context_similarity: 0.0,
                    target_time_coverage: 0.0,
                    time_grounded: false,
                    text_aligned: false,
                    edit_ratio: 0.0,
                    replacement_ratio: 1.0,
                    decision: VerificationDecision::Uncertain,
                    replacement_text: None,
                    confidence: 0.35,
                    safety_reasons: vec!["SAFE_CHUNK_RETRY_UNAVAILABLE".into()],
                    hotwords: Vec::new(),
                    context: "Decoder surface degeneration detected; no safe runtime chunk retry capability was available".into(),
                });
                continue;
            }

            let retry_wav = temp_dir.join(format!("surface-retry-{ordinal}.wav"));
            let retry_start = candidate.start_ms as f64 / 1000.0;
            let retry_duration = candidate.end_ms.saturating_sub(candidate.start_ms) as f64 / 1000.0;
            if let Err(error) = extract_chunk(
                &config.ffmpeg_path,
                &full_wav_path,
                retry_start,
                retry_duration.max(0.05),
                &retry_wav,
            ).await {
                verification_debug_log(config, "SURFACE-RETRY-ERROR", format!("ordinal={} extract={}", ordinal, error));
                continue;
            }

            let Some((retry_raw, retry_stderr)) = run_funasr_srt_with_context(
                config,
                &retry_wav,
                cancel,
                false,
                None,
                &[],
                Some(config.funasr_chunk_seconds.min(15.0)),
            ).await? else { continue; };
            let retry_text = join_verifier_srt(&retry_raw);
            let retry_health = analyze_decoder_surface(&retry_text, candidate.end_ms.saturating_sub(candidate.start_ms));
            let lexical_equivalent = lexical_surface_equivalent(&first_pass, &retry_text);
            let healthier_surface = retry_health.strong_punctuation_count > first_health.strong_punctuation_count
                || (!retry_health.case_degenerated && first_health.case_degenerated)
                || (!retry_health.punctuation_degenerated && first_health.punctuation_degenerated)
                || retry_health.repetition_ratio + 0.08 < first_health.repetition_ratio;
            let decision = if lexical_equivalent && healthier_surface {
                VerificationDecision::Verified
            } else {
                VerificationDecision::Uncertain
            };
            let confidence = if matches!(decision, VerificationDecision::Verified) { 0.92 } else { 0.42 };
            let safety_reasons = if matches!(decision, VerificationDecision::Verified) {
                vec![
                    "RESET_SAFE_WINDOW_RETRY".into(),
                    "LEXICAL_UNITS_EQUIVALENT".into(),
                    "SURFACE_REPAIR_ONLY".into(),
                ]
            } else {
                vec![
                    "RESET_SAFE_WINDOW_RETRY".into(),
                    if lexical_equivalent { "SURFACE_NOT_IMPROVED".into() } else { "LEXICAL_RETRY_DISAGREES".into() },
                ]
            };
            verification_debug_log(
                config,
                "SURFACE-RETRY-RESULT",
                format!(
                    "ordinal={} lexicalEquivalent={} healthier={} decision={:?} firstHealth=({:.3},{},{:.3}) retryHealth=({:.3},{},{:.3}) retry=\"{}\" stderrTail=\"{}\"",
                    ordinal,
                    lexical_equivalent,
                    healthier_surface,
                    decision,
                    first_health.uppercase_ratio,
                    first_health.strong_punctuation_count,
                    first_health.repetition_ratio,
                    retry_health.uppercase_ratio,
                    retry_health.strong_punctuation_count,
                    retry_health.repetition_ratio,
                    verification_debug_sanitize(&retry_text),
                    verification_debug_sanitize(&tail_text(&retry_stderr, 800)),
                ),
            );
            let retry_edit_ratio = if lexical_equivalent {
                0.0
            } else {
                crate::transcript::verification::token_edit_ratio(&first_pass, &retry_text)
            };
            results.push(crate::transcript::verification::VerificationResult {
                target_segment_ids: candidate.target_segment_ids.clone(),
                suspicious_segment_ids: candidate.suspicious_segment_ids.clone(),
                suspicious_start_ms: candidate.suspicious_start_ms,
                suspicious_end_ms: candidate.suspicious_end_ms,
                start_ms: candidate.start_ms,
                end_ms: candidate.end_ms,
                context_start_ms: candidate.start_ms,
                context_end_ms: candidate.end_ms,
                reasons: candidate.reasons.clone(),
                suspicion_score: candidate.score,
                first_pass_text: first_pass,
                expanded_nano_text: Some(retry_text.clone()),
                expanded_target_text: Some(retry_text),
                correction_kind: None,
                left_context_similarity: 1.0,
                right_context_similarity: 1.0,
                target_time_coverage: 1.0,
                time_grounded: false,
                text_aligned: lexical_equivalent,
                edit_ratio: retry_edit_ratio,
                replacement_ratio: 1.0,
                decision,
                replacement_text: None,
                confidence,
                safety_reasons,
                hotwords: Vec::new(),
                context: "Reset Nano retry used a fresh decoder process with <=15s internal chunking; only punctuation/casing may be reused when lexical units are identical".into(),
            });
            continue;
        }

        if start_index == 0 || end_index + 1 >= segments.len() {
            verification_debug_log(
                config,
                "CANDIDATE-SKIP",
                format!(
                    "ordinal={} ids={:?} indices={:?} reason=missing_outer_neighbor segmentCount={}",
                    ordinal, candidate.target_segment_ids, candidate.target_indices, segments.len()
                ),
            );
            continue;
        }

        let left = &segments[start_index - 1];
        let right = &segments[end_index + 1];
        let first_pass = target_surface(&surface, &candidate);
        // Context references are bounded because ContextSpan may only contain the tail/head of a
        // long neighboring Raw cue. Comparing against an entire 12s cue would create false low scores.
        let left_reference = context_tail(&left.text, 12);
        let right_reference = context_head(&right.text, 12);
        verification_debug_log(
            config,
            "CANDIDATE",
            format!(
                "ordinal={} suspiciousIds={:?} targetIds={:?} suspiciousSpan={}..{}ms rewriteSpan={}..{}ms score={:.3} reasons={:?} hotwords={:?} firstPass=\"{}\" left=\"{}\" right=\"{}\"",
                ordinal,
                candidate.suspicious_segment_ids,
                candidate.target_segment_ids,
                candidate.suspicious_start_ms,
                candidate.suspicious_end_ms,
                candidate.start_ms,
                candidate.end_ms,
                candidate.score,
                candidate.reasons,
                candidate.hotwords,
                verification_debug_sanitize(&first_pass),
                verification_debug_sanitize(&left_reference),
                verification_debug_sanitize(&right_reference),
            ),
        );

        // Three spans are deliberately separate:
        // SuspiciousSpan = what triggered review;
        // RewriteSpan = minimal Raw cues we may modify;
        // ContextSpan = audio the Expanded Nano is allowed to hear.
        let rewrite_start = candidate.start_ms as f64 / 1000.0;
        let rewrite_end = candidate.end_ms as f64 / 1000.0;
        let core_start = rewrite_start.min(candidate.suspicious_start_ms as f64 / 1000.0);
        let core_end = rewrite_end.max(candidate.suspicious_end_ms as f64 / 1000.0);
        let core_duration = (core_end - core_start).max(0.05);
        let margin = ((18.0 - core_duration).max(0.0) * 0.5).min(5.5).max(2.0);
        let window_start = (core_start - margin).max(0.0);
        let window_end = (core_end + margin).min(duration);
        let window_duration = window_end - window_start;

        verification_debug_log(
            config,
            "EXPANDED-WINDOW",
            format!(
                "ordinal={} suspicious={:.3}..{:.3}s rewrite={:.3}..{:.3}s context={:.3}..{:.3}s duration={:.3}s",
                ordinal,
                candidate.suspicious_start_ms as f64 / 1000.0,
                candidate.suspicious_end_ms as f64 / 1000.0,
                rewrite_start,
                rewrite_end,
                window_start,
                window_end,
                window_duration,
            ),
        );

        if !(2.0..=18.05).contains(&window_duration) {
            let reason = "CONTEXT_WINDOW_OUTSIDE_SAFETY_BOUNDS".to_string();
            verification_debug_log(config, "SAFETY-GATE", format!("ordinal={} decision=UNCERTAIN reasons={:?}", ordinal, [reason.clone()]));
            results.push(crate::transcript::verification::VerificationResult {
                target_segment_ids: candidate.target_segment_ids.clone(),
                suspicious_segment_ids: candidate.suspicious_segment_ids.clone(),
                suspicious_start_ms: candidate.suspicious_start_ms,
                suspicious_end_ms: candidate.suspicious_end_ms,
                start_ms: candidate.start_ms,
                end_ms: candidate.end_ms,
                context_start_ms: (window_start * 1000.0).round() as u64,
                context_end_ms: (window_end * 1000.0).round() as u64,
                reasons: candidate.reasons.clone(),
                suspicion_score: candidate.score,
                first_pass_text: first_pass,
                expanded_nano_text: None,
                expanded_target_text: None,
                correction_kind: None,
                left_context_similarity: 0.0,
                right_context_similarity: 0.0,
                target_time_coverage: 0.0,
                time_grounded: false,
                text_aligned: false,
                edit_ratio: 1.0,
                replacement_ratio: 0.0,
                decision: VerificationDecision::Uncertain,
                replacement_text: None,
                confidence: 0.20,
                safety_reasons: vec![reason],
                hotwords: candidate.hotwords.clone(),
                context: "Expanded context window is outside v2.6.2 safety bounds".into(),
            });
            continue;
        }

        let expanded_wav = temp_dir.join(format!("verify-expanded-{ordinal}.wav"));
        if let Err(error) = extract_chunk(
            &config.ffmpeg_path,
            &full_wav_path,
            window_start,
            window_duration,
            &expanded_wav,
        ).await {
            let reason = "EXPANDED_AUDIO_EXTRACT_FAILED".to_string();
            verification_debug_log(
                config,
                "EXPANDED-ERROR",
                format!("ordinal={} reason={} error={}", ordinal, reason, verification_debug_sanitize(&error)),
            );
            results.push(crate::transcript::verification::VerificationResult {
                target_segment_ids: candidate.target_segment_ids.clone(),
                suspicious_segment_ids: candidate.suspicious_segment_ids.clone(),
                suspicious_start_ms: candidate.suspicious_start_ms,
                suspicious_end_ms: candidate.suspicious_end_ms,
                start_ms: candidate.start_ms,
                end_ms: candidate.end_ms,
                context_start_ms: (window_start * 1000.0).round() as u64,
                context_end_ms: (window_end * 1000.0).round() as u64,
                reasons: candidate.reasons.clone(),
                suspicion_score: candidate.score,
                first_pass_text: first_pass,
                expanded_nano_text: None,
                expanded_target_text: None,
                correction_kind: None,
                left_context_similarity: 0.0,
                right_context_similarity: 0.0,
                target_time_coverage: 0.0,
                time_grounded: false,
                text_aligned: false,
                edit_ratio: 1.0,
                replacement_ratio: 0.0,
                decision: VerificationDecision::Uncertain,
                replacement_text: None,
                confidence: 0.20,
                safety_reasons: vec![reason],
                hotwords: candidate.hotwords.clone(),
                context: "Expanded audio extraction failed; Raw remains authoritative".into(),
            });
            continue;
        }

        let Some((nano_raw, nano_stderr)) = run_funasr_srt_with_context(
            config,
            &expanded_wav,
            cancel,
            false,
            hotword_flag,
            &candidate.hotwords,
            runtime_caps.chunk.then_some(config.funasr_chunk_seconds.min(15.0)),
        ).await? else { break; };

        let expanded_nano_text = join_verifier_srt(&nano_raw);
        let expanded_segments = parse_srt_segments(
            &nano_raw,
            window_start,
            window_start,
            window_end,
            true,
            90_000 + ordinal,
        );
        verification_debug_log(
            config,
            "EXPANDED-NANO",
            format!(
                "ordinal={} cues={} text=\"{}\" stderrTail=\"{}\"",
                ordinal,
                expanded_segments.len(),
                verification_debug_sanitize(&expanded_nano_text),
                verification_debug_sanitize(&tail_text(&nano_stderr, 1200)),
            ),
        );

        let (expanded_before, time_target_raw, expanded_after, target_time_coverage) =
            expanded_time_partition(&expanded_segments, rewrite_start, rewrite_end);
        let time_left_similarity = context_preservation(&left_reference, &expanded_before);
        let time_right_similarity = context_preservation(&right_reference, &expanded_after);
        let aligned = extract_local_rewrite_by_alignment(
            &left_reference,
            &first_pass,
            &right_reference,
            &expanded_nano_text,
        );
        let text_aligned = aligned.is_some();
        // A single cue covering the whole Expanded window trivially reports coverage=1.0 but does
        // not localize RewriteSpan. Require actual cue material on both sides before treating SRT
        // time as precise grounding. Text alignment remains a separate, weaker evidence path.
        let time_grounded = expanded_segments.len() >= 3
            && !expanded_before.trim().is_empty()
            && !expanded_after.trim().is_empty()
            && target_time_coverage > 0.0;
        let (expanded_target_raw, left_similarity, right_similarity, extraction_source) =
            if let Some(local) = aligned {
                (
                    local.replacement_text,
                    local.left_context_similarity,
                    local.right_context_similarity,
                    "ALIGNED_MINIMAL_DIFF",
                )
            } else {
                (
                    time_target_raw.clone(),
                    time_left_similarity,
                    time_right_similarity,
                    "TIME_MIDPOINT_FALLBACK",
                )
            };
        let expanded_target = canonicalize_known_entities(&expanded_target_raw, &memory);

        verification_debug_log(
            config,
            "CORRECTION-CANDIDATE",
            format!(
                "ordinal={} source={} timeGrounded={} textAligned={} first=\"{}\" timeTarget=\"{}\" expandedTargetRaw=\"{}\" expandedTarget=\"{}\" coverage={:.3} leftSimilarity={:.3} rightSimilarity={:.3} timeLeftSimilarity={:.3} timeRightSimilarity={:.3} before=\"{}\" after=\"{}\"",
                ordinal,
                extraction_source,
                time_grounded,
                text_aligned,
                verification_debug_sanitize(&first_pass),
                verification_debug_sanitize(&time_target_raw),
                verification_debug_sanitize(&expanded_target_raw),
                verification_debug_sanitize(&expanded_target),
                target_time_coverage,
                left_similarity,
                right_similarity,
                time_left_similarity,
                time_right_similarity,
                verification_debug_sanitize(&expanded_before),
                verification_debug_sanitize(&expanded_after),
            ),
        );

        let assessment = assess_expanded_candidate(
            &first_pass,
            &expanded_target,
            &candidate.reasons,
            candidate.score,
            left_similarity,
            right_similarity,
            target_time_coverage,
            time_grounded,
            text_aligned,
        );
        verification_debug_log(
            config,
            "SAFETY-GATE",
            format!(
                "ordinal={} kind={:?} decision={:?} confidence={:.3} timeGrounded={} textAligned={} editRatio={:.3} replacementRatio={:.3} coverage={:.3} leftSimilarity={:.3} rightSimilarity={:.3} reasons={:?} replacement={:?}",
                ordinal,
                assessment.correction_kind,
                assessment.decision,
                assessment.confidence,
                time_grounded,
                text_aligned,
                assessment.edit_ratio,
                assessment.replacement_ratio,
                target_time_coverage,
                left_similarity,
                right_similarity,
                assessment.reasons,
                assessment.replacement_text,
            ),
        );

        let context = match assessment.correction_kind {
            CorrectionKind::BoundaryReconstruction => "Expanded Nano produced a local boundary reconstruction with preserved outer context",
            CorrectionKind::FragmentRemoval => "Expanded Nano omitted a short suspicious fragment while both outer contexts remained stable",
            CorrectionKind::LexicalReplacement => "Expanded Nano produced a localized lexical replacement accepted by the Safety Gate",
            CorrectionKind::EntityReplacement => "Expanded Nano produced a local entity replacement; stable entity memory is formatting/bias evidence only",
            CorrectionKind::LargeRewrite => "Expanded Nano changed too much of the local surface; Safety Gate rejected automatic mutation",
        }.to_string();

        results.push(crate::transcript::verification::VerificationResult {
            target_segment_ids: candidate.target_segment_ids.clone(),
            suspicious_segment_ids: candidate.suspicious_segment_ids.clone(),
            suspicious_start_ms: candidate.suspicious_start_ms,
            suspicious_end_ms: candidate.suspicious_end_ms,
            start_ms: candidate.start_ms,
            end_ms: candidate.end_ms,
            context_start_ms: (window_start * 1000.0).round() as u64,
            context_end_ms: (window_end * 1000.0).round() as u64,
            reasons: candidate.reasons.clone(),
            suspicion_score: candidate.score,
            first_pass_text: first_pass,
            expanded_nano_text: Some(expanded_nano_text),
            expanded_target_text: Some(expanded_target),
            correction_kind: Some(assessment.correction_kind),
            left_context_similarity: left_similarity,
            right_context_similarity: right_similarity,
            target_time_coverage,
            time_grounded,
            text_aligned,
            edit_ratio: assessment.edit_ratio,
            replacement_ratio: assessment.replacement_ratio,
            decision: assessment.decision,
            replacement_text: assessment.replacement_text,
            confidence: assessment.confidence,
            safety_reasons: assessment.reasons,
            hotwords: candidate.hotwords.clone(),
            context,
        });
    }

    // Phase 2: rebuild current-document memory only after Candidate + Safety Gate decisions.
    // CORRECTED spans contribute their accepted replacement; UNCERTAIN suspicious spans are
    // excluded so a conservative Canonical fallback cannot teach the memory an unresolved typo.
    // This committed snapshot is intentionally not fed back into the same pass, preventing
    // "observe -> remember -> use memory to prove the same observation" self-reinforcement.
    let mut committed_surface = surface.clone();
    let uncertain_ids = results
        .iter()
        .filter(|r| matches!(r.decision, VerificationDecision::Uncertain))
        .flat_map(|r| r.suspicious_segment_ids.iter().cloned())
        .collect::<std::collections::HashSet<_>>();

    for result in results.iter().filter(|r| matches!(r.decision, VerificationDecision::Corrected)) {
        let Some(replacement) = result.replacement_text.as_deref() else { continue };
        let mut indices = result.target_segment_ids.iter()
            .filter_map(|id| committed_surface.iter().position(|segment| &segment.id == id))
            .collect::<Vec<_>>();
        if indices.len() != result.target_segment_ids.len() { continue; }
        indices.sort_unstable();
        if indices.windows(2).any(|pair| pair[1] != pair[0] + 1) { continue; }
        let first = indices[0];
        let last = *indices.last().unwrap_or(&first);
        if replacement.trim().is_empty() {
            for index in indices.into_iter().rev() { committed_surface.remove(index); }
        } else {
            committed_surface[first].text = replacement.trim().to_string();
            committed_surface[first].start_ms = committed_surface[first].start_ms.min(result.start_ms);
            committed_surface[first].end_ms = committed_surface[last].end_ms.max(result.end_ms);
            for index in (first + 1..=last).rev() { committed_surface.remove(index); }
        }
    }
    committed_surface.retain(|segment| {
        !segment.text.trim().is_empty() && !uncertain_ids.contains(&segment.id)
    });
    let committed_memory = build_entity_memory(&committed_surface);
    verification_debug_log(
        config,
        "ENTITY-MEMORY-COMMIT",
        format!(
            "source=post_candidate trustedSegments={} stableCount={} stable={}",
            committed_surface.len(),
            committed_memory.stable.len(),
            committed_memory.stable.iter()
                .map(|e| format!("{}x{} aliases={:?}", e.canonical, e.occurrences, e.aliases))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    );

    let _ = send(on_event, TranscriptionEvent::PhaseProgress {
        phase: "verification".into(),
        completed: candidate_count as u64,
        total: Some(candidate_count as u64),
        unit: "candidates".into(),
        message: format!("可疑片段复听 {candidate_count} / {candidate_count}"),
    });

    let verified_count = results.iter()
        .filter(|r| matches!(r.decision, VerificationDecision::Verified)).count();
    let corrected_count = results.iter()
        .filter(|r| matches!(r.decision, VerificationDecision::Corrected)).count();
    let uncertain_count = results.len().saturating_sub(verified_count + corrected_count);
    verification_debug_log(
        config,
        "VERIFY-END",
        format!(
            "results={} verified={} corrected={} uncertain={}",
            results.len(), verified_count, corrected_count, uncertain_count
        ),
    );
    Ok(results)
}

struct SrtStreamParser {
    cue_index: usize,
    buffer: Vec<String>,
}

impl SrtStreamParser {
    fn new() -> Self {
        Self {
            cue_index: 0,
            buffer: Vec::new(),
        }
    }

    fn push_line(&mut self, line: String) -> Option<TranscriptSegment> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return self.flush();
        }
        if !self.buffer.is_empty()
            && self.buffer.iter().any(|l| l.contains("-->"))
            && trimmed.parse::<usize>().is_ok()
        {
            let seg = self.flush();
            self.buffer.push(line);
            return seg;
        }
        self.buffer.push(line);
        None
    }

    fn flush(&mut self) -> Option<TranscriptSegment> {
        if self.buffer.is_empty() {
            return None;
        }
        let raw_buffer = std::mem::take(&mut self.buffer);
        let lines: Vec<&str> = raw_buffer
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        if lines.len() < 2 {
            return None;
        }
        let time_index = lines.iter().position(|line| line.contains("-->"))?;
        let mut times = lines[time_index].split("-->");
        let local_start = times.next().and_then(parse_srt_clock)?;
        let local_end = times.next().and_then(parse_srt_clock)?;
        let text = strip_model_tags(
            &lines
                .iter()
                .skip(time_index + 1)
                .copied()
                .collect::<Vec<_>>()
                .join(" "),
        );
        // Raw ASR must preserve the model surface. Only normalize whitespace here;
        // punctuation/casing/filler decisions belong to the Canonical pipeline.
        let cleaned = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if cleaned.is_empty() {
            return None;
        }
        self.cue_index += 1;
        Some(TranscriptSegment {
            id: format!("funasr-gguf-0-{}-{local_start:.3}", self.cue_index),
            start: local_start.max(0.0),
            end: local_end.max(local_start + 0.05),
            text: cleaned,
        })
    }
}

async fn run_funasr_native(
    request: StartTranscriptionRequest,
    on_event: Channel<TranscriptionEvent>,
    cancel: &AtomicBool,
) -> Result<(), String> {
    validate_funasr_config(&request.config)?;
    let duration = match request.media_duration {
        Some(v) if v.is_finite() && v > 0.0 => v,
        _ => probe_duration(&request.config.ffprobe_path, &request.video_path).await?,
    };

    let threads = request.config.threads.clamp(1, 16);
    let temp = TempDir::new().map_err(|e| format!("创建临时目录失败：{e}"))?;
    let full_wav = temp.path().join("full-audio.wav");

    send(&on_event, TranscriptionEvent::Started { duration })?;
    send(&on_event, TranscriptionEvent::PhaseStarted {
        phase: "recognition".into(),
        message: "正在进行原始语音识别".into(),
    })?;
    send(&on_event, TranscriptionEvent::PhaseProgress {
        phase: "recognition".into(),
        completed: 0,
        total: Some((duration.max(0.0) * 1000.0).round() as u64),
        unit: "milliseconds".into(),
        message: "正在提取音频……".into(),
    })?;

    extract_full_audio_wav(&request.config.ffmpeg_path, &request.video_path, &full_wav).await?;

    let model_label = match request.config.funasr_mode.as_str() {
        "nano" => "Fun-ASR-Nano Q8_0 · 高质量",
        "paraformer" => "Paraformer Q8 · 中文极速",
        _ => "FunASR GGUF",
    };

    let runtime_caps = if request.config.funasr_mode == "nano" {
        detect_nano_runtime_capabilities(&request.config.funasr_runtime_path).await
    } else {
        NanoRuntimeCapabilities::default()
    };
    let max_segment_seconds = request.config.funasr_chunk_seconds.clamp(5.0, 15.0);
    let segment_guard_label = if runtime_caps.vad_maxseg {
        format!("VAD max {:.0}s", max_segment_seconds)
    } else {
        "runtime无--vad-maxseg，启用后置退化重试".to_string()
    };

    send(&on_event, TranscriptionEvent::Log {
        message: format!(
            "FunASR 极速流式引擎 · {model_label} · 单次模型加载 · {threads}线程并行 · FSMN-VAD 全局流式转录 · {segment_guard_label}"
        ),
    })?;

    let mut command = hidden_command(&request.config.funasr_runtime_path);
    command
        .env("OMP_NUM_THREADS", threads.to_string())
        .env("GGML_NTHREADS", threads.to_string());
    if request.config.funasr_mode == "nano" {
        command.arg("--enc").arg(&request.config.funasr_encoder_model_path);
    }
    command.arg("-m").arg(&request.config.funasr_model_path);
    if Path::new(&request.config.funasr_vad_model_path).is_file() {
        command.arg("--vad").arg(&request.config.funasr_vad_model_path);
        if request.config.funasr_mode == "nano" && runtime_caps.vad_maxseg {
            command
                .arg("--vad-maxseg")
                .arg(format!("{:.0}", max_segment_seconds * 1000.0));
        }
    }
    command.arg("-a").arg(&full_wav).arg("--srt");
    command.stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);

    let mut child = command.spawn().map_err(|e| {
        format!("无法启动 FunASR llama.cpp（{}）：{e}", request.config.funasr_runtime_path)
    })?;

    let stdout = child.stdout.take().ok_or_else(|| "无法读取 FunASR stdout".to_string())?;
    let stderr = child.stderr.take().ok_or_else(|| "无法读取 FunASR stderr".to_string())?;

    let stderr_task = tokio::spawn(async move {
        let mut bytes = Vec::new();
        let mut reader = stderr;
        let _ = reader.read_to_end(&mut bytes).await;
        bytes
    });

    let mut segments: Vec<TranscriptSegment> = Vec::new();
    let mut parser = SrtStreamParser::new();
    let mut reader = BufReader::new(stdout).lines();

    send(&on_event, TranscriptionEvent::PhaseProgress {
        phase: "recognition".into(),
        completed: 0,
        total: Some((duration.max(0.0) * 1000.0).round() as u64),
        unit: "milliseconds".into(),
        message: "语音模型已就绪，正在实时转写……".into(),
    })?;

    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill().await;
            let _ = child.wait().await;
            let _ = stderr_task.await;
            send(&on_event, TranscriptionEvent::Cancelled {})?;
            return Ok(());
        }

        tokio::select! {
            line_res = reader.next_line() => {
                match line_res {
                    Ok(Some(line)) => {
                        if let Some(segment) = parser.push_line(line) {
                            let end = segment.end;
                            segments.push(segment);
                            let all_text = segments.iter().map(|s| s.text.as_str()).collect::<Vec<_>>().join(" ");
                            let lang = dominant_language(&all_text).to_string();
                            let _ = send(&on_event, TranscriptionEvent::PhaseProgress {
                                phase: "recognition".into(),
                                completed: (end.max(0.0) * 1000.0).round() as u64,
                                total: Some((duration.max(0.0) * 1000.0).round() as u64),
                                unit: "milliseconds".into(),
                                message: format!("正在转写 {} / {}", format_seconds_mmss(end), format_seconds_mmss(duration)),
                            });
                            let _ = send(&on_event, TranscriptionEvent::Snapshot {
                                segments: segments.clone(),
                                language: Some(lang),
                                processed_until: end,
                            });
                        }
                    }
                    Ok(None) => {
                        break;
                    }
                    Err(e) => {
                        return Err(format!("读取 FunASR 输出流失败：{e}"));
                    }
                }
            }
            status_res = child.wait() => {
                let _ = status_res;
                break;
            }
        }
    }

    if let Some(segment) = parser.flush() {
        segments.push(segment);
    }

    let status = child.wait().await.map_err(|e| format!("等待 FunASR 退出失败：{e}"))?;
    let stderr_bytes = stderr_task.await.unwrap_or_default();
    let stderr_text = String::from_utf8_lossy(&stderr_bytes).into_owned();

    if !status.success() && segments.is_empty() {
        return Err(format!(
            "FunASR llama.cpp 识别失败：{status}\n{}",
            tail_text(&stderr_text, 5000)
        ));
    }

    // IMPORTANT: From this point on `segments` remain the immutable FunASR Raw output.
    // Legacy cleanup/merge helpers are intentionally NOT applied before Finished;
    // all readability transforms belong to the Canonical pipeline in asr.rs.
    let all_text = segments.iter().map(|s| s.text.as_str()).collect::<Vec<_>>().join(" ");
    let detected_language = dominant_language(&all_text).to_string();
    send(&on_event, TranscriptionEvent::PhaseCompleted {
        phase: "recognition".into(),
        message: format!("原始语音识别完成 · {} 段", segments.len()),
    })?;

    // CTC may use a derived, de-duplicated ordering for alignment quality, but this
    // copy is never persisted as Raw and never replaces the FunASR output above.
    let mut alignment_segments = segments.clone();
    alignment_segments.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap_or(CmpOrdering::Equal));
    alignment_segments.dedup_by(|a, b| {
        (a.start - b.start).abs() < 0.12 && a.text.trim() == b.text.trim()
    });

    let ctc_runtime_ready = Path::new(&request.config.punctuation_runtime_path).is_file();
    let ctc_model_ready = Path::new(&request.config.alignment_model_path).is_file();
    let ctc_tokens_ready = Path::new(&request.config.alignment_tokens_path).is_file();
    let ctc_ready = ctc_runtime_ready && ctc_model_ready && ctc_tokens_ready;

    let mut pause_repairs = None;
    if request.config.funasr_mode == "nano" && ctc_ready && detected_language == "English" && !alignment_segments.is_empty() {
        let _ = send(&on_event, TranscriptionEvent::PhaseStarted {
            phase: "pause_alignment".into(),
            message: "正在分析 English CTC 停顿边界".into(),
        });
        let final_end = alignment_segments.iter().map(|s| s.end).fold(0.0_f64, f64::max).min(duration);
        match pause_alignment::build_selective_pause_repairs_in_range(
            &request.config.ffmpeg_path,
            &request.video_path,
            duration,
            &alignment_segments,
            &[],
            Path::new(&request.config.punctuation_runtime_path),
            Path::new(&request.config.alignment_model_path),
            Path::new(&request.config.alignment_tokens_path),
            Path::new(&request.config.punctuation_model_path),
            threads,
            0.0,
            final_end,
        ).await {
            Ok(repairs) => {
                if !repairs.is_empty() {
                    let _ = send(&on_event, TranscriptionEvent::Log {
                        message: format!("English CTC 对齐校准完成 · 修复 {} 处停顿", repairs.len()),
                    });
                    pause_repairs = Some(repairs);
                }
            }
            Err(e) => {
                let _ = send(&on_event, TranscriptionEvent::Log {
                    message: format!("English CTC 对齐跳过，保留原始时间戳：{e}"),
                });
            }
        }
        let _ = send(&on_event, TranscriptionEvent::PhaseCompleted {
            phase: "pause_alignment".into(),
            message: "English CTC 停顿边界分析完成".into(),
        });
    }

    let verification_results = if request.config.funasr_mode == "nano" {
        let _ = send(&on_event, TranscriptionEvent::PhaseStarted {
            phase: "verification".into(),
            message: "正在复听少量可疑片段".into(),
        });
        match build_selective_verification_results(
            &request.config,
            &full_wav,
            duration,
            &segments,
            temp.path(),
            cancel,
            &on_event,
        ).await {
            Ok(results) => {
                if !results.is_empty() {
                    let corrected = results.iter().filter(|r| matches!(r.decision, crate::transcript::verification::VerificationDecision::Corrected)).count();
                    let verified = results.iter().filter(|r| matches!(r.decision, crate::transcript::verification::VerificationDecision::Verified)).count();
                    let uncertain = results.len().saturating_sub(corrected + verified);
                    let _ = send(&on_event, TranscriptionEvent::Log {
                        message: format!(
                            "Selective Verify 完成 · 候选 {} · 已确认 {} · 已纠正 {} · 不确定 {}",
                            results.len(), verified, corrected, uncertain
                        ),
                    });
                }
                results
            }
            Err(error) => {
                verification_debug_log(&request.config, "VERIFY-ERROR", verification_debug_sanitize(&error));
                let _ = send(&on_event, TranscriptionEvent::Log {
                    message: format!("Selective Verify 跳过：{error}"),
                });
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };

    if request.config.funasr_mode == "nano" {
        let _ = send(&on_event, TranscriptionEvent::PhaseCompleted {
            phase: "verification".into(),
            message: format!("可疑片段复听完成 · {} 个结果", verification_results.len()),
        });
    }

    let bridge_repairs = if request.config.funasr_mode == "nano" {
        let _ = send(&on_event, TranscriptionEvent::PhaseStarted {
            phase: "boundary_review".into(),
            message: "正在检查跨段异常边界".into(),
        });
        match build_selective_bridge_repairs(
            &request.config,
            &full_wav,
            duration,
            &segments,
            &detected_language,
            temp.path(),
            cancel,
            &on_event,
        ).await {
            Ok(repairs) => repairs,
            Err(error) => {
                let _ = send(&on_event, TranscriptionEvent::Log {
                    message: format!("Bridge 边界复听跳过：{error}"),
                });
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };
    if request.config.funasr_mode == "nano" {
        let _ = send(&on_event, TranscriptionEvent::PhaseCompleted {
            phase: "boundary_review".into(),
            message: format!("跨段边界检查完成 · {} 个修复", bridge_repairs.len()),
        });
    }

    send(
        &on_event,
        TranscriptionEvent::Finished {
            segments,
            language: Some(detected_language),
            pause_repairs: pause_repairs.unwrap_or_default(),
            bridge_repairs,
            verification_results,
        },
    )?;
    Ok(())
}

/// Run the OpenASR native server once and feed it fixed, slightly-overlapping
/// windows.  MOSS returns a complete response per window, so snapshots are
/// emitted after each response and the overlap is removed at the segment level.
/// The frontend only sees the stitched Raw timeline; chunk boundaries never
/// become user-facing subtitle metadata.
async fn run_openasr_moss(
    request: StartTranscriptionRequest,
    on_event: Channel<TranscriptionEvent>,
    cancel: &AtomicBool,
) -> Result<(), String> {
    if request.config.openasr_runtime_path.trim().is_empty()
        || !Path::new(&request.config.openasr_runtime_path).is_file()
    {
        return Err(format!("OpenASR Runtime 不存在：{}", request.config.openasr_runtime_path));
    }
    if request.config.openasr_model_path.trim().is_empty()
        || !Path::new(&request.config.openasr_model_path).is_file()
    {
        return Err(format!("MOSS q4 模型不存在：{}", request.config.openasr_model_path));
    }
    let duration = match request.media_duration {
        Some(value) if value.is_finite() && value > 0.0 => value,
        _ => probe_duration(&request.config.ffprobe_path, &request.video_path).await?,
    };
    let chunk_seconds = request.config.moss_chunk_seconds.clamp(15.0, 120.0);
    let overlap_seconds = request
        .config
        .moss_overlap_seconds
        .clamp(0.0, (chunk_seconds / 3.0).min(5.0));
    let step_seconds = (chunk_seconds - overlap_seconds).max(1.0);
    let temp = TempDir::new().map_err(|error| format!("创建 MOSS 临时目录失败：{error}"))?;

    let port = TcpListener::bind(("127.0.0.1", 0))
        .map_err(|error| format!("为 OpenASR 分配本地端口失败：{error}"))?
        .local_addr()
        .map_err(|error| format!("读取 OpenASR 本地端口失败：{error}"))?
        .port();
    let address = format!("127.0.0.1:{port}");
    let model_parent = Path::new(&request.config.openasr_model_path)
        .parent()
        .unwrap_or_else(|| Path::new("."));
    let mut server = hidden_command(&request.config.openasr_runtime_path);
    server
        .arg("serve")
        .arg("--addr")
        .arg(&address)
        .arg("--max-native-sessions-per-model")
        .arg("1")
        .arg("--model-pack")
        .arg(&request.config.openasr_model_path)
        .env("OPENASR_OFFLINE", "1")
        .env("OPENASR_HOME", model_parent)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = server
        .spawn()
        .map_err(|error| format!("无法启动 OpenASR 服务（{}）：{error}", request.config.openasr_runtime_path))?;
    let stderr = child.stderr.take();
    let stderr_task = tokio::spawn(async move {
        let Some(mut stderr) = stderr else { return Vec::new(); };
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes).await;
        bytes
    });

    let client = reqwest::Client::new();
    let health_url = format!("http://{address}/health");
    let mut healthy = false;
    for _ in 0..120 {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill().await;
            let _ = child.wait().await;
            let _ = stderr_task.await;
            send(&on_event, TranscriptionEvent::Cancelled {})?;
            return Ok(());
        }
        if let Ok(response) = client.get(&health_url).send().await {
            if response.status().is_success() {
                healthy = true;
                break;
            }
        }
        sleep(Duration::from_millis(250)).await;
    }
    if !healthy {
        let _ = child.kill().await;
        let _ = child.wait().await;
        let stderr_text = String::from_utf8_lossy(&stderr_task.await.unwrap_or_default()).into_owned();
        return Err(format!("OpenASR 服务启动超时\n{}", tail_text(&stderr_text, 4000)));
    }

    send(&on_event, TranscriptionEvent::Started { duration })?;
    send(&on_event, TranscriptionEvent::PhaseStarted {
        phase: "recognition".into(),
        message: "MOSS q4 原始识别".into(),
    })?;
    send(&on_event, TranscriptionEvent::Log {
        message: "OpenASR MOSS q4 单实例识别服务已就绪".into(),
    })?;

    let total_ms = (duration * 1000.0).round() as u64;
    send(&on_event, TranscriptionEvent::PhaseProgress {
        phase: "recognition".into(),
        completed: 0,
        total: Some(total_ms),
        unit: "milliseconds".into(),
        message: "MOSS 模型已就绪，正在逐段转写……".into(),
    })?;

    let mut segments: Vec<TranscriptSegment> = Vec::new();
    let mut chunk_index = 0usize;
    let mut start = 0.0f64;
    while start < duration {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill().await;
            let _ = child.wait().await;
            let _ = stderr_task.await;
            send(&on_event, TranscriptionEvent::Cancelled {})?;
            return Ok(());
        }
        let actual_duration = (duration - start).min(chunk_seconds);
        let wav_path = temp.path().join(format!("moss-{chunk_index:04}.wav"));
        extract_chunk(
            &request.config.ffmpeg_path,
            &request.video_path,
            start,
            actual_duration,
            &wav_path,
        )
        .await?;
        let mut attempts = 0usize;
        let (status, body) = loop {
            attempts += 1;
            let bytes = tokio::fs::read(&wav_path)
                .await
                .map_err(|error| format!("读取 MOSS 音频分片失败：{error}"))?;
            let part = reqwest::multipart::Part::bytes(bytes)
                .file_name(format!("moss-{chunk_index:04}.wav"))
                .mime_str("audio/wav")
                .map_err(|error| format!("创建 MOSS 音频请求失败：{error}"))?;
            let form = reqwest::multipart::Form::new()
                .part("file", part)
                .text("model", "moss-transcribe-diarize")
                .text("response_format", "verbose_json");
            let response = client
                .post(format!("http://{address}/v1/audio/transcriptions"))
                .multipart(form)
                .send()
                .await
                .map_err(|error| format!("MOSS 分片 {} 请求失败：{error}", chunk_index + 1))?;
            let status = response.status();
            let body = response
                .text()
                .await
                .map_err(|error| format!("读取 MOSS 分片 {} 响应失败：{error}", chunk_index + 1))?;
            if status.as_u16() == 429 && attempts < 20 {
                send(&on_event, TranscriptionEvent::Log {
                    message: format!("OpenASR MOSS 会话忙碌（429），正在等待释放重试（第 {}/20 次）……", attempts),
                })?;
                sleep(Duration::from_millis(1500)).await;
                continue;
            }
            break (status, body);
        };
        if !status.is_success() {
            return Err(format!("MOSS 分片 {} 识别失败（{}）：{}", chunk_index + 1, status, tail_text(&body, 2000)));
        }
        let local_segments = parse_openasr_segments(&body, chunk_index, actual_duration);
        let ownership_start = if chunk_index == 0 {
            0.0
        } else {
            start + overlap_seconds / 2.0
        };
        for mut segment in local_segments {
            segment.start += start;
            segment.end += start;
            if segment.start > duration + 0.25 || segment.end <= ownership_start {
                continue;
            }
            if segment.start < ownership_start {
                segment.start = ownership_start;
            }
            let duplicate = segments.iter().any(|previous| {
                normalize_overlap_text(&previous.text) == normalize_overlap_text(&segment.text)
                    && (previous.start - segment.start).abs() <= overlap_seconds + 1.0
            });
            if !duplicate {
                segments.push(segment);
            }
        }
        segments.sort_by(|left, right| {
            left.start
                .partial_cmp(&right.start)
                .unwrap_or(CmpOrdering::Equal)
                .then_with(|| left.end.partial_cmp(&right.end).unwrap_or(CmpOrdering::Equal))
        });
        let processed_until = (start + actual_duration).min(duration);
        let all_text = segments.iter().map(|segment| segment.text.as_str()).collect::<Vec<_>>().join(" ");
        send(&on_event, TranscriptionEvent::PhaseProgress {
            phase: "recognition".into(),
            completed: (processed_until * 1000.0).round() as u64,
            total: Some(total_ms),
            unit: "milliseconds".into(),
            message: format!("MOSS 正在转写 {} / {}", format_seconds_mmss(processed_until), format_seconds_mmss(duration)),
        })?;
        send(&on_event, TranscriptionEvent::Snapshot {
            segments: segments.clone(),
            language: Some(dominant_language(&all_text).to_string()),
            processed_until,
        })?;
        chunk_index += 1;
        start += step_seconds;
    }

    let _ = child.kill().await;
    let _ = child.wait().await;
    let stderr_text = String::from_utf8_lossy(&stderr_task.await.unwrap_or_default()).into_owned();
    if segments.is_empty() {
        return Err(format!("MOSS 未产生有效文本\n{}", tail_text(&stderr_text, 3000)));
    }
    let all_text = segments.iter().map(|segment| segment.text.as_str()).collect::<Vec<_>>().join(" ");
    send(&on_event, TranscriptionEvent::PhaseCompleted {
        phase: "recognition".into(),
        message: format!("MOSS 原始语音识别完成 · {} 段", segments.len()),
    })?;
    send(&on_event, TranscriptionEvent::Finished {
        segments,
        language: Some(dominant_language(&all_text).to_string()),
        pause_repairs: Vec::new(),
        bridge_repairs: Vec::new(),
        verification_results: Vec::new(),
    })?;
    Ok(())
}

fn normalize_overlap_text(text: &str) -> String {
    text.chars()
        .filter(|character| !character.is_whitespace() && !"，。！？,.!?、".contains(*character))
        .flat_map(char::to_lowercase)
        .collect()
}

fn parse_openasr_segments(body: &str, chunk_index: usize, chunk_duration: f64) -> Vec<TranscriptSegment> {
    if let Ok(value) = serde_json::from_str::<Value>(body) {
        let mut parsed = Vec::new();
        collect_openasr_segment_arrays(&value, &mut parsed, chunk_index);
        if !parsed.is_empty() {
            return parsed
                .into_iter()
                .filter(|segment| !segment.text.trim().is_empty())
                .map(|mut segment| {
                    segment.start = segment.start.max(0.0).min(chunk_duration);
                    segment.end = segment.end.max(segment.start + 0.05).min(chunk_duration.max(segment.start + 0.05));
                    segment
                })
                .collect();
        }
        let text = openasr_text_value(&value);
        if !text.trim().is_empty() {
            let tagged = parse_openasr_tagged_text(&text, chunk_index, chunk_duration);
            if !tagged.is_empty() {
                return tagged;
            }
            return vec![TranscriptSegment {
                id: format!("openasr-moss-{chunk_index}-0"),
                start: 0.0,
                end: chunk_duration,
                text: strip_model_tags(text.trim()),
            }];
        }
    }
    parse_openasr_tagged_text(body, chunk_index, chunk_duration)
}

fn parse_openasr_tagged_text(body: &str, chunk_index: usize, chunk_duration: f64) -> Vec<TranscriptSegment> {
    let Ok(pattern) = regex::Regex::new(
        r"\[([0-9]+(?:\.[0-9]+)?)\](?:\s*\[S\d+\])?\s*([^\[\]]+?)\s*\[([0-9]+(?:\.[0-9]+)?)\]",
    ) else {
        return if body.trim().is_empty() {
            Vec::new()
        } else {
            vec![TranscriptSegment {
                id: format!("openasr-moss-{chunk_index}-0"),
                start: 0.0,
                end: chunk_duration,
                text: strip_model_tags(body.trim()),
            }]
        };
    };
    let mut result = Vec::new();
    for (index, capture) in pattern.captures_iter(body).enumerate() {
        let Some(start) = parse_srt_clock(capture.get(1).map(|value| value.as_str()).unwrap_or("")) else { continue; };
        let Some(end) = parse_srt_clock(capture.get(3).map(|value| value.as_str()).unwrap_or("")) else { continue; };
        let text = strip_model_tags(capture.get(2).map(|value| value.as_str()).unwrap_or("").trim());
        if text.is_empty() { continue; }
        result.push(TranscriptSegment {
            id: format!("openasr-moss-{chunk_index}-{index}"),
            start,
            end: end.max(start + 0.05),
            text,
        });
    }
    if result.is_empty() && !body.trim().is_empty() {
        result.push(TranscriptSegment {
            id: format!("openasr-moss-{chunk_index}-0"),
            start: 0.0,
            end: chunk_duration,
            text: strip_model_tags(body.trim()),
        });
    }
    result
}

fn collect_openasr_segment_arrays(value: &Value, output: &mut Vec<TranscriptSegment>, chunk_index: usize) {
    if let Some(array) = value.as_array() {
        collect_openasr_segment_array(array, output, chunk_index);
    }
    if let Some(object) = value.as_object() {
        for key in ["segments", "utterances", "results", "chunks"] {
            if let Some(array) = object.get(key).and_then(Value::as_array) {
                collect_openasr_segment_array(array, output, chunk_index);
            }
        }
    }
    if let Some(object) = value.as_object() {
        for (key, child) in object {
            if !["segments", "utterances", "results", "chunks"].contains(&key.as_str())
                && (child.is_object() || child.is_array())
            {
                collect_openasr_segment_arrays(child, output, chunk_index);
            }
        }
    }
}

fn collect_openasr_segment_array(array: &[Value], output: &mut Vec<TranscriptSegment>, chunk_index: usize) {
        for (index, item) in array.iter().enumerate() {
            let start = item.get("start").and_then(Value::as_f64).or_else(|| item.get("start_time").and_then(Value::as_f64));
            let end = item.get("end").and_then(Value::as_f64).or_else(|| item.get("end_time").and_then(Value::as_f64));
            let text = item.get("text").and_then(Value::as_str).or_else(|| item.get("transcript").and_then(Value::as_str));
            if let (Some(start), Some(end), Some(text)) = (start, end, text) {
                output.push(TranscriptSegment {
                    id: format!("openasr-moss-{chunk_index}-{index}"),
                    start,
                    end,
                    text: strip_model_tags(text.trim()),
                });
            }
        }
}

fn openasr_text_value(value: &Value) -> String {
    if let Some(text) = value.as_str() { return text.to_string(); }
    if let Some(object) = value.as_object() {
        for key in ["text", "transcript", "transcription", "output"] {
            if let Some(candidate) = object.get(key) {
                let text = openasr_text_value(candidate);
                if !text.trim().is_empty() { return text; }
            }
        }
        for child in object.values() {
            let text = openasr_text_value(child);
            if !text.trim().is_empty() { return text; }
        }
    }
    if let Some(array) = value.as_array() {
        return array.iter().map(openasr_text_value).filter(|text| !text.trim().is_empty()).collect::<Vec<_>>().join(" ");
    }
    String::new()
}

async fn probe_duration(ffprobe: &str, video: &str) -> Result<f64, String> {
    let out = hidden_command(ffprobe)
        .args(["-v", "error", "-show_entries", "format=duration", "-of", "default=noprint_wrappers=1:nokey=1", video])
        .output().await
        .map_err(|e| format!("无法启动 ffprobe（{ffprobe}）：{e}"))?;
    if !out.status.success() {
        return Err(format!("ffprobe 读取视频时长失败：{}", String::from_utf8_lossy(&out.stderr)));
    }
    String::from_utf8_lossy(&out.stdout).trim().parse::<f64>().map_err(|e| format!("无法解析视频时长：{e}"))
}

async fn extract_chunk(ffmpeg: &str, video: &str, start: f64, duration: f64, wav: &Path) -> Result<(), String> {
    let out = hidden_command(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-ss"])
        .arg(format!("{start:.3}"))
        .args(["-t"])
        .arg(format!("{duration:.3}"))
        .args(["-i", video, "-vn", "-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le"])
        .arg(wav)
        .output().await
        .map_err(|e| format!("无法启动 ffmpeg（{ffmpeg}）：{e}"))?;
    if !out.status.success() {
        return Err(format!("FFmpeg 提取音频失败：{}", String::from_utf8_lossy(&out.stderr)));
    }
    Ok(())
}

#[cfg(test)]
mod transcriber_tests {
    use super::{dominant_language, is_chinese_text};

    #[test]
    fn language_detection_distinguishes_chinese_with_english_terms() {
        assert!(is_chinese_text("我们今天使用 Transformer 和 PyTorch 来训练一个大语言模型。"));
        assert_eq!(dominant_language("我们今天使用 Transformer 和 PyTorch 来训练一个大语言模型。"), "Chinese");
        assert!(!is_chinese_text("I'm an ugly duckling, quick to the party."));
        assert_eq!(dominant_language("I'm an ugly duckling, quick to the party."), "English");
        assert_eq!(dominant_language("怖いね。俺は。はい。"), "Japanese");
    }

    #[test]
    fn srt_stream_parser_parses_consecutive_cues() {
        use super::SrtStreamParser;
        let mut parser = SrtStreamParser::new();
        assert!(parser.push_line("1".into()).is_none());
        assert!(parser.push_line("00:00:01,000 --> 00:00:04,500".into()).is_none());
        assert!(parser.push_line("Hello world!".into()).is_none());
        let seg1 = parser.push_line("".into()).expect("segment 1");
        assert_eq!(seg1.text, "Hello world!");
        assert_eq!(seg1.start, 1.0);
        assert_eq!(seg1.end, 4.5);

        assert!(parser.push_line("2".into()).is_none());
        assert!(parser.push_line("00:00:05,200 --> 00:00:08,100".into()).is_none());
        assert!(parser.push_line("This is line 2.".into()).is_none());
        let seg2 = parser.push_line("3".into()).expect("segment 2");
        assert_eq!(seg2.text, "This is line 2.");
        assert_eq!(seg2.start, 5.2);
        assert_eq!(seg2.end, 8.1);

        assert!(parser.push_line("00:00:08,500 --> 00:00:10,000".into()).is_none());
        assert!(parser.push_line("Final sentence.".into()).is_none());
        let seg3 = parser.flush().expect("segment 3");
        assert_eq!(seg3.text, "Final sentence.");
        assert_eq!(seg3.start, 8.5);
        assert_eq!(seg3.end, 10.0);
    }
}

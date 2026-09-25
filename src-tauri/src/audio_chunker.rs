use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    process::Stdio,
    sync::atomic::{AtomicBool, Ordering},
};
use tokio::{io::AsyncReadExt, process::Command};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn hidden_command(program: impl AsRef<Path>) -> Command {
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

/// 表示一段语音中的停顿/静音区间（秒）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioPause {
    pub start: f64,
    pub end: f64,
}

impl AudioPause {
    pub fn duration(&self) -> f64 {
        (self.end - self.start).max(0.0)
    }

    pub fn midpoint(&self) -> f64 {
        (self.start + self.end) / 2.0
    }
}

/// 规划出的自适应音频切片
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedChunk {
    pub index: usize,
    pub start: f64,
    pub end: f64,
    pub duration: f64,
    /// 是否在自然停顿点软切（若为 false 表示快嘴无停顿，使用安全重叠硬切兜底）
    pub is_soft_cut: bool,
}

/// 解析 FFmpeg `silencedetect` 滤镜输出日志
pub fn parse_silencedetect_output(stderr: &str) -> Vec<AudioPause> {
    let mut pauses = Vec::new();
    let mut current_start: Option<f64> = None;

    for line in stderr.lines() {
        if let Some(pos) = line.find("silence_start:") {
            let tail = &line[pos + "silence_start:".len()..];
            if let Some(val_str) = tail.split_whitespace().next() {
                if let Ok(val) = val_str.parse::<f64>() {
                    if val.is_finite() && val >= 0.0 {
                        current_start = Some(val);
                    }
                }
            }
        } else if let Some(pos) = line.find("silence_end:") {
            let tail = &line[pos + "silence_end:".len()..];
            // 典型输出格式: ` 12.345 | silence_duration: 0.545`
            let end_str = tail.split('|').next().unwrap_or("").trim();
            if let Some(val_str) = end_str.split_whitespace().next() {
                if let Ok(end_val) = val_str.parse::<f64>() {
                    if let Some(start_val) = current_start.take() {
                        if end_val > start_val {
                            pauses.push(AudioPause {
                                start: start_val,
                                end: end_val,
                            });
                        }
                    }
                }
            }
        }
    }

    pauses
}

/// 调用 FFmpeg 执行超轻量静音与换气停顿点检测（实时率 > 1000x，耗时极短）
pub async fn detect_silence_pauses_ffmpeg(
    ffmpeg_path: impl AsRef<Path>,
    media_path: impl AsRef<Path>,
    cancel: &AtomicBool,
) -> Result<Vec<AudioPause>, String> {
    if cancel.load(Ordering::Relaxed) {
        return Ok(Vec::new());
    }

    let mut cmd = hidden_command(ffmpeg_path.as_ref());
    cmd.arg("-hide_banner")
        .arg("-nostats")
        .arg("-vn")
        .arg("-i")
        .arg(media_path.as_ref())
        // 噪声阈值 -30dB，最小持续停顿 0.22 秒（覆盖绝大多数自然换气与句末停顿）
        .arg("-af")
        .arg("silencedetect=noise=-30dB:d=0.22")
        .arg("-f")
        .arg("null")
        .arg("-")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = cmd
        .spawn()
        .map_err(|error| format!("启动 FFmpeg 停顿点检测失败：{error}"))?;

    let stderr = child.stderr.take();
    let read_task = tokio::spawn(async move {
        let Some(mut stderr) = stderr else { return String::new(); };
        let mut buffer = String::new();
        let _ = stderr.read_to_string(&mut buffer).await;
        buffer
    });

    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Ok(Vec::new());
        }
        match child.try_wait() {
            Ok(Some(_status)) => break,
            Ok(None) => tokio::time::sleep(std::time::Duration::from_millis(50)).await,
            Err(e) => return Err(format!("等待 FFmpeg 停顿检测出错：{e}")),
        }
    }

    let stderr_text = read_task.await.unwrap_or_default();
    Ok(parse_silencedetect_output(&stderr_text))
}

/// 自适应停顿点软切片规划器：
/// 在 [current_start + min_seconds, current_start + max_seconds] 范围内搜索最优自然停顿点。
/// 若搜寻到自然停顿，则在停顿区间正中落刀（两端天然静音，0 音素撕裂）；
/// 若遇到罕见连续快嘴无停顿，退化为 target_seconds 硬切并保留 fallback_overlap 重叠防丢字。
pub fn plan_adaptive_chunks(
    total_duration: f64,
    pauses: &[AudioPause],
    target_seconds: f64,
    min_seconds: f64,
    max_seconds: f64,
    fallback_overlap: f64,
) -> Vec<PlannedChunk> {
    if total_duration <= 0.0 {
        return Vec::new();
    }

    let target_seconds = target_seconds.max(10.0);
    let min_seconds = min_seconds.clamp(5.0, target_seconds);
    let max_seconds = max_seconds.max(target_seconds + 2.0);
    let fallback_overlap = fallback_overlap.clamp(0.0, 5.0);

    let mut chunks = Vec::new();
    let mut current_start = 0.0f64;
    let mut chunk_index = 0usize;

    while current_start < total_duration {
        let remaining = total_duration - current_start;
        if remaining <= max_seconds {
            chunks.push(PlannedChunk {
                index: chunk_index,
                start: current_start,
                end: total_duration,
                duration: total_duration - current_start,
                is_soft_cut: true,
            });
            break;
        }

        let search_min = current_start + min_seconds;
        let search_max = (current_start + max_seconds).min(total_duration);
        let ideal_cut = (current_start + target_seconds).min(total_duration);

        // 寻找中点落在 [search_min, search_max] 且有效时长 >= 0.18s 的自然停顿候选
        let candidates: Vec<&AudioPause> = pauses
            .iter()
            .filter(|p| {
                let mid = p.midpoint();
                mid >= search_min && mid <= search_max && p.duration() >= 0.18
            })
            .collect();

        if let Some(best_pause) = candidates.iter().max_by(|a, b| {
            let dur_a = a.duration();
            let dur_b = b.duration();
            let dist_a = (a.midpoint() - ideal_cut).abs();
            let dist_b = (b.midpoint() - ideal_cut).abs();
            // 得分策略：停顿越长越安全，离期望时长 30s 越近越佳
            let score_a = dur_a - 0.05 * dist_a;
            let score_b = dur_b - 0.05 * dist_b;
            score_a.partial_cmp(&score_b).unwrap_or(std::cmp::Ordering::Equal)
        }) {
            // 在停顿中点平滑落刀
            let cut_point = best_pause.midpoint().clamp(search_min, search_max);
            chunks.push(PlannedChunk {
                index: chunk_index,
                start: current_start,
                end: cut_point,
                duration: cut_point - current_start,
                is_soft_cut: true,
            });
            // 下一段直接从停顿中点开始（0 重叠，天然静音无破损）
            current_start = cut_point;
        } else {
            // 兜底：未找到合适停顿点，采用 target_seconds 并在下一段保留 overlap
            let cut_point = (current_start + target_seconds).min(total_duration);
            chunks.push(PlannedChunk {
                index: chunk_index,
                start: current_start,
                end: cut_point,
                duration: cut_point - current_start,
                is_soft_cut: false,
            });
            let step = (cut_point - current_start - fallback_overlap).max(min_seconds);
            current_start += step;
        }

        chunk_index += 1;
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_silencedetect_sample_output() {
        let sample = r#"
[Parsed_silencedetect_1 @ 00000206095d4280] silence_start: 12.45
[Parsed_silencedetect_1 @ 00000206095d4280] silence_end: 13.20 | silence_duration: 0.75
[Parsed_silencedetect_1 @ 00000206095d4280] silence_start: 28.80
[Parsed_silencedetect_1 @ 00000206095d4280] silence_end: 29.50 | silence_duration: 0.70
"#;
        let pauses = parse_silencedetect_output(sample);
        assert_eq!(pauses.len(), 2);
        assert!((pauses[0].start - 12.45).abs() < 1e-4);
        assert!((pauses[0].end - 13.20).abs() < 1e-4);
        assert!((pauses[1].start - 28.80).abs() < 1e-4);
        assert!((pauses[1].end - 29.50).abs() < 1e-4);
    }

    #[test]
    fn plans_adaptive_chunks_with_natural_pauses() {
        let pauses = vec![
            AudioPause { start: 28.5, end: 29.5 },
            AudioPause { start: 58.2, end: 59.0 },
            AudioPause { start: 87.0, end: 88.0 },
        ];
        let chunks = plan_adaptive_chunks(100.0, &pauses, 30.0, 20.0, 36.0, 1.0);
        assert_eq!(chunks.len(), 4);
        // Chunk 0 切在 29.0s (第一处停顿中点)
        assert!((chunks[0].start - 0.0).abs() < 1e-3);
        assert!((chunks[0].end - 29.0).abs() < 1e-3);
        assert!(chunks[0].is_soft_cut);

        // Chunk 1 从 29.0s 到 58.6s (第二处停顿中点)
        assert!((chunks[1].start - 29.0).abs() < 1e-3);
        assert!((chunks[1].end - 58.6).abs() < 1e-3);
        assert!(chunks[1].is_soft_cut);

        // Chunk 2 从 58.6s 到 87.5s (第三处停顿中点)
        assert!((chunks[2].start - 58.6).abs() < 1e-3);
        assert!((chunks[2].end - 87.5).abs() < 1e-3);
        assert!(chunks[2].is_soft_cut);

        // Chunk 3 从 87.5s 到 100.0s (结尾)
        assert!((chunks[3].start - 87.5).abs() < 1e-3);
        assert!((chunks[3].end - 100.0).abs() < 1e-3);
    }

    #[test]
    fn plans_chunks_fallback_when_no_pauses() {
        let pauses = Vec::new();
        let chunks = plan_adaptive_chunks(70.0, &pauses, 30.0, 20.0, 36.0, 1.0);
        assert_eq!(chunks.len(), 3);
        assert!(!chunks[0].is_soft_cut);
        assert_eq!(chunks[0].start, 0.0);
        assert_eq!(chunks[0].end, 30.0);
        // Fallback hard cut with 1.0s overlap -> next starts at 29.0
        assert_eq!(chunks[1].start, 29.0);
    }

    #[test]
    fn short_audio_produces_single_chunk() {
        let pauses = Vec::new();
        let chunks = plan_adaptive_chunks(15.0, &pauses, 30.0, 20.0, 36.0, 1.0);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].start, 0.0);
        assert_eq!(chunks[0].end, 15.0);
    }
}

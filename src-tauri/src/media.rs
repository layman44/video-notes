use serde::{Deserialize, Serialize};
use std::{
    env, fs,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
    time::Duration,
};
use tauri::{AppHandle, Emitter, Manager};
use url::Url;

const AUDIO_CHUNK_SECONDS: f64 = 2.0 * 60.0;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub(crate) fn clean_path(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if s.starts_with(r"\\?\") || s.starts_with("//?/") {
        PathBuf::from(&s[4..])
    } else {
        path.to_path_buf()
    }
}

fn media_command(program: &Path) -> Command {
    let clean_prog = clean_path(program);
    #[cfg(windows)]
    {
        let mut command = Command::new(clean_prog);
        command.creation_flags(CREATE_NO_WINDOW);
        command
    }
    #[cfg(not(windows))]
    {
        Command::new(clean_prog)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourcePreview {
    pub title: String,
    pub platform: String,
    pub duration: String,
    pub source_url: String,
    pub author: Option<String>,
    pub thumbnail_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaToolStatus {
    pub name: &'static str,
    pub available: bool,
    pub path: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaToolsStatus {
    pub ready: bool,
    pub yt_dlp: MediaToolStatus,
    pub ffmpeg: MediaToolStatus,
}

#[derive(Debug, Clone)]
pub struct MediaToolPaths {
    pub yt_dlp: PathBuf,
    pub ffmpeg: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaProgress {
    pub job_id: String,
    pub stage: &'static str,
    pub progress: u8,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioChunk {
    pub index: usize,
    pub path: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaPreparationResult {
    pub task_dir: String,
    pub source_file: String,
    #[serde(default)]
    pub video_file: Option<String>,
    #[serde(default)]
    pub thumbnail_file: Option<String>,
    pub metadata_file: Option<String>,
    pub duration_seconds: f64,
    pub chunks: Vec<AudioChunk>,
}

#[derive(Debug, Deserialize)]
struct YtDlpMetadata {
    title: Option<String>,
    duration: Option<f64>,
    webpage_url: Option<String>,
    original_url: Option<String>,
    uploader: Option<String>,
    thumbnail: Option<String>,
    extractor: Option<String>,
    extractor_key: Option<String>,
}

#[derive(Debug, Clone, Copy)]
enum OutputStream {
    Stdout,
    Stderr,
}

fn tool_status(name: &'static str, path: Option<PathBuf>) -> MediaToolStatus {
    MediaToolStatus {
        name,
        available: path.is_some(),
        path: path.map(|value| value.to_string_lossy().into_owned()),
        version: None,
    }
}

fn candidate_tool_dirs(app: &AppHandle) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(override_dir) = env::var_os("VIDEO_NOTES_TOOLS_DIR") {
        candidates.push(clean_path(&PathBuf::from(override_dir)));
    }
    if let Ok(resource_dir) = app.path().resource_dir() {
        let clean_res = clean_path(&resource_dir);
        candidates.push(clean_res.join("resources").join("tools"));
        candidates.push(clean_res.join("tools"));
        candidates.push(clean_res);
    }
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join("tools"),
    );
    candidates
}

pub(crate) fn find_on_path(filename: &str) -> Option<PathBuf> {
    env::var_os("PATH").and_then(|paths| {
        env::split_paths(&paths)
            .map(|directory| directory.join(filename))
            .find(|candidate| candidate.is_file())
    })
}

pub(crate) fn find_node_runtime() -> Option<PathBuf> {
    find_on_path("node.exe")
        .or_else(|| find_on_path("node"))
        .or_else(|| {
            let standard_path = PathBuf::from(r"C:\Program Files\nodejs\node.exe");
            if standard_path.is_file() {
                Some(standard_path)
            } else {
                None
            }
        })
}

pub(crate) fn find_tool(app: &AppHandle, filename: &str) -> Option<PathBuf> {
    candidate_tool_dirs(app)
        .into_iter()
        .map(|directory| directory.join(filename))
        .find(|candidate| candidate.is_file())
        .or_else(|| find_on_path(filename))
}

pub fn inspect_media_tools(app: &AppHandle) -> MediaToolsStatus {
    let yt_dlp = tool_status("yt-dlp", find_tool(app, "yt-dlp.exe"));
    let ffmpeg = tool_status("FFmpeg", find_tool(app, "ffmpeg.exe"));
    MediaToolsStatus {
        ready: yt_dlp.available && ffmpeg.available,
        yt_dlp,
        ffmpeg,
    }
}

pub fn resolve_media_tools(app: &AppHandle) -> Result<MediaToolPaths, String> {
    let yt_dlp = find_tool(app, "yt-dlp.exe");
    let ffmpeg = find_tool(app, "ffmpeg.exe");
    let missing = [
        ("yt-dlp", yt_dlp.is_none()),
        ("FFmpeg", ffmpeg.is_none()),
    ]
    .into_iter()
    .filter_map(|(name, is_missing)| is_missing.then_some(name))
    .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(format!(
            "缺少媒体组件：{}。请重新安装完整版本的 VideoNotes",
            missing.join("、")
        ));
    }

    Ok(MediaToolPaths {
        yt_dlp: yt_dlp.expect("checked above"),
        ffmpeg: ffmpeg.expect("checked above"),
    })
}

fn platform_for_host(host: &str) -> &'static str {
    let host = host.to_ascii_lowercase();
    if host == "bilibili.com"
        || host.ends_with(".bilibili.com")
        || host == "b23.tv"
        || host.ends_with(".b23.tv")
    {
        "bilibili"
    } else if host == "douyin.com" || host.ends_with(".douyin.com") {
        "douyin"
    } else if host == "youtube.com"
        || host.ends_with(".youtube.com")
        || host == "youtu.be"
        || host.ends_with(".youtu.be")
    {
        "youtube"
    } else if host == "tiktok.com" || host.ends_with(".tiktok.com") {
        "tiktok"
    } else if host == "twitter.com"
        || host.ends_with(".twitter.com")
        || host == "x.com"
        || host.ends_with(".x.com")
    {
        "twitter"
    } else if host == "kuaishou.com" || host.ends_with(".kuaishou.com") {
        "kuaishou"
    } else if host == "xiaohongshu.com"
        || host.ends_with(".xiaohongshu.com")
        || host == "xhslink.com"
        || host.ends_with(".xhslink.com")
    {
        "xiaohongshu"
    } else if host == "weibo.com"
        || host.ends_with(".weibo.com")
        || host == "weibo.cn"
        || host.ends_with(".weibo.cn")
    {
        "weibo"
    } else {
        "video"
    }
}

pub fn extract_supported_url(input: &str) -> Result<(String, &'static str), String> {
    for candidate in input.split(|character: char| {
        character.is_whitespace()
            || matches!(
                character,
                ',' | '，'
                    | '。'
                    | '；'
                    | ';'
                    | ')'
                    | '）'
                    | ']'
                    | '】'
                    | '}'
                    | '"'
                    | '\''
                    | '>'
                    | '》'
            )
    }) {
        let Ok(url) = Url::parse(candidate) else {
            continue;
        };
        if !matches!(url.scheme(), "http" | "https") {
            continue;
        }
        let Some(host) = url.host_str() else {
            continue;
        };
        let host_lower = host.to_ascii_lowercase();
        if host_lower == "localhost"
            || host_lower == "127.0.0.1"
            || host_lower == "::1"
            || host_lower == "0.0.0.0"
        {
            continue;
        }
        let platform = platform_for_host(host);

        let normalized_url = if platform == "douyin" {
            if let Some((_, modal_id)) = url.query_pairs().find(|(key, _)| key == "modal_id") {
                let modal_id = modal_id.trim();
                if !modal_id.is_empty() && modal_id.chars().all(|character| character.is_ascii_digit()) {
                    format!("https://www.douyin.com/video/{modal_id}")
                } else {
                    url.into()
                }
            } else {
                url.into()
            }
        } else {
            url.into()
        };

        return Ok((normalized_url, platform));
    }

    Err("未在输入内容中找到有效的视频链接".to_string())
}

fn format_duration(seconds: f64) -> String {
    let total = seconds.max(0.0).round() as u64;
    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    let seconds = total % 60;
    if hours > 0 {
        format!("{hours:02}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

fn friendly_process_error(stderr: &str, fallback: &str) -> String {
    let normalized = stderr.to_ascii_lowercase();
    if (normalized.contains("sign in") && normalized.contains("bot"))
        || (normalized.contains("confirm") && normalized.contains("bot"))
    {
        "YouTube 触发了防机器人拦截（Sign in to confirm you're not a bot），请尝试在代理软件中切换节点，或在应用数据目录放置 cookies.txt".to_string()
    } else if normalized.contains("login")
        || normalized.contains("sign in")
        || normalized.contains("cookies")
    {
        "该视频需要登录或平台授权，当前版本仅支持公开且无需登录的内容".to_string()
    } else if normalized.contains("private") {
        "该视频不是公开内容，无法处理".to_string()
    } else if normalized.contains("unsupported url") {
        "该链接暂不受当前媒体解析组件支持".to_string()
    } else if normalized.contains("unavailable") || normalized.contains("deleted") {
        "视频不可用、已删除或受到地区限制".to_string()
    } else if normalized.contains("timed out")
        || normalized.contains("network")
        || normalized.contains("connection")
    {
        "连接视频平台失败，请检查网络后重试".to_string()
    } else {
        stderr
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .map(|line| line.trim().chars().take(300).collect())
            .unwrap_or_else(|| fallback.to_string())
    }
}

fn is_cookie_file_fresh(path: &Path) -> bool {
    if let Ok(metadata) = fs::metadata(path) {
        if metadata.is_file() && metadata.len() > 100 {
            if let Ok(modified) = metadata.modified() {
                if let Ok(elapsed) = modified.elapsed() {
                    return elapsed < Duration::from_secs(3 * 60);
                }
            }
        }
    }
    false
}

pub fn ensure_douyin_cookies(
    app: &AppHandle,
    source_url: &str,
    force_refresh: bool,
) -> Result<PathBuf, String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("无法定位应用数据目录：{error}"))?;
    fs::create_dir_all(&app_data_dir).map_err(|error| format!("无法创建数据目录：{error}"))?;
    let cookie_file = app_data_dir.join("douyin_cookies.txt");

    if !force_refresh && is_cookie_file_fresh(&cookie_file) {
        return Ok(cookie_file);
    }

    let _ = fs::remove_file(&cookie_file);

    let script_path = find_tool(app, "douyin-cookies.mjs")
        .ok_or_else(|| "缺少抖音反爬解析脚本（douyin-cookies.mjs）".to_string())?;

    let node_path = find_node_runtime()
        .ok_or_else(|| "未检测到 Node.js 运行时环境，解析抖音链接需要 Node.js 支持".to_string())?;

    let output = media_command(&node_path)
        .arg(&script_path)
        .arg("--url")
        .arg(source_url)
        .arg("--output")
        .arg(&cookie_file)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("解析抖音链接失败（缺少运行环境）：{error}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let combined = if !stderr.trim().is_empty() {
            stderr.trim().to_string()
        } else {
            stdout.trim().to_string()
        };

        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&combined) {
            if let Some(err) = val.get("error").and_then(|v| v.as_str()) {
                return Err(format!("获取视频信息失败：{err}"));
            }
        }
        return Err(format!("获取视频信息失败：{combined}"));
    }

    if !cookie_file.is_file() || fs::metadata(&cookie_file).map(|m| m.len()).unwrap_or(0) == 0 {
        return Err("获取视频信息失败，请检查链接有效性或稍后重试".to_string());
    }

    Ok(cookie_file)
}

const BROWSER_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

fn execute_probe_command(
    tools: &MediaToolPaths,
    source_url: &str,
    cookie_file: Option<&Path>,
) -> Result<std::process::Output, String> {
    let mut command = media_command(&tools.yt_dlp);
    command.args([
        "--ignore-config",
        "--no-playlist",
        "--no-warnings",
        "--no-write-subs",
        "--no-write-auto-subs",
        "--socket-timeout",
        "15",
        "--retries",
        "3",
        "--dump-single-json",
    ]);

    if let Some(node_path) = find_node_runtime() {
        command.args(["--js-runtimes", &format!("node:{}", clean_path(&node_path).display())]);
    }

    if let Some(cookie_path) = cookie_file {
        command.arg("--cookies").arg(cookie_path);
        command.arg("--user-agent").arg(BROWSER_USER_AGENT);
        command.args(["--add-header", "Referer:https://www.douyin.com/"]);
    }

    command.arg(source_url);
    command
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("无法启动媒体解析组件：{error}"))
}

pub fn probe_source(
    app: &AppHandle,
    tools: &MediaToolPaths,
    input: &str,
) -> Result<SourcePreview, String> {
    let (source_url, platform) = extract_supported_url(input)?;

    let mut cookie_file = if platform == "douyin" {
        Some(ensure_douyin_cookies(app, &source_url, false)?)
    } else if platform == "youtube" {
        let app_data_dir = app.path().app_data_dir().ok();
        let yt_cookie = app_data_dir.as_ref().map(|d| d.join("youtube_cookies.txt")).filter(|p| p.is_file());
        let generic_cookie = app_data_dir.as_ref().map(|d| d.join("cookies.txt")).filter(|p| p.is_file());
        yt_cookie.or(generic_cookie)
    } else {
        None
    };

    let mut output = execute_probe_command(tools, &source_url, cookie_file.as_deref())?;

    // 如果是抖音且第一次解析失败，可能是 Cookie 过期或失效，强制刷新 Cookie 后重试一次
    if !output.status.success() && platform == "douyin" {
        if let Ok(fresh_cookie) = ensure_douyin_cookies(app, &source_url, true) {
            cookie_file = Some(fresh_cookie);
            if let Ok(retry_output) =
                execute_probe_command(tools, &source_url, cookie_file.as_deref())
            {
                output = retry_output;
            }
        }
    }

    if !output.status.success() {
        return Err(friendly_process_error(
            &String::from_utf8_lossy(&output.stderr),
            "无法解析该视频链接",
        ));
    }

    let metadata: YtDlpMetadata = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("媒体信息格式无效：{error}"))?;
    let canonical_url = metadata
        .webpage_url
        .or(metadata.original_url)
        .filter(|url| extract_supported_url(url).is_ok())
        .unwrap_or(source_url);
    let resolved_platform = metadata
        .extractor_key
        .as_deref()
        .or(metadata.extractor.as_deref())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_else(|| platform.to_string());

    Ok(SourcePreview {
        title: metadata.title.unwrap_or_else(|| "未命名视频".to_string()),
        platform: resolved_platform,
        duration: metadata
            .duration
            .map(format_duration)
            .unwrap_or_else(|| "--:--".to_string()),
        source_url: canonical_url,
        author: metadata.uploader,
        thumbnail_url: metadata.thumbnail,
    })
}

fn emit_progress(
    app: &AppHandle,
    job_id: &str,
    stage: &'static str,
    progress: u8,
    message: impl Into<String>,
) {
    let _ = app.emit(
        "media-progress",
        MediaProgress {
            job_id: job_id.to_string(),
            stage,
            progress,
            message: message.into(),
        },
    );
}

fn spawn_line_reader<R: Read + Send + 'static>(
    reader: R,
    stream: OutputStream,
    sender: mpsc::Sender<(OutputStream, String)>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        let mut buffer = Vec::new();
        loop {
            buffer.clear();
            match reader.read_until(b'\n', &mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let line = String::from_utf8_lossy(&buffer).trim().to_string();
                    if !line.is_empty() && sender.send((stream, line)).is_err() {
                        break;
                    }
                }
            }
        }
    })
}

fn parse_percent(line: &str) -> Option<u8> {
    let percent = line
        .strip_prefix("download:")?
        .trim()
        .trim_end_matches('%')
        .trim()
        .parse::<f64>()
        .ok()?;
    Some(percent.round().clamp(0.0, 100.0) as u8)
}

pub(crate) fn validate_job_id(job_id: &str) -> Result<(), String> {
    if job_id.is_empty()
        || job_id.len() > 64
        || !job_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err("任务标识无效".to_string());
    }
    Ok(())
}

fn find_video_file(source_dir: &Path) -> Result<PathBuf, String> {
    fs::read_dir(source_dir)
        .map_err(|error| format!("无法读取任务目录：{error}"))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.is_file()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("video."))
                && !matches!(
                    path.extension().and_then(|value| value.to_str()),
                    Some("json" | "jpg" | "jpeg" | "png" | "webp" | "part" | "ytdl")
                )
        })
        .ok_or_else(|| "下载完成后未找到本地视频".to_string())
}

fn find_thumbnail_file(source_dir: &Path) -> Option<PathBuf> {
    ["video.jpg", "video.jpeg", "video.png", "video.webp"]
        .into_iter()
        .map(|name| source_dir.join(name))
        .find(|path| path.is_file())
}

fn format_selector_for_quality(quality: &str) -> &'static str {
    match quality {
        "1080p" => "bestvideo[height<=1080][vcodec^=avc1]+bestaudio[acodec^=mp4a]/best[height<=1080][vcodec^=avc1]/bestvideo[height<=1080]+bestaudio/best[height<=1080]/best",
        "best" => "bestvideo[vcodec^=avc1]+bestaudio[acodec^=mp4a]/best[vcodec^=avc1]/bestvideo+bestaudio/best",
        _ => "bestvideo[height<=720][vcodec^=avc1]+bestaudio[acodec^=mp4a]/best[height<=720][vcodec^=avc1]/bestvideo[height<=720]+bestaudio/best[height<=720]/best",
    }
}

fn run_ytdlp_download(
    app: &AppHandle,
    tools: &MediaToolPaths,
    job_id: &str,
    source_url: &str,
    source_dir: &Path,
    quality: &str,
    cookie_file: Option<&Path>,
    cancelled: &AtomicBool,
) -> Result<PathBuf, String> {
    let ffmpeg_dir = tools.ffmpeg.parent().unwrap_or(Path::new("."));
    let ffmpeg_dir_clean = clean_path(ffmpeg_dir);
    let source_dir_clean = clean_path(source_dir);

    let mut command = media_command(&tools.yt_dlp);
    command.args([
        "--ignore-config",
        "--no-playlist",
        "--no-warnings",
        "--no-write-subs",
        "--no-write-auto-subs",
        "--no-write-comments",
        "--socket-timeout",
        "15",
        "--retries",
        "3",
        "--write-thumbnail",
        "--convert-thumbnails",
        "jpg",
        "--write-info-json",
        "--clean-info-json",
        "--newline",
        "--progress-delta",
        "0.5",
        "--progress-template",
        "download:%(progress._percent_str)s",
        "--print",
        "after_move:filepath",
        "--format",
        format_selector_for_quality(quality),
        "--merge-output-format",
        "mp4",
        "--paths",
    ])
    .arg(&source_dir_clean)
    .args(["--output", "video.%(ext)s", "--ffmpeg-location"])
    .arg(&ffmpeg_dir_clean);

    if let Some(node_path) = find_node_runtime() {
        command.args(["--js-runtimes", &format!("node:{}", clean_path(&node_path).display())]);
    }

    if let Some(cookies) = cookie_file {
        command.arg("--cookies").arg(clean_path(cookies));
        if source_url.contains("douyin.com") {
            command.arg("--user-agent").arg(BROWSER_USER_AGENT);
            command.args(["--add-header", "Referer:https://www.douyin.com/"]);
        }
    } else if source_url.contains("douyin.com") {
        command.arg("--user-agent").arg(BROWSER_USER_AGENT);
        command.args(["--add-header", "Referer:https://www.douyin.com/"]);
    } else if source_url.contains("bilibili.com") || source_url.contains("b23.tv") {
        command.arg("--user-agent").arg(BROWSER_USER_AGENT);
        command.args(["--add-header", "Referer:https://www.bilibili.com/"]);
    }

    command.arg(source_url);

    println!("\n========================================================");
    println!("[VIDEO_DOWNLOAD_START]");
    println!("  Job ID:     {}", job_id);
    println!("  Source URL: {}", source_url);
    println!("  Target Dir: {}", source_dir.display());
    println!("  Command:    {:?}", command);
    println!("========================================================\n");

    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("无法启动视频下载：{error}"))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "无法读取下载进度".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "无法读取下载错误".to_string())?;
    let (sender, receiver) = mpsc::channel();
    let stdout_reader = spawn_line_reader(stdout, OutputStream::Stdout, sender.clone());
    let stderr_reader = spawn_line_reader(stderr, OutputStream::Stderr, sender);
    let mut error_lines = Vec::new();

    let status = loop {
        while let Ok((stream, line)) = receiver.try_recv() {
            match stream {
                OutputStream::Stdout => {
                    println!("[yt-dlp stdout] {}", line);
                    if let Some(progress) = parse_percent(&line) {
                        emit_progress(
                            app,
                            job_id,
                            "download",
                            progress,
                            format!("正在下载视频 {progress}%"),
                        );
                    }
                }
                OutputStream::Stderr => {
                    println!("[yt-dlp stderr] {}", line);
                    error_lines.push(line);
                }
            }
        }

        if cancelled.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            return Err("任务已取消，已下载的临时数据将用于下次继续".to_string());
        }
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            break status;
        }
        thread::sleep(Duration::from_millis(100));
    };

    let _ = stdout_reader.join();
    let _ = stderr_reader.join();
    while let Ok((stream, line)) = receiver.try_recv() {
        if matches!(stream, OutputStream::Stderr) {
            println!("[yt-dlp stderr] {}", line);
            error_lines.push(line);
        }
    }
    if !status.success() {
        let err_summary = error_lines.join("\n");
        println!("\n[VIDEO_DOWNLOAD_FAILED] status: {:?}, stderr:\n{}\n", status, err_summary);
        return Err(friendly_process_error(
            &err_summary,
            "视频下载失败",
        ));
    }

    println!("\n[VIDEO_DOWNLOAD_SUCCESS] Job ID: {}\n", job_id);
    emit_progress(app, job_id, "download", 100, "视频已保存到本地");
    find_video_file(source_dir)
}

fn download_video(
    app: &AppHandle,
    tools: &MediaToolPaths,
    job_id: &str,
    source_url: &str,
    source_dir: &Path,
    quality: &str,
    cancelled: &AtomicBool,
) -> Result<PathBuf, String> {
    emit_progress(app, job_id, "download", 0, "正在获取视频流……");

    let platform = extract_supported_url(source_url)
        .map(|(_, platform)| platform)
        .unwrap_or("video");
    let is_douyin = platform == "douyin";
    let is_youtube = platform == "youtube";

    let cookie_file = if is_douyin {
        emit_progress(app, job_id, "download", 2, "正在连接视频平台……");
        Some(ensure_douyin_cookies(app, source_url, false)?)
    } else if is_youtube {
        let app_data_dir = app.path().app_data_dir().ok();
        let yt_cookie = app_data_dir.as_ref().map(|d| d.join("youtube_cookies.txt")).filter(|p| p.is_file());
        let generic_cookie = app_data_dir.as_ref().map(|d| d.join("cookies.txt")).filter(|p| p.is_file());
        yt_cookie.or(generic_cookie)
    } else {
        None
    };

    let result = run_ytdlp_download(
        app,
        tools,
        job_id,
        source_url,
        source_dir,
        quality,
        cookie_file.as_deref(),
        cancelled,
    );

    // 如果抖音下载失败且未被用户取消，强制刷新 Cookie 重试一次
    if result.is_err() && is_douyin && !cancelled.load(Ordering::Relaxed) {
        emit_progress(app, job_id, "download", 2, "连接已超时，正在自动重试……");
        if let Ok(fresh_cookie) = ensure_douyin_cookies(app, source_url, true) {
            return run_ytdlp_download(
                app,
                tools,
                job_id,
                source_url,
                source_dir,
                quality,
                Some(&fresh_cookie),
                cancelled,
            );
        }
    }

    result
}

fn parse_ffmpeg_duration(output: &str) -> Option<f64> {
    let marker = "Duration:";
    let pos = output.find(marker)?;
    let rest = output[pos + marker.len()..].trim_start();
    let duration_str = rest.split(',').next()?.trim();
    let parts = duration_str.split(':').collect::<Vec<_>>();
    if parts.len() == 3 {
        let hours: f64 = parts[0].trim().parse().ok()?;
        let minutes: f64 = parts[1].trim().parse().ok()?;
        let seconds: f64 = parts[2].trim().parse().ok()?;
        let total = hours * 3600.0 + minutes * 60.0 + seconds;
        if total > 0.0 {
            Some(total)
        } else {
            None
        }
    } else {
        None
    }
}

fn probe_duration(ffmpeg: &Path, source_file: &Path) -> Result<f64, String> {
    let output = media_command(ffmpeg)
        .args(["-hide_banner", "-i"])
        .arg(source_file)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("无法启动音频检测：{error}"))?;
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    parse_ffmpeg_duration(&text).ok_or_else(|| "无法读取音频时长".to_string())
}

fn normalize_audio(
    app: &AppHandle,
    tools: &MediaToolPaths,
    job_id: &str,
    source_file: &Path,
    task_dir: &Path,
    chunks_dir: &Path,
    duration_seconds: f64,
    cancelled: &AtomicBool,
) -> Result<Vec<AudioChunk>, String> {
    fs::create_dir_all(chunks_dir).map_err(|error| format!("无法创建音频缓存目录：{error}"))?;
    emit_progress(app, job_id, "normalize", 0, "正在提取与优化音频……");

    let clean_task_dir = clean_path(task_dir);
    let chunks_dir_clean = clean_path(chunks_dir);
    let source_file_clean = clean_path(source_file);
    let media_wav = clean_task_dir.join("media.wav");

    println!("\n========================================================");
    println!("[AUDIO_EXTRACT_START]");
    println!("  Job ID:      {}", job_id);
    println!("  Source File: {}", source_file_clean.display());
    println!("  Media WAV:   {}", media_wav.display());
    println!("========================================================\n");

    let mut child = media_command(&tools.ffmpeg)
        .args(["-hide_banner", "-nostdin", "-y", "-i"])
        .arg(&source_file_clean)
        .args([
            "-vn",
            "-map_metadata",
            "-1",
            "-ac",
            "1",
            "-ar",
            "16000",
            "-c:a",
            "pcm_s16le",
            "-threads",
            "1",
            "-progress",
            "pipe:1",
            "-nostats",
        ])
        .arg(&media_wav)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("无法启动音频处理：{error}"))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "无法读取转码进度".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "无法读取转码错误".to_string())?;
    let (sender, receiver) = mpsc::channel();
    let stdout_reader = spawn_line_reader(stdout, OutputStream::Stdout, sender.clone());
    let stderr_reader = spawn_line_reader(stderr, OutputStream::Stderr, sender);
    let mut error_lines = Vec::new();

    let status = loop {
        while let Ok((stream, line)) = receiver.try_recv() {
            match stream {
                OutputStream::Stdout if line.starts_with("out_time_us=") => {
                    let microseconds = line
                        .trim_start_matches("out_time_us=")
                        .parse::<f64>()
                        .unwrap_or_default();
                    let progress = if duration_seconds > 0.0 {
                        ((microseconds / 1_000_000.0 / duration_seconds) * 100.0)
                            .round()
                            .clamp(0.0, 95.0) as u8
                    } else {
                        0
                    };
                    emit_progress(
                        app,
                        job_id,
                        "normalize",
                        progress,
                        format!("正在处理音频 {progress}%"),
                    );
                }
                OutputStream::Stderr => error_lines.push(line),
                OutputStream::Stdout => {}
            }
        }

        if cancelled.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            return Err("任务已取消".to_string());
        }
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            break status;
        }
        thread::sleep(Duration::from_millis(100));
    };

    let _ = stdout_reader.join();
    let _ = stderr_reader.join();
    while let Ok((stream, line)) = receiver.try_recv() {
        if matches!(stream, OutputStream::Stderr) {
            error_lines.push(line);
        }
    }
    if !status.success() {
        return Err(friendly_process_error(
            &error_lines.join("\n"),
            "音频处理失败",
        ));
    }

    if !media_wav.is_file() || fs::metadata(&media_wav).map(|m| m.len()).unwrap_or(0) == 0 {
        return Err("音频处理失败：未生成有效音频文件".to_string());
    }

    let output_pattern = chunks_dir_clean.join("chunk-%03d.wav");
    let segment_list = chunks_dir_clean.join("chunks.csv");
    let slice_res = media_command(&tools.ffmpeg)
        .args(["-hide_banner", "-nostdin", "-y", "-i"])
        .arg(&media_wav)
        .args([
            "-c",
            "copy",
            "-f",
            "segment",
            "-segment_time",
            "120",
            "-reset_timestamps",
            "1",
            "-segment_format",
            "wav",
            "-segment_list_type",
            "csv",
            "-segment_list",
        ])
        .arg(&segment_list)
        .arg(&output_pattern)
        .stdin(Stdio::null())
        .output();

    if slice_res.is_err() || !slice_res.as_ref().unwrap().status.success() {
        let single_chunk = AudioChunk {
            index: 0,
            path: media_wav.to_string_lossy().into_owned(),
            start_seconds: 0.0,
            end_seconds: duration_seconds,
        };
        emit_progress(app, job_id, "normalize", 100, "音频准备完成");
        return Ok(vec![single_chunk]);
    }

    let mut paths = fs::read_dir(chunks_dir)
        .map_err(|error| format!("无法读取音频切片：{error}"))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("wav"))
        })
        .collect::<Vec<_>>();
    paths.sort();
    if paths.is_empty() {
        let single_chunk = AudioChunk {
            index: 0,
            path: media_wav.to_string_lossy().into_owned(),
            start_seconds: 0.0,
            end_seconds: duration_seconds,
        };
        emit_progress(app, job_id, "normalize", 100, "音频准备完成");
        return Ok(vec![single_chunk]);
    }
    let chunks = paths
        .into_iter()
        .enumerate()
        .map(|(index, path)| {
            let start_seconds = index as f64 * AUDIO_CHUNK_SECONDS;
            AudioChunk {
                index,
                path: path.to_string_lossy().into_owned(),
                start_seconds,
                end_seconds: (start_seconds + AUDIO_CHUNK_SECONDS).min(duration_seconds),
            }
        })
        .collect();
    emit_progress(app, job_id, "normalize", 100, "音频准备完成");
    Ok(chunks)
}

pub fn prepare_media(
    app: &AppHandle,
    tools: &MediaToolPaths,
    app_data_dir: &Path,
    job_id: &str,
    source_url: &str,
    quality: &str,
    cancelled: Arc<AtomicBool>,
) -> Result<MediaPreparationResult, String> {
    validate_job_id(job_id)?;
    let (source_url, _) = extract_supported_url(source_url)?;
    let task_dir = app_data_dir.join("tasks").join(job_id);
    let manifest_path = task_dir.join("media.json");
    let cached = if manifest_path.is_file() {
        let manifest = fs::read_to_string(&manifest_path).map_err(|error| error.to_string())?;
        Some(
            serde_json::from_str::<MediaPreparationResult>(&manifest)
                .map_err(|error| error.to_string())?,
        )
    } else {
        None
    };
    if let Some(cached) = cached.as_ref() {
        let chunks_ready = !cached.chunks.is_empty()
            && cached
                .chunks
                .iter()
                .all(|chunk| Path::new(&chunk.path).is_file());
        let video_ready = cached
            .video_file
            .as_deref()
            .is_some_and(|path| Path::new(path).is_file());
        if chunks_ready && video_ready {
            emit_progress(app, job_id, "ready", 100, "已复用本地视频与音频");
            return Ok(cached.clone());
        }
    }

    let source_dir = task_dir.join("source");
    let chunks_dir = task_dir.join("chunks");
    fs::create_dir_all(&source_dir).map_err(|error| format!("无法创建任务目录：{error}"))?;
    let video_file = find_video_file(&source_dir)
        .or_else(|_| download_video(app, tools, job_id, &source_url, &source_dir, quality, &cancelled))?;
    if cancelled.load(Ordering::Relaxed) {
        return Err("任务已取消".to_string());
    }
    let duration_seconds = probe_duration(&tools.ffmpeg, &video_file)?;
    let cached_chunks = cached.as_ref().filter(|manifest| {
        !manifest.chunks.is_empty()
            && manifest
                .chunks
                .iter()
                .all(|chunk| Path::new(&chunk.path).is_file())
    });
    let chunks = if let Some(manifest) = cached_chunks {
        emit_progress(app, job_id, "normalize", 100, "已复用本地已处理音频");
        manifest.chunks.clone()
    } else {
        normalize_audio(
            app,
            tools,
            job_id,
            &video_file,
            &task_dir,
            &chunks_dir,
            duration_seconds,
            &cancelled,
        )?
    };
    let metadata_file = [
        source_dir.join("video.info.json"),
        source_dir.join("source.info.json"),
    ]
    .into_iter()
    .find(|path| path.is_file())
    .map(|path| path.to_string_lossy().into_owned());
    let thumbnail_file =
        find_thumbnail_file(&source_dir).map(|path| path.to_string_lossy().into_owned());
    let result = MediaPreparationResult {
        task_dir: task_dir.to_string_lossy().into_owned(),
        source_file: video_file.to_string_lossy().into_owned(),
        video_file: Some(video_file.to_string_lossy().into_owned()),
        thumbnail_file,
        metadata_file,
        duration_seconds,
        chunks,
    };
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&result).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("无法保存媒体清单：{error}"))?;
    emit_progress(app, job_id, "ready", 100, "本地视频已准备完成");
    Ok(result)
}

pub fn load_media(app_data_dir: &Path, job_id: &str) -> Result<MediaPreparationResult, String> {
    validate_job_id(job_id)?;
    let manifest_path = app_data_dir.join("tasks").join(job_id).join("media.json");
    let manifest =
        fs::read_to_string(manifest_path).map_err(|_| "该任务还没有可用的本地媒体".to_string())?;
    serde_json::from_str(&manifest).map_err(|error| format!("无法读取媒体清单：{error}"))
}

pub fn export_audio(
    _app: &AppHandle,
    tools: &MediaToolPaths,
    app_data_dir: &Path,
    job_id: &str,
    output_path: &Path,
) -> Result<(), String> {
    validate_job_id(job_id)?;
    let media = load_media(app_data_dir, job_id)?;
    let video_file = media.video_file.as_deref().unwrap_or(&media.source_file);
    if !Path::new(video_file).is_file() {
        return Err("没有找到该任务的本地视频，请先重新下载视频".to_string());
    }
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("无法创建音频导出目录：{error}"))?;
    }
    let output = media_command(&tools.ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(video_file)
        .args(["-vn", "-c:a", "aac", "-b:a", "128k"])
        .arg(output_path)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("无法启动音频导出：{error}"))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if detail.is_empty() {
            "音频导出失败".to_string()
        } else {
            format!("音频导出失败：{detail}")
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        extract_supported_url, format_duration, format_selector_for_quality, parse_percent,
        validate_job_id,
    };

    #[test]
    fn selects_expected_formats() {
        assert!(format_selector_for_quality("720p").contains("height<=720"));
        assert!(format_selector_for_quality("1080p").contains("height<=1080"));
        assert!(!format_selector_for_quality("best").contains("height<="));
    }

    #[test]
    fn extracts_supported_url_from_share_text() {
        let (url, platform) =
            extract_supported_url("复制打开哔哩哔哩 https://b23.tv/abc123，查看视频").unwrap();
        assert_eq!(url, "https://b23.tv/abc123");
        assert_eq!(platform, "bilibili");
    }

    #[test]
    fn normalizes_douyin_modal_url() {
        let (url, platform) = extract_supported_url(
            "https://www.douyin.com/jingxuan?modal_id=7679755115485613353",
        )
        .unwrap();
        assert_eq!(url, "https://www.douyin.com/video/7679755115485613353");
        assert_eq!(platform, "douyin");

        let (url, platform) = extract_supported_url(
            "分享 https://www.douyin.com/recommend?modal_id=7679755115485613353&source=feed 视频",
        )
        .unwrap();
        assert_eq!(url, "https://www.douyin.com/video/7679755115485613353");
        assert_eq!(platform, "douyin");
    }

    #[test]
    fn extracts_youtube_url_from_share_text() {
        let (url, platform) =
            extract_supported_url("Check out this video: https://www.youtube.com/watch?v=dQw4w9WgXcQ !").unwrap();
        assert_eq!(url, "https://www.youtube.com/watch?v=dQw4w9WgXcQ");
        assert_eq!(platform, "youtube");

        let (url, platform) =
            extract_supported_url("TikTok分享：来看这个 https://www.tiktok.com/@user/video/1234567 真的好笑").unwrap();
        assert_eq!(url, "https://www.tiktok.com/@user/video/1234567");
        assert_eq!(platform, "tiktok");

        let (url, platform) =
            extract_supported_url("Twitter推文 https://x.com/username/status/9876543210 很有意思").unwrap();
        assert_eq!(url, "https://x.com/username/status/9876543210");
        assert_eq!(platform, "twitter");

        let (url, platform) =
            extract_supported_url("快手视频：【我的作品】 https://www.kuaishou.com/short-video/3x7890 看看吧").unwrap();
        assert_eq!(url, "https://www.kuaishou.com/short-video/3x7890");
        assert_eq!(platform, "kuaishou");

        let (url, platform) =
            extract_supported_url("小红书： http://xhslink.com/a/bCdEfG 复制打开").unwrap();
        assert_eq!(url, "http://xhslink.com/a/bCdEfG");
        assert_eq!(platform, "xiaohongshu");

        let (url, platform) =
            extract_supported_url("通用视频： https://example.com/video/lecture1.mp4 课件视频").unwrap();
        assert_eq!(url, "https://example.com/video/lecture1.mp4");
        assert_eq!(platform, "video");
    }

    #[test]
    fn rejects_local_and_invalid_urls() {
        assert!(extract_supported_url("http://127.0.0.1/video").is_err());
        assert!(extract_supported_url("http://localhost/video").is_err());
        assert!(extract_supported_url("file:///C:/secret.txt").is_err());
        assert!(extract_supported_url("not a url at all").is_err());
    }

    #[test]
    fn formats_short_and_long_durations() {
        assert_eq!(format_duration(82.0), "01:22");
        assert_eq!(format_duration(3_661.0), "01:01:01");
    }

    #[test]
    fn parses_machine_readable_progress() {
        assert_eq!(parse_percent("download: 42.7%"), Some(43));
        assert_eq!(parse_percent("unrelated"), None);
    }

    #[test]
    fn validates_task_directory_ids() {
        assert!(validate_job_id("job-123_abc").is_ok());
        assert!(validate_job_id("../other-task").is_err());
        assert!(validate_job_id("任务一").is_err());
    }
}

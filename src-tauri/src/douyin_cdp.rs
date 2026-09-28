use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio_tungstenite::{connect_async, tungstenite::Message};

#[derive(Debug, Deserialize)]
struct CookieItem {
    name: String,
    value: String,
    domain: String,
    path: Option<String>,
    secure: Option<bool>,
    expires: Option<f64>,
}

pub fn find_browser_executable() -> Option<PathBuf> {
    if let Ok(custom) = std::env::var("DOUYIN_BROWSER_PATH") {
        let p = PathBuf::from(custom);
        if p.is_file() {
            return Some(p);
        }
    }

    let candidates = [
        r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
    ];

    for c in candidates {
        let p = PathBuf::from(c);
        if p.is_file() {
            return Some(p);
        }
    }

    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        let edge = PathBuf::from(&local).join(r"Microsoft\Edge\Application\msedge.exe");
        if edge.is_file() {
            return Some(edge);
        }
        let chrome = PathBuf::from(&local).join(r"Google\Chrome\Application\chrome.exe");
        if chrome.is_file() {
            return Some(chrome);
        }
    }

    crate::media::find_on_path("msedge.exe").or_else(|| crate::media::find_on_path("chrome.exe"))
}

async fn wait_for_devtools_port(user_dir: &Path, max_wait: Duration) -> Result<u16, String> {
    let port_file = user_dir.join("DevToolsActivePort");
    let start = Instant::now();
    while start.elapsed() < max_wait {
        if let Ok(content) = tokio::fs::read_to_string(&port_file).await {
            let lines: Vec<&str> = content.lines().collect();
            if !lines.is_empty() {
                if let Ok(port) = lines[0].trim().parse::<u16>() {
                    return Ok(port);
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err("启动无头浏览器超时：未找到 DevToolsActivePort".to_string())
}

async fn get_page_ws_url(port: u16) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client
        .get(format!("http://127.0.0.1:{port}/json/list"))
        .send()
        .await
        .map_err(|e| format!("请求 DevTools 目标列表失败：{e}"))?;

    let targets: Vec<serde_json::Value> = resp.json().await.map_err(|e| e.to_string())?;
    for target in targets {
        let is_page = target.get("type").and_then(|t| t.as_str()) == Some("page");
        if is_page {
            if let Some(ws_url) = target.get("webSocketDebuggerUrl").and_then(|u| u.as_str()) {
                return Ok(ws_url.to_string());
            }
        }
    }
    Err("未在无头浏览器中找到页面调试通道".to_string())
}

async fn send_cdp_command<S>(
    ws: &mut S,
    id: i64,
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, String>
where
    S: SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error>
        + StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
        + Unpin,
{
    let req = serde_json::json!({
        "id": id,
        "method": method,
        "params": params,
    });
    ws.send(Message::Text(req.to_string()))
        .await
        .map_err(|e| format!("发送 CDP 命令失败 ({method}): {e}"))?;

    while let Some(msg) = ws.next().await {
        let msg = msg.map_err(|e| format!("读取 CDP 响应失败: {e}"))?;
        if let Message::Text(txt) = msg {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&txt) {
                if v.get("id").and_then(|i| i.as_i64()) == Some(id) {
                    if let Some(err) = v.get("error") {
                        let err_msg = err
                            .get("message")
                            .and_then(|m| m.as_str())
                            .unwrap_or("未知错误");
                        return Err(format!("CDP 命令执行失败 ({method}): {err_msg}"));
                    }
                    return Ok(v.get("result").cloned().unwrap_or(serde_json::json!({})));
                }
            }
        }
    }
    Err(format!("CDP 连接提前关闭，未收到响应 ({method})"))
}

async fn extract_cookies_via_cdp(ws_url: &str, target_url: &str) -> Result<Vec<CookieItem>, String> {
    let (mut ws_stream, _) = connect_async(ws_url)
        .await
        .map_err(|e| format!("连接 DevTools WebSocket 失败：{e}"))?;

    send_cdp_command(&mut ws_stream, 1, "Page.enable", serde_json::json!({})).await?;
    send_cdp_command(&mut ws_stream, 2, "Network.enable", serde_json::json!({})).await?;
    send_cdp_command(
        &mut ws_stream,
        3,
        "Network.setUserAgentOverride",
        serde_json::json!({
            "userAgent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36"
        }),
    )
    .await?;

    send_cdp_command(
        &mut ws_stream,
        4,
        "Page.navigate",
        serde_json::json!({ "url": target_url }),
    )
    .await?;

    // 等待 5 秒供抖音 JS 反爬与令牌初始化脚本执行完成
    tokio::time::sleep(Duration::from_secs(5)).await;

    let res = send_cdp_command(&mut ws_stream, 5, "Network.getAllCookies", serde_json::json!({})).await?;
    let cookies_val = res
        .get("cookies")
        .ok_or_else(|| "未能从浏览器提取到有效 Cookie".to_string())?;
    let cookies: Vec<CookieItem> =
        serde_json::from_value(cookies_val.clone()).map_err(|e| e.to_string())?;

    let _ = ws_stream.close(None).await;
    Ok(cookies)
}

pub async fn extract_douyin_cookies(source_url: &str, output_path: &Path) -> Result<(), String> {
    let browser_path = find_browser_executable().ok_or_else(|| {
        "未找到 Microsoft Edge 或 Google Chrome 浏览器，无法获取抖音反爬令牌".to_string()
    })?;

    let temp_root = std::env::temp_dir();
    let user_dir = tempfile::Builder::new()
        .prefix("douyin-cdp-")
        .tempdir_in(&temp_root)
        .map_err(|e| format!("创建无头浏览器临时目录失败：{e}"))?;
    let user_dir_path = user_dir.path().to_path_buf();

    let mut cmd = Command::new(&browser_path);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.args([
        "--headless=new",
        "--disable-gpu",
        "--no-sandbox",
        "--remote-debugging-port=0",
        &format!("--user-data-dir={}", user_dir_path.display()),
        "--no-first-run",
        "--no-default-browser-check",
        "about:blank",
    ]);
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::null());
    cmd.stderr(Stdio::null());

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("无法启动无头浏览器（{}）：{e}", browser_path.display()))?;
    let pid = child.id();

    let run_res = async {
        let port = wait_for_devtools_port(&user_dir_path, Duration::from_secs(12)).await?;
        let ws_url = get_page_ws_url(port).await?;
        let cookies = extract_cookies_via_cdp(&ws_url, source_url).await?;

        if cookies.is_empty() {
            return Err("未能从浏览器提取到有效 Cookie".to_string());
        }

        let mut lines = vec!["# Netscape HTTP Cookie File".to_string()];
        for c in &cookies {
            if c.name.is_empty() {
                continue;
            }
            let domain = if c.domain.starts_with('.') {
                c.domain.clone()
            } else {
                format!(".{}", c.domain)
            };
            let secure = if c.secure.unwrap_or(false) {
                "TRUE"
            } else {
                "FALSE"
            };
            let expires = c.expires.map(|e| if e > 0.0 { e as i64 } else { 0 }).unwrap_or(0);
            let path = c.path.as_deref().unwrap_or("/");
            lines.push(format!(
                "{}\tTRUE\t{}\t{}\t{}\t{}\t{}",
                domain, path, secure, expires, c.name, c.value
            ));
        }

        tokio::fs::write(output_path, lines.join("\n") + "\n")
            .await
            .map_err(|e| format!("保存 Cookie 文件失败：{e}"))?;

        Ok(())
    }
    .await;

    // 清理无头浏览器进程
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .output();
    }
    let _ = child.kill();
    let _ = user_dir.close();

    run_res
}

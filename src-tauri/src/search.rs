use reqwest::header::{HeaderMap, HeaderValue, REFERER, USER_AGENT};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

const SEARCH_PAGE_SIZE: usize = 10;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResultItem {
    pub id: String,
    pub title: String,
    pub author: String,
    pub platform: String,
    pub duration: String,
    pub cover_url: Option<String>,
    pub video_url: String,
    pub play_count: Option<String>,
    pub pub_date: Option<String>,
}

fn strip_html_tags(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut in_tag = false;
    for ch in input.chars() {
        if ch == '<' {
            in_tag = true;
        } else if ch == '>' {
            in_tag = false;
        } else if !in_tag {
            result.push(ch);
        }
    }
    result
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .trim()
        .to_string()
}

fn format_pub_date(ts: i64) -> String {
    if ts <= 0 {
        return String::new();
    }
    let total_secs = ts + 8 * 3600; // UTC+8 北京时间
    let days = total_secs / 86400;

    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!("{:04}-{:02}-{:02}", y, m, d)
}

fn format_play_count(raw: i64) -> String {
    if raw >= 100_000_000 {
        format!("{:.1}亿", raw as f64 / 100_000_000.0)
    } else if raw >= 10_000 {
        format!("{:.1}万", raw as f64 / 10_000.0)
    } else {
        raw.to_string()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResultResponse {
    pub items: Vec<SearchResultItem>,
    pub total_pages: usize,
    pub total_count: usize,
    pub page: usize,
}

pub async fn search_bilibili(
    keyword: &str,
    order: Option<&str>,
    duration: Option<usize>,
    page: usize,
) -> Result<SearchResultResponse, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .build()
        .map_err(|e| format!("构建 HTTP 客户端失败：{e}"))?;

    let mut headers = HeaderMap::new();
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36",
        ),
    );
    headers.insert(
        REFERER,
        HeaderValue::from_static("https://search.bilibili.com/video?keyword=%E6%B5%8B%E8%AF%95"),
    );
    headers.insert(
        reqwest::header::COOKIE,
        HeaderValue::from_static("buvid3=56A76318-8687-3E3B-9A4B-3DE822EE10C215579infoc; b_nut=1700000000; b_lsid=12345_12345;"),
    );

    let encoded_keyword = url::form_urlencoded::byte_serialize(keyword.as_bytes()).collect::<String>();
    let order_str = order.unwrap_or("totalrank");
    let dur_str = duration.unwrap_or(0);
    let target_page = page.max(1);

    let url = format!(
        "https://api.bilibili.com/x/web-interface/search/type?search_type=video&keyword={}&order={}&duration={}&page={}&page_size={}",
        encoded_keyword,
        order_str,
        dur_str,
        target_page,
        SEARCH_PAGE_SIZE
    );

    let resp = client
        .get(&url)
        .headers(headers)
        .send()
        .await
        .map_err(|e| format!("请求哔哩哔哩搜索失败：{e}"))?;

    if !resp.status().is_success() {
        return Err(format!("哔哩哔哩搜索响应异常：HTTP {}", resp.status()));
    }

    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("解析哔哩哔哩搜索结果 JSON 失败：{e}"))?;

    let code = body["code"].as_i64().unwrap_or(0);
    if code != 0 {
        let msg = body["message"].as_str().unwrap_or("请求受限");
        return Err(format!("哔哩哔哩搜索提示：{msg} (错误码 {code})"));
    }

    let total_count = body["data"]["numResults"].as_i64().unwrap_or(0) as usize;
    let total_pages = total_count.div_ceil(SEARCH_PAGE_SIZE).max(1);

    let mut items = Vec::new();
    if let Some(result_list) = body["data"]["result"].as_array() {
        for entry in result_list.iter().take(SEARCH_PAGE_SIZE) {
            let bvid = entry["bvid"].as_str().unwrap_or("").trim();
            if bvid.is_empty() {
                continue;
            }

            let raw_title = entry["title"].as_str().unwrap_or("未命名视频");
            let title = strip_html_tags(raw_title);

            let raw_author = entry["author"].as_str().unwrap_or("UP主");
            let author = strip_html_tags(raw_author);

            let duration_str = entry["duration"].as_str().unwrap_or("--:--").to_string();

            let mut pic = entry["pic"].as_str().unwrap_or("").to_string();
            if pic.starts_with("//") {
                pic = format!("https:{pic}");
            }
            let cover_url = if pic.is_empty() { None } else { Some(pic) };

            let video_url = format!("https://www.bilibili.com/video/{bvid}");

            let play_count = entry["play"]
                .as_i64()
                .map(format_play_count)
                .or_else(|| entry["play"].as_str().map(|s| s.to_string()));

            let pub_date = entry["pubdate"]
                .as_i64()
                .map(format_pub_date)
                .filter(|s| !s.is_empty());

            items.push(SearchResultItem {
                id: bvid.to_string(),
                title,
                author,
                platform: "bilibili".to_string(),
                duration: duration_str,
                cover_url,
                video_url,
                play_count,
                pub_date,
            });
        }
    }

    Ok(SearchResultResponse {
        items,
        total_pages,
        total_count,
        page: target_page,
    })
}

pub async fn search_videos(
    _app: &tauri::AppHandle,
    keyword: String,
    order: Option<String>,
    duration: Option<usize>,
    page: Option<usize>,
) -> Result<SearchResultResponse, String> {
    let kw = keyword.trim();
    if kw.is_empty() {
        return Ok(SearchResultResponse {
            items: Vec::new(),
            total_pages: 1,
            total_count: 0,
            page: 1,
        });
    }

    let p = page.unwrap_or(1);
    search_bilibili(kw, order.as_deref(), duration, p).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_date_formatting() {
        // 1708234800 is 2024-02-18 UTC
        let d = format_pub_date(1708234800);
        assert_eq!(d, "2024-02-18");
    }

    #[tokio::test]
    async fn test_search_bilibili_live() {
        let res = search_bilibili("深度学习", Some("click"), Some(0), 1).await;
        println!("Search Bilibili 深度学习: {:?}", res);
        assert!(res.is_ok());
        let resp = res.unwrap();
        assert!(!resp.items.is_empty());
        assert!(resp.total_pages > 0);
        println!("First item pub_date: {:?}", resp.items[0].pub_date);
        assert!(resp.items[0].pub_date.is_some());
    }
}

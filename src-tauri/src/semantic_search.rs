//! Local transcript retrieval for remembered-content navigation.
//!
//! Indexes bilingual transcript text with SQLite FTS5 for exact keyword BM25 retrieval
//! and real high-precision dense neural embeddings (via `embedding.rs`) on CPU for deep
//! semantic search, with graceful fallback to hashed vectors.

use crate::asr;
use crate::embedding;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::Path;

const MAX_RESULTS: usize = 5;
const VECTOR_TOP_K: usize = 40;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticSearchResult {
    pub chunk_id: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub segment_ids: Vec<String>,
    pub snippet: String,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticSearchResponse {
    pub query: String,
    pub results: Vec<SemanticSearchResult>,
    pub indexed_segments: usize,
    pub vector_mode: String,
}

pub fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "CREATE VIRTUAL TABLE IF NOT EXISTS transcript_search USING fts5(
            video_id UNINDEXED,
            chunk_id UNINDEXED,
            content,
            tokenize='unicode61 remove_diacritics 0'
        );
        CREATE TABLE IF NOT EXISTS transcript_search_meta(
            chunk_id TEXT PRIMARY KEY,
            video_id TEXT NOT NULL,
            start_ms INTEGER NOT NULL,
            end_ms INTEGER NOT NULL,
            segment_ids TEXT NOT NULL,
            snippet TEXT NOT NULL,
            vector BLOB NOT NULL
        );
        CREATE TABLE IF NOT EXISTS transcript_search_state(
            video_id TEXT PRIMARY KEY,
            transcript_hash TEXT NOT NULL,
            indexed_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        CREATE INDEX IF NOT EXISTS transcript_search_meta_video ON transcript_search_meta(video_id);
        CREATE INDEX IF NOT EXISTS transcript_search_state_hash ON transcript_search_state(transcript_hash);",
    )
}

pub fn search(
    db: &mut Connection,
    model_data_dir: &Path,
    task_root: &Path,
    video_id: &str,
    query: &str,
) -> Result<SemanticSearchResponse, String> {
    let query = query.trim();
    let embedding_status = embedding::model_status(model_data_dir);
    let vector_mode = if embedding_status.installed {
        "local-embedding"
    } else {
        "local-hash"
    };

    if query.is_empty() {
        return Ok(SemanticSearchResponse {
            query: String::new(),
            results: Vec::new(),
            indexed_segments: 0,
            vector_mode: vector_mode.into(),
        });
    }

    println!("[semantic_search] ==================== 收到搜索请求 ====================");
    println!("[semantic_search] 搜索词: \"{query}\", 视频 ID: \"{video_id}\"");
    println!("[semantic_search] 向量引擎模式: installed={}, vector_mode={}", embedding_status.installed, vector_mode);

    initialize(db).map_err(|error| format!("初始化字幕搜索索引失败：{error}"))?;
    let transcript = asr::load_transcript(task_root, video_id)?;
    let hash = transcript_hash(&transcript, embedding_status.installed);
    let indexed_hash: Option<String> = db
        .query_row(
            "SELECT transcript_hash FROM transcript_search_state WHERE video_id=?1",
            [video_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;

    if indexed_hash.as_deref() != Some(hash.as_str()) {
        println!("[semantic_search] 索引哈希不一致 (已存: {:?}, 目标: {:?})，重新建立索引...", indexed_hash, hash);
        index_transcript(db, model_data_dir, video_id, &transcript, &hash, embedding_status.installed)?;
    }

    let tokens = tokenize(query);
    if tokens.is_empty() {
        return Ok(SemanticSearchResponse {
            query: query.into(),
            results: Vec::new(),
            indexed_segments: transcript.segments.len(),
            vector_mode: vector_mode.into(),
        });
    }

    // 1. Vector Search (Neural dense embedding or hashed fallback)
    let query_vector = if embedding_status.installed {
        embedding::embed_query(model_data_dir, query)
            .unwrap_or_else(|e| {
                eprintln!("[semantic_search] Qwen3 提取查询向量失败: {e}，回退至哈希");
                hashed_vector(&tokens)
            })
    } else {
        hashed_vector(&tokens)
    };

    let load_vectors = |db_conn: &Connection| -> Result<Vec<(String, Vec<f32>)>, String> {
        let mut statement = db_conn
            .prepare("SELECT chunk_id, vector FROM transcript_search_meta WHERE video_id=?1")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([video_id], |row| {
                let id: String = row.get(0)?;
                let bytes: Vec<u8> = row.get(1)?;
                Ok((id, decode_vector(&bytes)))
            })
            .map_err(|error| error.to_string())?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row.map_err(|error| error.to_string())?);
        }
        Ok(result)
    };

    let mut stored_vectors = load_vectors(db)?;
    // 如果已有向量维度与当前查询向量维度不一致（例如旧版本 384 维，当前 Qwen3 1024 维），自动触发全量重建
    let dim_mismatch = stored_vectors.iter().any(|(_, v)| !v.is_empty() && v.len() != query_vector.len());
    if dim_mismatch {
        println!("[semantic_search] 检测到数据库旧向量维度不匹配，正在自动使用 Qwen3 重新生成 1024 维索引...");
        index_transcript(db, model_data_dir, video_id, &transcript, &hash, embedding_status.installed)?;
        stored_vectors = load_vectors(db)?;
    }

    let mut vector_hits = Vec::new();
    for (id, vector) in stored_vectors {
        let score = cosine(&query_vector, &vector);
        if score > 0.0 {
            vector_hits.push((id, score));
        }
    }
    vector_hits.sort_by(|a, b| b.1.total_cmp(&a.1));
    vector_hits.truncate(VECTOR_TOP_K);

    println!("[semantic_search] --- 1. Qwen3 语义向量匹配结果 (候选召回 {} 条): ---", vector_hits.len());
    for (i, (id, score)) in vector_hits.iter().take(5).enumerate() {
        println!("  [向量Top #{}] ID: {}, 相似度余弦分: {:.4}", i + 1, id, score);
    }

    // 2. FTS5 BM25 Lexical Search
    let match_query = tokens
        .iter()
        .map(|token| format!("\"{}\"", token.replace('"', "")))
        .collect::<Vec<_>>()
        .join(" OR ");

    let mut bm25_hits = Vec::new();
    let mut statement = db
        .prepare("SELECT chunk_id, bm25(transcript_search) FROM transcript_search WHERE video_id=?1 AND transcript_search MATCH ?2 ORDER BY bm25(transcript_search) LIMIT 40")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![video_id, match_query], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
        })
        .map_err(|error| error.to_string())?;

    for row in rows {
        bm25_hits.push(row.map_err(|error| error.to_string())?);
    }

    println!("[semantic_search] --- 2. FTS5 BM25 字面匹配结果 (命中 {} 条): ---", bm25_hits.len());
    for (i, (id, score)) in bm25_hits.iter().take(5).enumerate() {
        println!("  [字面Top #{}] ID: {}, BM25 分数: {:.4}", i + 1, id, score);
    }

    // 3. Reciprocal Rank Fusion (RRF)
    let mut fused: HashMap<String, f64> = HashMap::new();
    for (rank, (id, _)) in bm25_hits.iter().enumerate() {
        *fused.entry(id.clone()).or_default() += 1.0 / (60.0 + rank as f64 + 1.0);
    }
    for (rank, (id, _)) in vector_hits.iter().enumerate() {
        *fused.entry(id.clone()).or_default() += 1.0 / (60.0 + rank as f64 + 1.0);
    }

    let mut ranked = fused.into_iter().collect::<Vec<_>>();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
    ranked.truncate(MAX_RESULTS);

    let mut results = Vec::with_capacity(ranked.len());
    for (chunk_id, score) in ranked {
        let result = db
            .query_row(
                "SELECT start_ms,end_ms,segment_ids,snippet FROM transcript_search_meta WHERE chunk_id=?1",
                [chunk_id.as_str()],
                |row| {
                    let ids: String = row.get(2)?;
                    Ok(SemanticSearchResult {
                        chunk_id: chunk_id.clone(),
                        start_ms: row.get(0)?,
                        end_ms: row.get(1)?,
                        segment_ids: serde_json::from_str(&ids).unwrap_or_default(),
                        snippet: row.get(3)?,
                        score,
                    })
                },
            )
            .map_err(|error| error.to_string())?;
        results.push(result);
    }

    println!("[semantic_search] --- 3. 最终融合 (RRF) 排序输出 (Top {} 条): ---", results.len());
    for (i, res) in results.iter().enumerate() {
        println!("  [结果 #{}] [{:.1}s - {:.1}s] RRF得分: {:.4} | 内容: \"{}\"",
            i + 1, res.start_ms as f64 / 1000.0, res.end_ms as f64 / 1000.0, res.score, res.snippet.chars().take(40).collect::<String>());
    }
    println!("[semantic_search] ========================================================");

    Ok(SemanticSearchResponse {
        query: query.into(),
        results,
        indexed_segments: transcript.segments.len(),
        vector_mode: vector_mode.into(),
    })
}

pub fn clear_video(db: &Connection, video_id: &str) -> Result<(), String> {
    initialize(db).map_err(|error| error.to_string())?;
    db.execute(
        "DELETE FROM transcript_search WHERE video_id=?1",
        [video_id],
    )
    .map_err(|error| error.to_string())?;
    db.execute(
        "DELETE FROM transcript_search_meta WHERE video_id=?1",
        [video_id],
    )
    .map_err(|error| error.to_string())?;
    db.execute(
        "DELETE FROM transcript_search_state WHERE video_id=?1",
        [video_id],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn index_transcript(
    db: &mut Connection,
    model_data_dir: &Path,
    video_id: &str,
    transcript: &asr::TranscriptResult,
    hash: &str,
    embedding_installed: bool,
) -> Result<(), String> {
    let chunks = make_chunks(transcript);
    let chunk_texts = chunks.iter().map(|c| c.text.clone()).collect::<Vec<_>>();
    println!("[semantic_search] >>> 正在为视频 \"{video_id}\" 生成语义索引 (共 {} 个切块, 安装状态: {})...", chunk_texts.len(), embedding_installed);

    let vectors = if embedding_installed {
        match embedding::embed_texts(model_data_dir, &chunk_texts) {
            Ok(v) => v,
            Err(err) => {
                eprintln!("[semantic_search] 向量模型提取失败，回退到哈希向量: {err}");
                chunks.iter().map(|c| hashed_vector(&tokenize(&c.text))).collect()
            }
        }
    } else {
        chunks.iter().map(|c| hashed_vector(&tokenize(&c.text))).collect()
    };

    let tx = db.transaction().map_err(|error| error.to_string())?;
    tx.execute("DELETE FROM transcript_search WHERE video_id=?1", [video_id])
        .map_err(|error| error.to_string())?;
    tx.execute(
        "DELETE FROM transcript_search_meta WHERE video_id=?1",
        [video_id],
    )
    .map_err(|error| error.to_string())?;

    for (index, chunk) in chunks.into_iter().enumerate() {
        let chunk_id = format!("{video_id}:{index}");
        let tokenized = tokenize(&chunk.text).join(" ");
        let vector = vectors.get(index).cloned().unwrap_or_else(|| hashed_vector(&tokenize(&chunk.text)));
        let ids = serde_json::to_string(&chunk.segment_ids).map_err(|error| error.to_string())?;
        tx.execute(
            "INSERT INTO transcript_search(video_id,chunk_id,content) VALUES(?1,?2,?3)",
            params![video_id, chunk_id, tokenized],
        )
        .map_err(|error| error.to_string())?;
        tx.execute(
            "INSERT INTO transcript_search_meta(chunk_id,video_id,start_ms,end_ms,segment_ids,snippet,vector) VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![chunk_id, video_id, chunk.start_ms, chunk.end_ms, ids, chunk.text, encode_vector(&vector)],
        )
        .map_err(|error| error.to_string())?;
    }
    tx.execute(
        "INSERT INTO transcript_search_state(video_id,transcript_hash) VALUES(?1,?2) ON CONFLICT(video_id) DO UPDATE SET transcript_hash=excluded.transcript_hash,indexed_at=CURRENT_TIMESTAMP",
        params![video_id, hash],
    )
    .map_err(|error| error.to_string())?;
    tx.commit().map_err(|error| error.to_string())
}

#[derive(Debug)]
struct Chunk {
    start_ms: u64,
    end_ms: u64,
    segment_ids: Vec<String>,
    text: String,
}

fn make_chunks(transcript: &asr::TranscriptResult) -> Vec<Chunk> {
    let segments = &transcript.segments;
    let mut chunks = Vec::new();
    let mut start = 0usize;
    while start < segments.len() {
        let mut end = start;
        let mut chars = 0usize;
        while end < segments.len() && (end == start || (end - start < 5 && chars < 420)) {
            chars += segments[end].text.chars().count();
            if let Some(tr) = &segments[end].translated_text {
                chars += tr.chars().count();
            }
            end += 1;
        }
        let selected = &segments[start..end];
        if !selected.is_empty() {
            let snippet = selected
                .iter()
                .map(|item| {
                    if let Some(trans) = &item.translated_text {
                        if !trans.trim().is_empty() && trans.trim() != item.text.trim() {
                            return format!("{} ({})", item.text.trim(), trans.trim());
                        }
                    }
                    item.text.trim().to_string()
                })
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join(" ");

            chunks.push(Chunk {
                start_ms: selected.first().map(|item| item.start_ms).unwrap_or_default(),
                end_ms: selected.last().map(|item| item.end_ms).unwrap_or_default(),
                segment_ids: selected.iter().map(|item| item.id.clone()).collect(),
                text: snippet,
            });
        }
        start = if end > start + 1 { end - 1 } else { end };
    }
    chunks
}

fn tokenize(text: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut latin = String::new();
    let flush = |latin: &mut String, result: &mut Vec<String>| {
        if !latin.is_empty() {
            result.push(latin.to_lowercase());
            latin.clear();
        }
    };
    for character in text.chars() {
        if character.is_ascii_alphanumeric() || character == '_' {
            latin.push(character);
        } else {
            flush(&mut latin, &mut result);
            if !character.is_whitespace() && !character.is_ascii_punctuation() {
                result.push(character.to_string());
            }
        }
    }
    flush(&mut latin, &mut result);
    result
}

const HASH_VECTOR_DIM: usize = 256;

fn hashed_vector(tokens: &[String]) -> Vec<f32> {
    let mut vector = vec![0.0f32; HASH_VECTOR_DIM];
    for token in tokens {
        let mut hash = 2_166_136_261u32;
        for byte in token.as_bytes() {
            hash ^= u32::from(*byte);
            hash = hash.wrapping_mul(16_777_619);
        }
        let index = (hash as usize) % HASH_VECTOR_DIM;
        vector[index] += 1.0;
    }
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in &mut vector {
            *value /= norm;
        }
    }
    vector
}

fn encode_vector(vector: &[f32]) -> Vec<u8> {
    vector.iter().flat_map(|value| value.to_le_bytes()).collect()
}

fn decode_vector(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

fn cosine(left: &[f32], right: &[f32]) -> f64 {
    if left.len() != right.len() || left.is_empty() {
        return 0.0;
    }
    left.iter()
        .zip(right)
        .map(|(a, b)| f64::from(*a) * f64::from(*b))
        .sum()
}

fn transcript_hash(transcript: &asr::TranscriptResult, embedding_installed: bool) -> String {
    let input = transcript
        .segments
        .iter()
        .map(|segment| (
            &segment.id,
            segment.start_ms,
            segment.end_ms,
            &segment.text,
            segment.translated_text.as_deref().unwrap_or(""),
        ))
        .collect::<Vec<_>>();
    let model_tag = if embedding_installed { "qwen3-v2" } else { "hash-v1" };
    let bytes = serde_json::to_vec(&(input, model_tag)).unwrap_or_default();
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asr::{TranscriptResult, TranscriptSegment};
    use tempfile::tempdir;

    #[test]
    fn tokenizes_chinese_and_latin_without_dropping_words() {
        assert_eq!(tokenize("缓存 cache 失效"), vec!["缓", "存", "cache", "失", "效"]);
    }

    #[test]
    fn vector_is_normalized() {
        let vector = hashed_vector(&tokenize("缓存缓存"));
        let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 0.0001);
    }

    #[test]
    fn indexes_and_retrieves_timestamped_chunks() {
        let root = tempdir().unwrap();
        let transcript = TranscriptResult {
            job_id: "video-1".into(),
            model_id: "test".into(),
            language: "zh".into(),
            translation_language: None,
            text: "第一段 缓存失效 第二段 业务降级".into(),
            segments: vec![
                TranscriptSegment {
                    id: "seg-1".into(),
                    chunk_index: 0,
                    start: 0.0,
                    end: 2.0,
                    start_ms: 0,
                    end_ms: 2000,
                    text: "第一段 缓存失效".into(),
                    translated_text: None,
                    avg_confidence: Some(0.9),
                },
                TranscriptSegment {
                    id: "seg-2".into(),
                    chunk_index: 0,
                    start: 2.0,
                    end: 4.0,
                    start_ms: 2000,
                    end_ms: 4000,
                    text: "第二段 业务降级".into(),
                    translated_text: None,
                    avg_confidence: Some(0.9),
                },
            ],
            pause_repairs: None,
        };
        asr::save_transcript(root.path(), "video-1", &transcript).unwrap();

        let mut db = Connection::open_in_memory().unwrap();
        let response = search(&mut db, root.path(), root.path(), "video-1", "缓存失效").unwrap();
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].start_ms, 0);
    }
}

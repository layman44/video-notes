use std::fs;
use std::path::Path;
use crate::transcript::model::RawTranscript;

/// Stable content fingerprint for an immutable Raw ASR revision.
///
/// `createdAt`, `rawRevisionId` and `rawContentHash` are intentionally ignored so
/// re-running the same ASR result does not create a false conflict just because
/// wall-clock metadata changed. This is an integrity fingerprint, not a
/// cryptographic security primitive.
pub fn raw_content_hash(raw: &RawTranscript) -> Result<String, String> {
    let mut value = serde_json::to_value(raw).map_err(|e| format!("序列化原始转录用于指纹计算失败：{e}"))?;
    if let Some(metadata) = value.get_mut("metadata").and_then(|v| v.as_object_mut()) {
        metadata.remove("createdAt");
        metadata.remove("rawRevisionId");
        metadata.remove("rawContentHash");
        // Kept only for source compatibility; Canonical pipeline version is not a Raw ASR fact.
        metadata.remove("pipelineVersion");
    }
    let stable = serde_json::to_vec(&value).map_err(|e| format!("编码原始转录指纹数据失败：{e}"))?;
    Ok(format!("fnv1a64:{:016x}", fnv1a64(&stable)))
}

pub fn raw_revision_id(raw: &RawTranscript) -> Result<String, String> {
    Ok(format!("raw-{}", raw_content_hash(raw)?.trim_start_matches("fnv1a64:")))
}

/// Writes Raw ASR once. Re-writing equivalent Raw content is a no-op; different
/// content is rejected so Canonical can never silently drift away from the
/// persisted Raw fact layer.
pub fn save_raw_transcript(dir: &Path, raw: &RawTranscript) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("创建任务目录失败：{e}"))?;
    let path = dir.join("raw_transcript.json");
    let json = serde_json::to_string_pretty(raw).map_err(|e| format!("序列化原始转录失败：{e}"))?;

    if path.is_file() {
        let existing = fs::read_to_string(&path).map_err(|e| format!("读取现有原始转录失败：{e}"))?;
        let existing_raw: RawTranscript = serde_json::from_str(&existing)
            .map_err(|e| format!("现有原始转录损坏：{e}"))?;

        // v2.0 integration wrote already-cleaned text into raw_transcript.json and had no
        // Raw revision identity. Preserve that file for diagnostics, but do not let it block
        // creation of a true immutable Raw revision after upgrading to v2.1.
        let legacy_pre_revision = (existing_raw.metadata.raw_revision_id.is_none()
            || existing_raw.metadata.raw_content_hash.is_none())
            && existing_raw.metadata.pipeline_version != crate::transcript::storage::version::CURRENT_PIPELINE_VERSION;
        if legacy_pre_revision {
            archive_legacy_raw(dir, &path, &existing_raw)?;
        } else {
            if raw_content_hash(&existing_raw)? == raw_content_hash(raw)? {
                return Ok(());
            }
            return Err(
                "raw_transcript.json 已存在且属于不同 Raw revision；为保证 Raw/Canonical 一致性，本次任务已停止。若确需重建，请显式清理任务数据或调用 save_raw_transcript_force"
                    .into(),
            );
        }
    }

    fs::write(&path, json).map_err(|e| format!("保存原始转录文件失败：{e}"))
}


fn archive_legacy_raw(dir: &Path, path: &Path, raw: &RawTranscript) -> Result<(), String> {
    let hash = raw_content_hash(raw)?.trim_start_matches("fnv1a64:").to_string();
    let archive = dir.join(format!("raw_transcript.legacy-{}.json", &hash[..hash.len().min(12)]));
    if archive.is_file() {
        fs::remove_file(path).map_err(|e| format!("清理已归档的旧 Raw 文件失败：{e}"))?;
    } else {
        fs::rename(path, &archive).map_err(|e| format!("归档旧版 Raw 文件失败：{e}"))?;
    }
    Ok(())
}


/// Persists an immutable Raw revision and updates `raw_transcript.json` as the
/// compatibility/current pointer. Previous non-legacy revisions are preserved under
/// `raw_revisions/<revision-id>.json`.
pub fn save_raw_revision(dir: &Path, raw: &RawTranscript) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("创建任务目录失败：{e}"))?;
    let revisions_dir = dir.join("raw_revisions");
    fs::create_dir_all(&revisions_dir).map_err(|e| format!("创建 Raw revision 目录失败：{e}"))?;

    let mut candidate = raw.clone();
    let hash = raw_content_hash(&candidate)?;
    let revision = raw_revision_id(&candidate)?;
    candidate.metadata.raw_content_hash = Some(hash.clone());
    candidate.metadata.raw_revision_id = Some(revision.clone());

    let revision_path = revisions_dir.join(format!("{revision}.json"));
    if revision_path.is_file() {
        let existing: RawTranscript = serde_json::from_str(
            &fs::read_to_string(&revision_path).map_err(|e| format!("读取 Raw revision 失败：{e}"))?
        ).map_err(|e| format!("解析 Raw revision 失败：{e}"))?;
        if raw_content_hash(&existing)? != hash {
            return Err(format!("Raw revision id 冲突：{revision}"));
        }
    } else {
        let json = serde_json::to_string_pretty(&candidate).map_err(|e| format!("序列化 Raw revision 失败：{e}"))?;
        fs::write(&revision_path, json).map_err(|e| format!("保存 Raw revision 失败：{e}"))?;
    }

    let current_path = dir.join("raw_transcript.json");
    if current_path.is_file() {
        let existing_text = fs::read_to_string(&current_path).map_err(|e| format!("读取当前 Raw 失败：{e}"))?;
        let existing: RawTranscript = serde_json::from_str(&existing_text).map_err(|e| format!("解析当前 Raw 失败：{e}"))?;
        let legacy_pre_revision = (existing.metadata.raw_revision_id.is_none()
            || existing.metadata.raw_content_hash.is_none())
            && existing.metadata.pipeline_version != crate::transcript::storage::version::CURRENT_PIPELINE_VERSION;
        if legacy_pre_revision {
            archive_legacy_raw(dir, &current_path, &existing)?;
        } else {
            let existing_hash = raw_content_hash(&existing)?;
            let existing_revision = existing.metadata.raw_revision_id.clone()
                .unwrap_or(raw_revision_id(&existing)?);
            let existing_revision_path = revisions_dir.join(format!("{existing_revision}.json"));
            if !existing_revision_path.is_file() {
                let mut normalized_existing = existing.clone();
                normalized_existing.metadata.raw_content_hash = Some(existing_hash.clone());
                normalized_existing.metadata.raw_revision_id = Some(existing_revision.clone());
                fs::write(
                    &existing_revision_path,
                    serde_json::to_string_pretty(&normalized_existing).map_err(|e| format!("序列化历史 Raw revision 失败：{e}"))?,
                ).map_err(|e| format!("保存历史 Raw revision 失败：{e}"))?;
            }
            if existing_hash == hash {
                // Ensure the compatibility pointer also contains revision metadata.
                if existing.metadata.raw_revision_id.as_deref() == Some(revision.as_str())
                    && existing.metadata.raw_content_hash.as_deref() == Some(hash.as_str())
                {
                    return Ok(());
                }
            }
        }
    }

    fs::write(
        &current_path,
        serde_json::to_string_pretty(&candidate).map_err(|e| format!("序列化当前 Raw revision 失败：{e}"))?,
    ).map_err(|e| format!("更新当前 Raw revision 指针失败：{e}"))
}

pub fn save_raw_transcript_force(dir: &Path, raw: &RawTranscript) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("创建任务目录失败：{e}"))?;
    let path = dir.join("raw_transcript.json");
    let json = serde_json::to_string_pretty(raw).map_err(|e| format!("序列化原始转录失败：{e}"))?;
    fs::write(&path, json).map_err(|e| format!("强制保存原始转录文件失败：{e}"))
}

pub fn load_raw_transcript(dir: &Path) -> Result<Option<RawTranscript>, String> {
    let path = dir.join("raw_transcript.json");
    if !path.is_file() { return Ok(None); }
    let data = fs::read_to_string(&path).map_err(|e| format!("读取原始转录文件失败：{e}"))?;
    let raw = serde_json::from_str(&data).map_err(|e| format!("反序列化原始转录失败：{e}"))?;
    Ok(Some(raw))
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

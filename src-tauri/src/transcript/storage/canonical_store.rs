use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use crate::transcript::model::CanonicalTranscript;
use crate::transcript::storage::version::CURRENT_PIPELINE_VERSION;
use crate::transcript::transform::TransformLog;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PipelineManifest {
    pipeline_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    raw_revision_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    raw_content_hash: Option<String>,
}

pub fn save_canonical_transcript(dir: &Path, canonical: &CanonicalTranscript, log: Option<&TransformLog>) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("创建任务目录失败：{e}"))?;
    let canonical_path = dir.join("canonical_transcript.json");
    let log_path = dir.join("transform_log.json");
    let manifest_path = dir.join("pipeline_manifest.json");

    // The manifest is the commit marker. Remove the old marker first and write it
    // only after Canonical + TransformLog are complete, so cache reuse never treats
    // a partially-written pipeline output as current.
    if manifest_path.is_file() {
        fs::remove_file(&manifest_path).map_err(|e| format!("清理旧管道提交标记失败：{e}"))?;
    }

    let json = serde_json::to_string_pretty(canonical).map_err(|e| format!("序列化规范转录失败：{e}"))?;
    fs::write(&canonical_path, json).map_err(|e| format!("保存规范转录文件失败：{e}"))?;

    if let Some(l) = log {
        let log_json = serde_json::to_string_pretty(l).map_err(|e| format!("序列化转换日志失败：{e}"))?;
        fs::write(&log_path, log_json).map_err(|e| format!("保存转换日志文件失败：{e}"))?;
    } else if log_path.is_file() {
        fs::remove_file(&log_path).map_err(|e| format!("清理旧转换日志失败：{e}"))?;
    }

    let manifest = PipelineManifest {
        pipeline_version: CURRENT_PIPELINE_VERSION.into(),
        raw_revision_id: canonical.metadata.raw_revision_id.clone(),
        raw_content_hash: canonical.metadata.raw_content_hash.clone(),
    };
    fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest).map_err(|e| format!("序列化管道元数据失败：{e}"))?,
    ).map_err(|e| format!("保存管道元数据失败：{e}"))?;
    Ok(())
}

pub fn load_canonical_transcript(dir: &Path) -> Result<Option<CanonicalTranscript>, String> {
    let path = dir.join("canonical_transcript.json");
    if !path.is_file() { return Ok(None); }
    let data = fs::read_to_string(&path).map_err(|e| format!("读取规范转录文件失败：{e}"))?;
    let canonical = serde_json::from_str(&data).map_err(|e| format!("反序列化规范转录失败：{e}"))?;
    Ok(Some(canonical))
}

pub fn load_transform_log(dir: &Path) -> Result<Option<TransformLog>, String> {
    let path = dir.join("transform_log.json");
    if !path.is_file() { return Ok(None); }
    let data = fs::read_to_string(&path).map_err(|e| format!("读取转换日志文件失败：{e}"))?;
    let log = serde_json::from_str(&data).map_err(|e| format!("反序列化转换日志失败：{e}"))?;
    Ok(Some(log))
}


pub fn pipeline_version(dir: &Path) -> Result<Option<String>, String> {
    let path = dir.join("pipeline_manifest.json");
    if !path.is_file() { return Ok(None); }
    let data = fs::read_to_string(&path).map_err(|e| format!("读取管道元数据失败：{e}"))?;
    let manifest: PipelineManifest = serde_json::from_str(&data)
        .map_err(|e| format!("反序列化管道元数据失败：{e}"))?;
    Ok(Some(manifest.pipeline_version))
}

pub fn pipeline_is_current(dir: &Path) -> Result<bool, String> {
    let manifest_path = dir.join("pipeline_manifest.json");
    if !manifest_path.is_file() { return Ok(false); }
    let data = fs::read_to_string(&manifest_path).map_err(|e| format!("读取管道元数据失败：{e}"))?;
    let manifest: PipelineManifest = serde_json::from_str(&data)
        .map_err(|e| format!("反序列化管道元数据失败：{e}"))?;
    if manifest.pipeline_version != CURRENT_PIPELINE_VERSION { return Ok(false); }

    let raw_path = dir.join("raw_transcript.json");
    let canonical_path = dir.join("canonical_transcript.json");
    let log_path = dir.join("transform_log.json");
    if !raw_path.is_file() || !canonical_path.is_file() || !log_path.is_file() { return Ok(false); }

    let raw: crate::transcript::model::RawTranscript = serde_json::from_str(
        &fs::read_to_string(&raw_path).map_err(|e| format!("读取 Raw 一致性检查失败：{e}"))?
    ).map_err(|e| format!("解析 Raw 一致性检查失败：{e}"))?;
    let canonical: CanonicalTranscript = serde_json::from_str(
        &fs::read_to_string(&canonical_path).map_err(|e| format!("读取 Canonical 一致性检查失败：{e}"))?
    ).map_err(|e| format!("解析 Canonical 一致性检查失败：{e}"))?;
    let log: TransformLog = serde_json::from_str(
        &fs::read_to_string(&log_path).map_err(|e| format!("读取 TransformLog 一致性检查失败：{e}"))?
    ).map_err(|e| format!("解析 TransformLog 一致性检查失败：{e}"))?;

    let computed_raw_hash = crate::transcript::storage::raw_store::raw_content_hash(&raw)?;
    Ok(
        manifest.raw_revision_id.is_some()
            && manifest.raw_content_hash.as_deref() == Some(computed_raw_hash.as_str())
            && manifest.raw_revision_id == raw.metadata.raw_revision_id
            && manifest.raw_content_hash == raw.metadata.raw_content_hash
            && canonical.metadata.raw_revision_id == raw.metadata.raw_revision_id
            && canonical.metadata.raw_content_hash == raw.metadata.raw_content_hash
            && log.raw_revision_id == raw.metadata.raw_revision_id
            && log.raw_content_hash == raw.metadata.raw_content_hash
    )
}

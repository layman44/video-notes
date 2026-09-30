import { Check, Download, FolderOpen, HardDrive, LoaderCircle, Trash2 } from "lucide-react";
import { useEffect, useState, useSyncExternalStore } from "react";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { modelDownloadStore, type ModelKind } from "../../lib/modelDownloadStore";
import { runtime } from "../../lib/runtime";
import { toast } from "../../lib/toast";
import type { AsrModelStatus, ModelReadiness, SummaryModelStatus, TranslationModelStatus } from "../../types";

interface ModelsPageProps {
  onStatusChange: (readiness: ModelReadiness) => void;
}

function formatBytes(value?: number) {
  if (!value) return "";
  if (value >= 1024 ** 3) return `${(value / 1024 ** 3).toFixed(2)} GiB`;
  return `${(value / 1024 ** 2).toFixed(1)} MiB`;
}

function ManagedModelRow({
  kind,
  model,
  description,
  downloading,
  disabled,
  progress,
  message,
  error,
  onDownload,
  onRemove,
}: {
  kind: ModelKind;
  model: AsrModelStatus | SummaryModelStatus | TranslationModelStatus | null;
  description: string;
  downloading: boolean;
  disabled: boolean;
  progress: number;
  message: string;
  error: string;
  onDownload: () => void;
  onRemove: () => void;
}) {
  const fallbackName =
      kind === "asr"
        ? "Fun-ASR-Nano (GGUF + VAD + CTC + 标点)"
        : kind === "moss"
          ? "MOSS-Transcribe-Diarize 0.9B q4（OpenASR）"
          : kind === "embedding"
            ? "Qwen3 Embedding 0.6B (通义千问语义大模型)"
            : "Qwen3.5 2B Q4_K_M (总结与翻译)";
  const fallbackSize =
      kind === "asr"
        ? "约 1.2 GiB"
        : kind === "moss"
          ? "约 860 MiB"
          : kind === "embedding"
            ? "约 595 MiB"
            : "约 1.19 GiB";
  return (
    <div className="model-row">
      <span className="model-icon"><HardDrive size={21} /></span>
      <div className="model-copy">
        <strong>{model?.name ?? fallbackName}</strong>
        <span>{description} · {model?.fileSize ? formatBytes(model.fileSize) : model?.sizeLabel ?? fallbackSize}</span>
        {downloading ? (
          <>
            <div className="model-progress-row">
              <div className="model-progress-track"><span style={{ width: `${progress}%` }} /></div>
              <strong className="model-progress-percent">{Math.round(Math.max(0, Math.min(100, progress)))}%</strong>
            </div>
            <span>{message || "正在准备下载……"}</span>
          </>
        ) : null}
        {error ? <span className="model-error" role="alert">{error}</span> : null}
      </div>
      {model?.installed ? (
        <div className="model-action-stack">
          <span className="installed-state"><Check size={14} />模型已安装</span>
        </div>
      ) : (
        <button className="primary-button compact-button" type="button" disabled={disabled || model === null || downloading} onClick={onDownload}>
          {downloading ? <LoaderCircle className="spin" size={15} /> : <Download size={15} />}
          {downloading ? "下载中" : model === null ? "检查中" : "下载"}
        </button>
      )}
      {model?.installed ? (
        <button className="icon-button" type="button" disabled={disabled} aria-label={`删除 ${model.name}`} onClick={onRemove}>
          <Trash2 size={17} />
        </button>
      ) : <span />}
    </div>
  );
}

export function ModelsPage({ onStatusChange }: ModelsPageProps) {
  const [deleteCandidate, setDeleteCandidate] = useState<{
    kind: ModelKind;
    name: string;
    sizeLabel: string;
  } | null>(null);

  const storeState = useSyncExternalStore(
    (listener) => modelDownloadStore.subscribe(listener),
    () => modelDownloadStore.getState(),
  );

  useEffect(() => {
    void modelDownloadStore.refresh(onStatusChange);
  }, [onStatusChange]);

  const download = (kind: ModelKind) => {
    console.log("[ModelsPage] 点击下载 kind:", kind);
    void modelDownloadStore.startDownload(kind, onStatusChange);
  };

  const getModelDetails = (kind: ModelKind) => {
    const model =
      kind === "asr"
        ? storeState.asrModel
        : kind === "moss"
          ? storeState.mossModel
          : kind === "translation"
            ? storeState.translationModel
            : kind === "embedding"
              ? storeState.embeddingModel
              : storeState.summaryModel;
    const name =
      model?.name ??
      (kind === "asr"
        ? "Fun-ASR-Nano"
        : kind === "moss"
          ? "MOSS-Transcribe-Diarize q4"
          : kind === "translation"
            ? "MiLMMT 46 1B (极速翻译)"
            : kind === "embedding"
              ? "Qwen3 Embedding 0.6B (通义千问语义大模型)"
              : "Qwen3.5 2B (结构化总结)");
    const sizeLabel = model?.fileSize
      ? formatBytes(model.fileSize)
      : (model?.sizeLabel ?? "本地占用");
    return { name, sizeLabel };
  };

  const remove = (kind: ModelKind) => {
    const { name, sizeLabel } = getModelDetails(kind);
    setDeleteCandidate({ kind, name, sizeLabel });
  };

  const confirmDelete = async () => {
    if (!deleteCandidate) return;
    const { kind, name, sizeLabel } = deleteCandidate;
    setDeleteCandidate(null);
    try {
      await modelDownloadStore.removeModel(kind, onStatusChange);
      toast.success(`已成功删除本地 ${name} 模型，释放 ${sizeLabel} 空间`);
    } catch (reason) {
      toast.error(`删除模型失败: ${reason instanceof Error ? reason.message : String(reason)}`);
    }
  };

  return (
    <section className="standard-page page-frame models-page">
      <header className="page-header">
        <div>
          <h1>模型</h1>
          <p>模型独立存储在本机，支持离线运行、断点续传与完整性校验。</p>
        </div>
        <button className="secondary-button" type="button" onClick={() => void runtime.openModelsDirectory()}>
          <FolderOpen size={17} />打开模型目录
        </button>
      </header>

      <div className="model-list">
        <ManagedModelRow
          kind="moss"
          model={storeState.mossModel}
          description="高精度多语言识别 · OpenASR 本地 CPU 推理"
          downloading={storeState.downloadingKind === "moss"}
          disabled={storeState.downloadingKind !== null && storeState.downloadingKind !== "moss"}
          progress={storeState.progress.moss}
          message={storeState.message.moss}
          error={storeState.error.moss}
          onDownload={() => download("moss")}
          onRemove={() => remove("moss")}
        />

        <ManagedModelRow
          kind="asr"
          model={storeState.asrModel}
          description="多语言自动识别 · 可疑片段由 Nano 带上下文复听"
          downloading={storeState.downloadingKind === "asr"}
          disabled={storeState.downloadingKind !== null && storeState.downloadingKind !== "asr"}
          progress={storeState.progress.asr}
          message={storeState.message.asr}
          error={storeState.error.asr}
          onDownload={() => download("asr")}
          onRemove={() => remove("asr")}
        />

        <ManagedModelRow
          kind="embedding"
          model={storeState.embeddingModel}
          description="字幕文本高精度语义向量抽取与智能定位"
          downloading={storeState.downloadingKind === "embedding"}
          disabled={storeState.downloadingKind !== null && storeState.downloadingKind !== "embedding"}
          progress={storeState.progress.embedding}
          message={storeState.message.embedding}
          error={storeState.error.embedding}
          onDownload={() => download("embedding")}
          onRemove={() => remove("embedding")}
        />

        <ManagedModelRow
          kind="summary"
          model={storeState.summaryModel}
          description="Markdown 结构化笔记提炼、章节归纳与外语字幕精准翻译"
          downloading={storeState.downloadingKind === "summary"}
          disabled={storeState.downloadingKind !== null && storeState.downloadingKind !== "summary"}
          progress={storeState.progress.summary}
          message={storeState.message.summary}
          error={storeState.error.summary}
          onDownload={() => download("summary")}
          onRemove={() => remove("summary")}
        />
      </div>

      {deleteCandidate ? (
        <ConfirmDialog
          title="删除本地模型"
          message="确定删除本地模型吗？磁盘空间将被释放，已有视频笔记与转录历史不受影响。"
          highlightText={deleteCandidate.name}
          confirmText="确定删除"
          isDanger
          onConfirm={() => void confirmDelete()}
          onCancel={() => setDeleteCandidate(null)}
        />
      ) : null}
    </section>
  );
}

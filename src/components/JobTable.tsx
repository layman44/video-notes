import {
  Download,
  Check,
  CircleEllipsis,
  FileAudio,
  FolderOpen,
  Pause,
  Play,
  RefreshCw,
  Trash2,
} from "lucide-react";
import { useEffect, useState } from "react";
import type { Job, JobStatus, Platform } from "../types";

export interface JobActionHandlers {
  onOpenDirectory: (job: Job) => Promise<void>;
  onRedownload: (job: Job) => Promise<void>;
  onRetranscribe: (job: Job) => Promise<void>;
  onExportAudio: (job: Job) => Promise<void>;
  onDelete: (job: Job) => Promise<void>;
}

interface JobTableProps extends JobActionHandlers {
  jobs: Job[];
  onOpen: (job: Job) => void;
}

const statusLabels: Record<JobStatus, string> = {
  completed: "已完成",
  transcribed: "转录完成",
  processing: "处理中",
  waiting: "音频就绪",
  paused: "已暂停",
  failed: "处理失败",
};

function PlatformMark({ platform }: { platform: Platform }) {
  return (
    <span className={`platform-mark platform-${platform}`} aria-hidden="true">
      {platform === "bilibili" ? "B" : "音"}
    </span>
  );
}

const phaseLabels: Record<string, string> = {
  media_download: "下载中",
  media_normalize: "音频处理中",
  recognition: "识别中",
  pause_alignment: "停顿对齐中",
  verification: "复听校验中",
  boundary_review: "边界检查中",
  word_alignment: "词级对齐中",
  standardization: "生成标准转录",
  semantic_segmentation: "语义分段复核",
  translation: "翻译中",
  summary: "生成笔记中",
};

function formatPhaseProgress(job: Job): string {
  if (job.phaseCompleted == null || job.phaseTotal == null || job.phaseTotal <= 0) return "";
  if (job.phaseUnit === "milliseconds") {
    const format = (milliseconds: number) => {
      const seconds = Math.floor(milliseconds / 1000);
      return `${String(Math.floor(seconds / 60)).padStart(2, "0")}:${String(seconds % 60).padStart(2, "0")}`;
    };
    return `${format(job.phaseCompleted)} / ${format(job.phaseTotal)}`;
  }
  return `${job.phaseCompleted} / ${job.phaseTotal}`;
}

function getProcessingStageInfo(job: Job): string {
  const label = phaseLabels[job.phase ?? ""] ?? "处理中";
  const progress = formatPhaseProgress(job);
  return progress ? `${label} ${progress}` : label;
}

function JobStatusCell({ job }: { job: Job }) {
  let content = statusLabels[job.status];
  if (job.status === "processing") {
    content = getProcessingStageInfo(job);
  }

  return (
    <div className={`job-status status-${job.status}`} title={job.statusMessage || content}>
      <span className="status-symbol" aria-hidden="true">
        {job.status === "completed" ? <Check size={14} strokeWidth={2.4} /> : null}
        {job.status === "transcribed" ? <Check size={14} strokeWidth={2.4} /> : null}
        {job.status === "waiting" ? <Check size={14} strokeWidth={2.4} /> : null}
        {job.status === "processing" ? <span className="progress-ring" /> : null}
        {job.status === "paused" ? <Pause size={12} fill="currentColor" /> : null}
        {job.status === "failed" ? "!" : null}
      </span>
      <span>{content}</span>
    </div>
  );
}

export function JobTable({
  jobs,
  onOpen,
  onOpenDirectory,
  onRedownload,
  onRetranscribe,
  onExportAudio,
  onDelete,
}: JobTableProps) {
  const [openMenuId, setOpenMenuId] = useState<string | null>(null);
  const [pendingAction, setPendingAction] = useState("");

  useEffect(() => {
    if (!openMenuId) return undefined;
    const closeMenu = (event: PointerEvent) => {
      const target = event.target;
      if (target instanceof Element && !target.closest(".more-actions-wrapper")) setOpenMenuId(null);
    };
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpenMenuId(null);
    };
    window.addEventListener("pointerdown", closeMenu);
    window.addEventListener("keydown", closeOnEscape);
    return () => {
      window.removeEventListener("pointerdown", closeMenu);
      window.removeEventListener("keydown", closeOnEscape);
    };
  }, [openMenuId]);

  const runAction = async (job: Job, name: string, action: (job: Job) => Promise<void>) => {
    const actionId = `${job.id}:${name}`;
    if (pendingAction) return;
    setPendingAction(actionId);
    setOpenMenuId(null);
    try {
      await action(job);
    } catch (reason) {
      window.alert(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setPendingAction("");
    }
  };

  return (
    <div className="job-table" role="table" aria-label="最近任务">
      <div className="job-table-header" role="row">
        <span role="columnheader">标题</span>
        <span role="columnheader">平台</span>
        <span role="columnheader">时长</span>
        <span role="columnheader">更新时间</span>
        <span role="columnheader">状态</span>
        <span className="action-heading" role="columnheader">操作</span>
      </div>
      <div className="job-table-body">
        {jobs.map((job) => (
          <div
            className="job-row"
            role="row"
            tabIndex={0}
            key={job.id}
            onClick={() => onOpen(job)}
            onKeyDown={(event) => {
              if (event.key === "Enter" || event.key === " ") {
                event.preventDefault();
                onOpen(job);
              }
            }}
          >
            <span className="job-title" role="cell">{job.title}</span>
            <span className="platform-cell" role="cell">
              <PlatformMark platform={job.platform} />
              {job.platform === "bilibili" ? "哔哩哔哩" : "抖音"}
            </span>
            <span role="cell">{job.duration}</span>
            <span role="cell">{job.updatedAt}</span>
            <span role="cell"><JobStatusCell job={job} /></span>
            <span className="row-actions" role="cell" onClick={(event) => event.stopPropagation()}>
              <button type="button" aria-label="打开任务" title="打开任务" onClick={() => onOpen(job)}>
                <Play size={17} />
              </button>
              <button
                type="button"
                aria-label="打开视频目录"
                title="打开视频目录"
                disabled={pendingAction === `${job.id}:folder`}
                onClick={() => void runAction(job, "folder", onOpenDirectory)}
              >
                <FolderOpen size={17} />
              </button>
              <span className="more-actions-wrapper">
                <button
                  type="button"
                  aria-label="更多操作"
                  title="更多操作"
                  aria-haspopup="menu"
                  aria-expanded={openMenuId === job.id}
                  onClick={() => setOpenMenuId((current) => current === job.id ? null : job.id)}
                >
                  <CircleEllipsis size={18} />
                </button>
                {openMenuId === job.id ? (
                  <span className="job-actions-menu" role="menu" aria-label={`${job.title}的更多操作`}>
                    <button type="button" role="menuitem" disabled={job.status === "processing" || Boolean(pendingAction)} onClick={() => void runAction(job, "redownload", onRedownload)}>
                      <Download size={15} />重新下载
                    </button>
                    <button type="button" role="menuitem" disabled={job.status === "processing" || (!(["waiting", "transcribed", "completed"].includes(job.status)) && (job.phase === "media_download" || job.phase === "media_normalize" || !job.phase)) || Boolean(pendingAction)} onClick={() => void runAction(job, "retranscribe", onRetranscribe)}>
                      <RefreshCw size={15} />重新转录
                    </button>
                    <button type="button" role="menuitem" disabled={(!(["waiting", "transcribed", "completed"].includes(job.status)) && (job.phase === "media_download" || job.phase === "media_normalize" || !job.phase)) || Boolean(pendingAction)} onClick={() => void runAction(job, "audio", onExportAudio)}>
                      <FileAudio size={15} />导出音频
                    </button>
                    <button className="is-danger" type="button" role="menuitem" disabled={job.status === "processing" || Boolean(pendingAction)} onClick={() => void runAction(job, "delete", onDelete)}>
                      <Trash2 size={15} />删除任务
                    </button>
                  </span>
                ) : null}
              </span>
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}

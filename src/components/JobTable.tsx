import {
  Download,
  CircleAlert,
  CircleCheck,
  CirclePause,
  ChevronLeft,
  ChevronRight,
  Clock3,
  CircleEllipsis,
  FileCheck2,
  FileAudio,
  FolderOpen,
  LoaderCircle,
  RefreshCw,
  SquareArrowOutUpRight,
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
  pagination?: boolean;
  pageSize?: number;
}

const statusLabels: Record<JobStatus, string> = {
  completed: "已完成",
  transcribed: "转录完成",
  processing: "处理中",
  waiting: "待处理",
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
  media_normalize: "音频提取",
  recognition: "转录识别",
  pause_alignment: "停顿校准",
  verification: "模型复听",
  boundary_review: "边界复核",
  word_alignment: "字级对齐",
  standardization: "文本规整",
  semantic_segmentation: "语义分段",
  translation: "中文字幕翻译",
  summary: "AI 总结生成",
};

function formatPhaseProgress(job: Job): string {
  if (job.phaseCompleted == null || job.phaseTotal == null || job.phaseTotal <= 0) return "";
  if (job.phaseUnit === "seconds" || job.phaseUnit === "milliseconds") {
    const percent = Math.min(100, Math.round((job.phaseCompleted / job.phaseTotal) * 100));
    return `${percent}%`;
  }
  if (job.phaseUnit === "percent") {
    const percent = Math.min(100, Math.round((job.phaseCompleted / job.phaseTotal) * 100));
    return `${percent}%`;
  }
  return `${job.phaseCompleted}/${job.phaseTotal}`;
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
  } else if (job.status === "paused") {
    if (job.phase === "media_download") {
      content = "下载已暂停";
    } else if (job.phase === "recognition" || job.phase === "pause_alignment" || job.phase === "verification" || job.phase === "standardization") {
      content = "转录已暂停";
    } else if (job.phase === "translation") {
      content = "翻译已暂停";
    } else if (job.phase === "summary") {
      content = "笔记生成已暂停";
    } else {
      content = "已暂停";
    }
  } else if (job.status === "failed") {
    if (job.phase === "media_download") {
      content = "下载失败";
    } else if (job.phase === "recognition") {
      content = "转写失败";
    } else if (job.phase === "translation") {
      content = "翻译失败";
    } else if (job.phase === "summary") {
      content = "笔记生成失败";
    } else {
      content = "处理失败";
    }
  }

  return (
    <div className={`job-status status-${job.status}`} title={job.statusMessage || content}>
      <span className="status-symbol" aria-hidden="true">
        {job.status === "completed" ? <CircleCheck size={14} strokeWidth={2.2} /> : null}
        {job.status === "transcribed" ? <FileCheck2 size={14} strokeWidth={2.2} /> : null}
        {job.status === "processing" ? <LoaderCircle className="spin" size={14} strokeWidth={2.2} /> : null}
        {job.status === "waiting" ? <Clock3 size={14} strokeWidth={2.2} /> : null}
        {job.status === "paused" ? <CirclePause size={14} strokeWidth={2.2} /> : null}
        {job.status === "failed" ? <CircleAlert size={14} strokeWidth={2.2} /> : null}
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
  pagination = false,
  pageSize = 10,
}: JobTableProps) {
  const [openMenuId, setOpenMenuId] = useState<string | null>(null);
  const [pendingAction, setPendingAction] = useState("");
  const [page, setPage] = useState(1);

  const totalPages = Math.max(1, Math.ceil(jobs.length / pageSize));
  const currentPage = Math.min(page, totalPages);

  useEffect(() => {
    if (page > totalPages) {
      setPage(totalPages);
    }
  }, [page, totalPages]);

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

  const displayJobs = pagination
    ? jobs.slice((currentPage - 1) * pageSize, currentPage * pageSize)
    : jobs;

  return (
    <div className="job-table-container">
      <div className="job-table" role="table" aria-label="任务列表">
        <div className="job-table-header" role="row">
          <span role="columnheader">标题</span>
          <span role="columnheader">平台</span>
          <span role="columnheader">时长</span>
          <span role="columnheader">更新时间</span>
          <span role="columnheader">状态</span>
          <span className="action-heading" role="columnheader">操作</span>
        </div>
        <div className="job-table-body">
          {displayJobs.length === 0 ? (
            <div className="job-table-empty">暂无任务记录</div>
          ) : (
            displayJobs.map((job) => (
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
                    <SquareArrowOutUpRight size={17} />
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
            ))
          )}
        </div>
      </div>
      {pagination && jobs.length > 0 ? (
        <div className="job-table-pagination">
          <div className="pagination-info">
            共 <strong>{jobs.length}</strong> 条任务 · 第 {currentPage} / {totalPages} 页
          </div>
          {totalPages > 1 ? (
            <div className="pagination-controls">
              <button
                type="button"
                className="pagination-nav-button"
                disabled={currentPage <= 1}
                onClick={() => setPage((p) => Math.max(1, p - 1))}
                aria-label="上一页"
              >
                <ChevronLeft size={15} />
                <span>上一页</span>
              </button>
              <div className="pagination-pages">
                {Array.from({ length: totalPages }, (_, i) => i + 1).map((p) => {
                  if (
                    totalPages <= 7 ||
                    p === 1 ||
                    p === totalPages ||
                    (p >= currentPage - 1 && p <= currentPage + 1)
                  ) {
                    return (
                      <button
                        type="button"
                        key={p}
                        className={`page-number-button ${p === currentPage ? "is-active" : ""}`}
                        onClick={() => setPage(p)}
                      >
                        {p}
                      </button>
                    );
                  }
                  if (p === 2 && currentPage > 3) {
                    return <span key="ellipsis-left" className="pagination-ellipsis">…</span>;
                  }
                  if (p === totalPages - 1 && currentPage < totalPages - 2) {
                    return <span key="ellipsis-right" className="pagination-ellipsis">…</span>;
                  }
                  return null;
                })}
              </div>
              <button
                type="button"
                className="pagination-nav-button"
                disabled={currentPage >= totalPages}
                onClick={() => setPage((p) => Math.min(totalPages, p + 1))}
                aria-label="下一页"
              >
                <span>下一页</span>
                <ChevronRight size={15} />
              </button>
            </div>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

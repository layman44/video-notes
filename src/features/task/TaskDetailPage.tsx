import {
  ArrowLeft,
  Check,
  ChevronLeft,
  Download,
  Loader2,
  Maximize2,
  FileText,
  Minimize2,
  Pause,
  Pencil,
  Play,
  RefreshCw,
  Search,
  Sparkles,
  Volume2,
  VolumeX,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import fallbackThumbnailUrl from "../../assets/rag-thumbnail.png";
import { completedSteps, noteMarkdown } from "../../data";
import {
  loadTranscriptDisplayMode,
  loadTranscriptViewMode,
  saveTranscriptDisplayMode,
  saveTranscriptViewMode,
  type TranscriptDisplayMode,
  type TranscriptViewMode,
} from "../../lib/preferences";
import { formatErrorMessage, runtime } from "../../lib/runtime";
import {
  isMossBackend,
  type AsrPhaseEvent,
  type AsrPipelinePhase,
  type AsrPhaseProgress,
  type AsrSnapshot,
  type Job,
  type MediaPreparationResult,
  type NoteResult,
  type ProcessingStep,
  TaskTab,
  TranscriptResult,
  TranscriptSegment,
  TranslationSegmentUpdate,
} from "../../types";

interface TaskDetailPageProps {
  job: Job;
  onBack: () => void;
  onNavigateToModels?: () => void;
  onComplete: (jobId: string) => void;
  onCancelMedia: () => Promise<boolean>;
  onResetJob?: () => Promise<void> | void;
  onRetryMedia: (options?: { resume?: boolean }) => void;
  onTranslate: () => void;
  onOrganize: () => void;
  onReorganize: () => void;
  onTranscriptEdited: () => void;
  autoPlayOnTranscriptClick: boolean;
  usesRealMediaPipeline: boolean;
  noteRevision: number;
}

const processingCopy = [
  "正在分析音频活动区间……",
  "正在转写 12:30–13:00 的语音……",
  "正在生成章节摘要与核心要点……",
];

type TranscriptUiPhase = "idle" | "recognizing" | "refining" | "translating" | "organizing" | "complete";

function resolveTranscriptUiPhase(
  job: Job,
  isComplete: boolean,
  isTranscribed: boolean,
  asrPhase: AsrPipelinePhase | null,
): TranscriptUiPhase {
  if (job.status === "processing") {
    if (job.phase === "translation") return "translating";
    if (job.phase === "summary") return "organizing";
    if (asrPhase === "recognition" || job.phase === "recognition") return "recognizing";
    if (asrPhase || ["pause_alignment", "verification", "boundary_review", "word_alignment", "standardization", "semantic_segmentation"].includes(job.phase ?? "")) {
      return "refining";
    }
  }
  if (isComplete || isTranscribed) return "complete";
  return "idle";
}

function transcriptPhaseDescription(
  phase: TranscriptUiPhase,
  statusMessage: string | undefined,
  hasTranslations: boolean,
) {
  const message = statusMessage?.trim();
  if (phase === "recognizing") return message ? `正在识别 · ${message}` : "正在实时识别语音……";
  if (phase === "refining") return message ? `正在校对 · ${message}` : "正在按时间轴生成并复核校正（标准）结果……";
  if (phase === "translating") return message ? `正在翻译 · ${message}` : "正在翻译，已完成部分会逐段出现……";
  if (phase === "organizing") return message ? `转录已定稿 · ${message}` : "转录已定稿，正在整理内容……";
  if (phase === "complete") return hasTranslations ? "转录与翻译已完成" : "转录已完成";
  return "按后端时间轴与视频同步";
}

function formatSegmentTime(ms: number): string {
  const totalSeconds = Math.floor(ms / 1000);
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;
  if (hours > 0) {
    return `${hours}:${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}`;
  }
  return `${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}`;
}

function buildSteps(progress: number): ProcessingStep[] {
  const boundaries = [18, 72, 100];
  const labels: ProcessingStep["label"][] = ["视频下载", "语音转写", "内容整理"];

  return labels.map((label, index) => {
    const previousBoundary = index === 0 ? 0 : boundaries[index - 1];
    const state = progress >= boundaries[index] ? "completed" : progress > previousBoundary ? "active" : "pending";
    return {
      id: (["download", "transcribe", "summarize"] as const)[index],
      label,
      detail: state === "completed" ? ["00:08", "02:11", "03:15"][index] : state === "active" ? `${progress}%` : "等待中",
      state,
    };
  });
}

const mediaReadySteps: ProcessingStep[] = [
  { id: "download", label: "视频保存与音频提取", detail: "已完成", state: "completed" },
  { id: "transcribe", label: "语音转写", detail: "等待模型", state: "pending" },
  { id: "summarize", label: "内容整理", detail: "等待中", state: "pending" },
];

const transcribedSteps: ProcessingStep[] = [
  { id: "download", label: "视频保存与音频提取", detail: "已完成", state: "completed" },
  { id: "transcribe", label: "本地语音转写", detail: "已完成", state: "completed" },
  { id: "summarize", label: "翻译 / 笔记", detail: "按需手动执行", state: "pending" },
];

const realCompletedSteps: ProcessingStep[] = [
  { id: "download", label: "视频保存与音频提取", detail: "已完成", state: "completed" },
  { id: "transcribe", label: "本地语音转写", detail: "已完成", state: "completed" },
  { id: "summarize", label: "Markdown 内容整理", detail: "已完成", state: "completed" },
];

function buildRealSteps(job: Job): ProcessingStep[] {
  const mediaActive = job.status === "processing" && (job.phase === "media_download" || job.phase === "media_normalize");
  const transcriptionActive = job.status === "processing" && [
    "recognition", "pause_alignment", "verification", "boundary_review", "word_alignment", "standardization", "semantic_segmentation",
  ].includes(job.phase ?? "");
  const mediaDone = !mediaActive && (job.status === "transcribed" || job.status === "completed" || transcriptionActive || job.phase === "translation" || job.phase === "summary");
  const transcriptDone = job.status === "transcribed" || job.status === "completed" || job.phase === "translation" || job.phase === "summary";
  const phaseDetail = job.phaseTotal && job.phaseCompleted != null
    ? `${job.phaseCompleted} / ${job.phaseTotal}`
    : (job.statusMessage || "处理中");
  return [
    { id: "download", label: "视频保存与音频提取", detail: mediaDone ? "已完成" : mediaActive ? phaseDetail : "等待中", state: mediaDone ? "completed" : mediaActive ? "active" : "pending" },
    { id: "transcribe", label: "本地语音转写", detail: transcriptDone ? "已完成" : transcriptionActive ? phaseDetail : "等待中", state: transcriptDone ? "completed" : transcriptionActive ? "active" : "pending" },
    { id: "summarize", label: "翻译 / 笔记", detail: job.phase === "translation" || job.phase === "summary" ? phaseDetail : job.status === "completed" ? "笔记已生成" : "按需手动执行", state: job.status === "completed" ? "completed" : job.phase === "translation" || job.phase === "summary" ? "active" : "pending" },
  ];
}

function ProcessingRail({
  steps,
  completed,
  transcribed,
  waiting,
  failed,
}: {
  steps: ProcessingStep[];
  completed: boolean;
  transcribed: boolean;
  waiting: boolean;
  failed: boolean;
}) {
  const summaryTitle = completed ? "处理完成" : transcribed ? "真实转录已完成" : waiting ? "等待处理" : failed ? "处理失败" : "本地处理中";
  const summaryDetail = completed ? "笔记已生成" : transcribed ? "可人工校正，并按需翻译或生成笔记" : waiting ? "等待开始下载与转录" : failed ? "可以重新尝试" : "保持窗口打开即可";
  return (
    <aside className="processing-rail" aria-label="处理记录">
      <div className="rail-heading">
        <h2>处理记录</h2>
        <ChevronLeft size={20} strokeWidth={1.8} aria-hidden="true" />
      </div>
      <ol className="step-list">
        {steps.map((step) => (
          <li className={`process-step is-${step.state}`} key={step.id}>
            <span className="step-marker" aria-hidden="true">
              {step.state === "completed" ? <Check size={13} strokeWidth={2.6} /> : null}
              {step.state === "active" ? <span className="step-pulse" /> : null}
            </span>
            <div>
              <strong>{step.label}</strong>
              <span>{step.detail}</span>
            </div>
          </li>
        ))}
      </ol>
      <div className={`completion-summary ${completed || transcribed || waiting ? "is-complete" : ""} ${failed ? "is-failed" : ""}`}>
        <span className="completion-icon" aria-hidden="true">
          {completed || transcribed || waiting ? <Check size={14} strokeWidth={2.5} /> : failed ? "!" : <span className="step-pulse" />}
        </span>
        <div>
          <strong>{summaryTitle}</strong>
          <span>{summaryDetail}</span>
        </div>
      </div>
    </aside>
  );
}

function NoteContent({ result, loading, error, isReal }: {
  result: NoteResult | null;
  loading: boolean;
  error: string;
  isReal: boolean;
}) {
  if (isReal) {
    return (
      <article className="note-document" aria-label="Markdown 笔记">
        {loading ? <p>正在读取本地 Markdown 笔记……</p> : null}
        {error ? <p className="model-error">{error}</p> : null}
        {result ? (
          <>
            <section>
              <h2>摘要</h2>
              <p>{result.summary}</p>
            </section>
            <hr />
            <section>
              <h2>核心要点</h2>
              <ul>{result.keyPoints.map((point) => <li key={point}>{point}</li>)}</ul>
            </section>
            <hr />
            <section>
              <h2>章节笔记</h2>
              <div className="chapter-notes">
                {result.chapters.map((chapter) => (
                  <section key={`${chapter.timestampMs}-${chapter.title}`}>
                    <h3><button type="button">[{formatTimestamp(chapter.timestampMs)}] {chapter.title}</button></h3>
                    <p>{chapter.content}</p>
                  </section>
                ))}
              </div>
            </section>
          </>
        ) : null}
      </article>
    );
  }
  return (
    <article className="note-document" aria-label="Markdown 笔记">
      <section>
        <h2>摘要</h2>
        <p>本视频从零开始讲解 RAG（Retrieval-Augmented Generation，检索增强生成）的工作原理。</p>
        <p>内容涵盖为什么需要 RAG、检索与生成如何配合、常见实现误区，以及一个最小可用的实现流程。</p>
        <p>通过具体示例帮助我们建立对 RAG 的系统性理解。</p>
      </section>
      <hr />
      <section>
        <h2>核心要点</h2>
        <ul>
          <li>RAG 用于缓解大模型的知识局限与幻觉问题，将外部知识检索引入生成流程。</li>
          <li>检索阶段负责找到与问题相关的高质量上下文，生成阶段基于上下文生成答案。</li>
          <li>检索质量、上下文选择和提示设计是影响效果的关键因素。</li>
          <li>评估应关注答案的正确性、可追溯性与相关性，而不仅是流畅度。</li>
        </ul>
      </section>
      <hr />
      <section>
        <h2>章节笔记</h2>
        <ul className="chapter-links">
          <li><button type="button">[00:00] 为什么需要 RAG</button></li>
          <li><button type="button">[06:42] 检索与生成如何配合</button></li>
          <li><button type="button">[18:15] 常见实现误区</button></li>
        </ul>
      </section>
    </article>
  );
}

function formatTimestamp(milliseconds: number) {
  const totalSeconds = Math.floor(milliseconds / 1000);
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;
  return hours > 0
    ? `${String(hours).padStart(2, "0")}:${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}`
    : `${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}`;
}

function durationToMilliseconds(duration: string) {
  const parts = duration.split(":").map(Number);
  if (parts.some((part) => !Number.isFinite(part))) return 0;
  if (parts.length === 3) return ((parts[0] * 60 * 60) + (parts[1] * 60) + parts[2]) * 1000;
  if (parts.length === 2) return ((parts[0] * 60) + parts[1]) * 1000;
  return 0;
}

function findActiveSegment(segments: TranscriptSegment[], currentTimeMs: number) {
  let low = 0;
  let high = segments.length - 1;
  let activeIndex = -1;
  while (low <= high) {
    const middle = Math.floor((low + high) / 2);
    if (segments[middle].startMs <= currentTimeMs) {
      activeIndex = middle;
      low = middle + 1;
    } else {
      high = middle - 1;
    }
  }
  return activeIndex >= 0 ? segments[activeIndex] : segments[0];
}

function buildDemoTranscript(jobId: string): TranscriptResult {
  const segments: TranscriptSegment[] = [
    { id: "demo-0", chunkIndex: 0, startMs: 0, endMs: 42_000, text: "Why do we need RAG? Large language models are powerful, but their knowledge can be outdated or incomplete.", translatedText: "为什么我们需要 RAG？大模型虽然很强大，但知识可能存在滞后或缺失。" },
    { id: "demo-1", chunkIndex: 0, startMs: 131_000, endMs: 184_000, text: "The RAG workflow can be divided into retrieval and generation stages.", translatedText: "RAG 的整体流程可以拆成检索和生成两个阶段。" },
    { id: "demo-2", chunkIndex: 0, startMs: 402_000, endMs: 459_000, text: "The retrieval stage finds the content most relevant to the question.", translatedText: "检索阶段会先找到与问题最相关的内容。" },
    { id: "demo-3", chunkIndex: 0, startMs: 624_000, endMs: 688_000, text: "Next, let's see how retrieval and generation work together.", translatedText: "接下来看看检索与生成如何配合。" },
    { id: "demo-4", chunkIndex: 0, startMs: 1_095_000, endMs: 1_160_000, text: "A common mistake is focusing only on the vector database while ignoring chunking and retrieval quality.", translatedText: "常见误区是只关注向量数据库，而忽略内容切分和召回质量。" },
    { id: "demo-5", chunkIndex: 0, startMs: 1_350_000, endMs: 1_425_000, text: "Finally, we evaluate the result in terms of accuracy, relevance, and traceability.", translatedText: "最后，我们从准确性、相关性和可追溯性几个角度评估最终效果。" },
  ];
  return {
    jobId,
    modelId: "demo",
    language: "en",
    translationLanguage: "zh",
    text: segments.map((segment) => segment.text).join("\n"),
    segments,
  };
}

function VideoTranscriptWorkspace({
  active,
  job,
  media,
  mediaLoading,
  mediaError,
  mediaStatus,
  onPrepareVideo,
  transcript,
  rawTranscript,
  transcriptLoading,
  transcriptRefreshing,
  transcriptError,
  showPosterPreview,
  autoPlayOnTranscriptClick,
  onSegmentEdited,
  transcriptPhase,
  transcriptStatusMessage,
  asrPhaseProgress,
}: {
  active: boolean;
  job: Job;
  media: MediaPreparationResult | null;
  mediaLoading: boolean;
  mediaError: string;
  mediaStatus: string;
  onPrepareVideo?: () => void;
  transcript: TranscriptResult | null;
  rawTranscript: TranscriptResult | null;
  transcriptLoading: boolean;
  transcriptRefreshing: boolean;
  transcriptError: string;
  showPosterPreview: boolean;
  autoPlayOnTranscriptClick: boolean;
  onSegmentEdited?: (segmentId: string, text: string) => void;
  transcriptPhase: TranscriptUiPhase;
  transcriptStatusMessage?: string;
  asrPhaseProgress: AsrPhaseProgress | null;
}) {
  const videoRef = useRef<HTMLVideoElement>(null);
  const rowRefs = useRef(new Map<string, HTMLElement>());
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const [query, setQuery] = useState("");
  const [currentTimeMs, setCurrentTimeMs] = useState(() => showPosterPreview ? 402_000 : 0);
  const [durationMs, setDurationMs] = useState(() => durationToMilliseconds(job.duration));
  const [isPlaying, setIsPlaying] = useState(false);
  const [isMuted, setIsMuted] = useState(false);
  const [isFullscreen, setIsFullscreen] = useState(false);
  const [displayMode, setDisplayMode] = useState<TranscriptDisplayMode>(loadTranscriptDisplayMode);
  const [viewMode, setViewMode] = useState<TranscriptViewMode>(loadTranscriptViewMode);
  const [loadedViewTranscript, setLoadedViewTranscript] = useState<TranscriptResult | null>(null);
  const [viewLoading, setViewLoading] = useState(false);
  const [viewError, setViewError] = useState("");
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editingText, setEditingText] = useState("");
  const [savingEdit, setSavingEdit] = useState(false);
  const isRecognizing = transcriptPhase === "recognizing";
  const isRefining = transcriptPhase === "refining";
  const isTranscriptFinished = (job.status === "transcribed" || job.status === "completed") && !isRecognizing && !isRefining && Boolean(transcript);
  const activeTranscript = isTranscriptFinished
    ? (viewMode === "standard" ? transcript : (rawTranscript ?? loadedViewTranscript))
    : (rawTranscript ?? loadedViewTranscript ?? transcript);

  const videoUrl = useMemo(
    () => media?.videoFile ? runtime.localAssetUrl(media.videoFile) : undefined,
    [media?.videoFile],
  );
  const posterUrl = useMemo(
    () => media?.thumbnailFile
      ? runtime.localAssetUrl(media.thumbnailFile)
      : job.thumbnailUrl ?? fallbackThumbnailUrl,
    [job.thumbnailUrl, media?.thumbnailFile],
  );
  // Frontend is presentation-only: Raw and Standard segment identities/timestamps come from backend events/storage.
  const segments = useMemo(() => activeTranscript?.segments ?? [], [activeTranscript]);
  const asrPercent = asrPhaseProgress?.total && asrPhaseProgress.total > 0
    ? Math.round(Math.min(100, Math.max(0, (asrPhaseProgress.completed / asrPhaseProgress.total) * 100)))
    : null;
  const hasTranslations = useMemo(
    () => segments.some((segment) => Boolean(segment.translatedText?.trim())),
    [segments],
  );
  const phaseDescription = transcriptPhaseDescription(transcriptPhase, transcriptStatusMessage, hasTranslations);
  const effectiveDisplayMode = hasTranslations ? displayMode : "original";
  const mediaPhaseActive = job.status === "processing" && (job.phase === "media_download" || job.phase === "media_normalize");
  const isDownloadingMedia = mediaLoading || (mediaPhaseActive && !media?.videoFile);
  const hasMediaError = Boolean(mediaError || (job.status === "failed" && !media?.videoFile));
  const mediaPercent = job.phaseTotal && job.phaseCompleted != null && job.phaseTotal > 0
    ? Math.min(100, Math.max(0, Math.round((job.phaseCompleted / job.phaseTotal) * 100)))
    : 0;
  const isConnecting = isDownloadingMedia && mediaPercent <= 5;
  const activeSegment = useMemo(
    () => findActiveSegment(segments, currentTimeMs),
    [currentTimeMs, segments],
  );
  const visibleSegments = useMemo(() => {
    const normalizedQuery = query.trim().toLocaleLowerCase();
    if (!normalizedQuery) return segments;
    return segments.filter((segment) => (
      segment.text.toLocaleLowerCase().includes(normalizedQuery)
      || segment.translatedText?.toLocaleLowerCase().includes(normalizedQuery)
    ));
  }, [query, segments]);

  const selectDisplayMode = useCallback((mode: TranscriptDisplayMode) => {
    setDisplayMode(mode);
    saveTranscriptDisplayMode(mode);
  }, []);

  const selectViewMode = useCallback((mode: TranscriptViewMode) => {
    setViewError("");
    setViewMode(mode);
    saveTranscriptViewMode(mode);
    setEditingId(null);
  }, []);

  const startEditing = useCallback((segment: TranscriptSegment) => {
    setEditingId(segment.id);
    setEditingText(segment.text);
  }, []);

  const saveEditing = useCallback(async () => {
    if (!editingId) return;
    setSavingEdit(true);
    try {
      await runtime.updateTranscriptSegment(job.id, editingId, editingText.trim());
      onSegmentEdited?.(editingId, editingText.trim());
      setEditingId(null);
    } catch (reason) {
      window.alert(formatErrorMessage(reason));
    } finally {
      setSavingEdit(false);
    }
  }, [editingId, editingText, job.id, onSegmentEdited]);

  useEffect(() => {
    if (viewMode === "standard" || rawTranscript || !transcript) {
      setLoadedViewTranscript(null);
      setViewLoading(false);
      return undefined;
    }

    let active = true;
    setViewLoading(true);
    setViewError("");
    void runtime.loadTranscriptView(job.id, "raw")
      .then((result) => {
        if (active) setLoadedViewTranscript(result);
      })
      .catch((reason) => {
        if (!active) return;
        const message = formatErrorMessage(reason);
        setViewError(message);
        setViewMode("standard");
        saveTranscriptViewMode("standard");
      })
      .finally(() => {
        if (active) setViewLoading(false);
      });
    return () => {
      active = false;
    };
  }, [job.id, rawTranscript, transcript, viewMode]);

  useEffect(() => {
    if (!isPlaying || !activeSegment) return;
    rowRefs.current.get(activeSegment.id)?.scrollIntoView({ block: "nearest" });
  }, [activeSegment, isPlaying]);

  useEffect(() => {
    const syncFullscreenState = () => {
      const frame = videoRef.current?.closest(".local-video-frame");
      setIsFullscreen(frame instanceof HTMLElement && document.fullscreenElement === frame);
    };
    document.addEventListener("fullscreenchange", syncFullscreenState);
    syncFullscreenState();
    return () => document.removeEventListener("fullscreenchange", syncFullscreenState);
  }, []);

  useEffect(() => {
    if (isRecognizing && segments.length > 0 && !isPlaying) {
      const el = scrollRef.current;
      if (el) {
        el.scrollTo({ top: el.scrollHeight, behavior: "smooth" });
      }
    }
  }, [segments.length, isRecognizing, isPlaying]);

  useEffect(() => {
    if (!active) videoRef.current?.pause();
  }, [active]);

  const seekTo = useCallback((milliseconds: number, playAfterSeek = false) => {
    const nextTimeMs = Math.max(0, Math.min(milliseconds, durationMs || milliseconds));
    const video = videoRef.current;
    if (video && Number.isFinite(video.duration)) {
      video.currentTime = nextTimeMs / 1000;
    }
    if (playAfterSeek && video?.currentSrc) {
      void video.play().catch(() => undefined);
    }
    setCurrentTimeMs(nextTimeMs);
  }, [durationMs]);

  const togglePlayback = useCallback(() => {
    const video = videoRef.current;
    if (!video?.currentSrc) return;
    if (video.paused) {
      void video.play();
    } else {
      video.pause();
    }
  }, []);

  const toggleMuted = useCallback(() => {
    const video = videoRef.current;
    if (!video) return;
    video.muted = !video.muted;
    setIsMuted(video.muted);
  }, []);

  const toggleFullscreen = useCallback(() => {
    const frame = videoRef.current?.closest(".local-video-frame");
    if (!(frame instanceof HTMLElement)) return;
    if (document.fullscreenElement) {
      void document.exitFullscreen().catch(() => undefined);
    } else {
      void frame.requestFullscreen().catch(() => undefined);
    }
  }, []);

  return (
    <div className="video-transcript-workspace">
      <section className="player-pane" aria-label="本地视频播放器">
        <div className="local-video-frame">
          <video
            ref={videoRef}
            src={videoUrl}
            poster={posterUrl}
            preload="metadata"
            onClick={togglePlayback}
            onLoadedMetadata={(event) => {
              const seconds = event.currentTarget.duration;
              if (Number.isFinite(seconds)) setDurationMs(Math.round(seconds * 1000));
            }}
            onTimeUpdate={(event) => setCurrentTimeMs(Math.round(event.currentTarget.currentTime * 1000))}
            onPlay={() => setIsPlaying(true)}
            onPause={() => setIsPlaying(false)}
            onEnded={() => setIsPlaying(false)}
          />
          {!videoUrl && !showPosterPreview ? (
            <div className="video-unavailable">
              {isConnecting ? (
                <div className="video-downloading-state">
                  <Loader2 className="spin" size={28} style={{ color: "#60a5fa", marginBottom: 2 }} />
                  <strong>正在连接视频平台……</strong>
                  <span className="video-download-status">正在解析视频信息与建立连接</span>
                </div>
              ) : isDownloadingMedia ? (
                <div className="video-downloading-state">
                  <Loader2 className="spin" size={28} style={{ color: "#60a5fa", marginBottom: 2 }} />
                  <strong>正在下载视频……</strong>
                  <span className="video-download-status">{mediaStatus || job.statusMessage || "正在传输高清视频流……"}</span>
                  <div className="video-progress-track">
                    <div className="video-progress-fill" style={{ width: `${mediaPercent}%` }} />
                  </div>
                  <span className="video-progress-number">{mediaPercent}%</span>
                </div>
              ) : hasMediaError ? (
                <div className="video-error-state">
                  <strong style={{ color: "#f87171" }}>本地视频准备失败</strong>
                  <span>{mediaError || job.errorMessage || "视频下载失败，请检查网络或链接有效性"}</span>
                  {onPrepareVideo ? (
                    <button type="button" onClick={onPrepareVideo}>
                      <RefreshCw size={15} />
                      重新尝试下载
                    </button>
                  ) : null}
                </div>
              ) : (
                <div className="video-missing-state">
                  <strong>当前任务未包含本地视频</strong>
                  <span>{mediaStatus || "现有转录和笔记已保存在本地，可随时补充下载视频。"}</span>
                  {onPrepareVideo ? (
                    <button type="button" onClick={onPrepareVideo}>
                      <Download size={15} />
                      补充下载本地视频
                    </button>
                  ) : null}
                </div>
              )}
            </div>
          ) : null}
          <div className="video-controls">
            <button type="button" aria-label={isPlaying ? "暂停" : "播放"} onClick={togglePlayback} disabled={!videoUrl}>
              {isPlaying ? <Pause size={18} fill="currentColor" /> : <Play size={18} fill="currentColor" />}
            </button>
            <button type="button" aria-label={isMuted ? "取消静音" : "静音"} onClick={toggleMuted} disabled={!videoUrl}>
              {isMuted ? <VolumeX size={18} /> : <Volume2 size={18} />}
            </button>
            <span>{formatTimestamp(currentTimeMs)} / {formatTimestamp(durationMs)}</span>
            <input
              aria-label="视频进度"
              type="range"
              min="0"
              max={Math.max(durationMs, 1)}
              value={Math.min(currentTimeMs, Math.max(durationMs, 1))}
              onChange={(event) => seekTo(Number(event.currentTarget.value))}
            />
            <span className="quality-label">最高 720p</span>
            <button
              type="button"
              aria-label={isFullscreen ? "退出全屏" : "全屏"}
              title={isFullscreen ? "退出全屏" : "全屏"}
              onClick={toggleFullscreen}
              disabled={!videoUrl && !showPosterPreview}
            >
              {isFullscreen ? <Minimize2 size={18} /> : <Maximize2 size={18} />}
            </button>
          </div>
        </div>
        <div className={`local-video-status ${videoUrl || showPosterPreview ? "is-ready" : ""}`}>
          <span aria-hidden="true" />
          {videoUrl
            ? "本地视频 · 最高 720p"
            : showPosterPreview
              ? "本地视频 · 最高 720p"
              : mediaLoading
                ? "正在准备本地视频"
                : "等待本地视频"}
        </div>
      </section>

      <section className="synced-transcript-pane" aria-label="同步转录">
        <div className="transcript-panel-header">
          <div>
            <strong>转录内容</strong>
            <span>
              {mediaPhaseActive
                ? "等待视频下载完成后开始转写"
                : transcriptRefreshing && transcriptPhase === "idle"
                  ? "正在同步最新转录结果……"
                  : phaseDescription}
            </span>
          </div>
          <div className="transcript-header-controls">
            {isRecognizing && asrPercent != null ? (
              <div className="transcript-live-progress-wrap" role="progressbar" aria-valuenow={asrPercent} aria-valuemin={0} aria-valuemax={100} title={`当前识别阶段 ${asrPercent}%`}>
                <div className="transcript-live-track">
                  <div className="transcript-live-fill" style={{ width: `${asrPercent}%` }} />
                </div>
                <span className="transcript-live-percent">{asrPercent}%</span>
              </div>
            ) : isRefining ? (
              <div className="transcript-phase-indicator" role="status" aria-live="polite">
                <Loader2 size={14} className="spin" aria-hidden="true" />
                <span>校对中</span>
              </div>
            ) : transcriptRefreshing ? (
              <span className="transcript-refresh-indicator" role="status">
                <Loader2 size={13} className="spin" aria-hidden="true" />
                同步中
              </span>
            ) : null}
            {isTranscriptFinished ? (
              <div className="transcript-display-switch" role="group" aria-label="转录文本视图">
                {([
                  ["raw", "原始"],
                  ["standard", "校正（标准）"],
                ] as const).map(([mode, label]) => (
                  <button
                    className={viewMode === mode ? "is-active" : ""}
                    type="button"
                    aria-pressed={viewMode === mode}
                    key={mode}
                    onClick={() => selectViewMode(mode)}
                  >
                    {label}
                  </button>
                ))}
              </div>
            ) : null}
            {hasTranslations ? (
              <div className="transcript-display-switch" role="group" aria-label="转录显示语言">
                {([
                  ["translated", "中文"],
                  ["bilingual", "双语"],
                  ["original", "原文"],
                ] as const).map(([mode, label]) => (
                  <button
                    className={effectiveDisplayMode === mode ? "is-active" : ""}
                    type="button"
                    aria-pressed={effectiveDisplayMode === mode}
                    key={mode}
                    onClick={() => selectDisplayMode(mode)}
                  >
                    {label}
                  </button>
                ))}
              </div>
            ) : transcript && !/^(zh|yue|chinese)/i.test(transcript.language ?? "") ? (
              <span className="transcript-mode-hint">{transcriptPhase === "translating" ? "已完成部分将实时呈现……" : "需要翻译时，可在页面上方点击“翻译成中文”"}</span>
            ) : null}
          </div>
        </div>
        <div className="transcript-toolbar">
          <label className="transcript-search">
            <Search size={17} aria-hidden="true" />
            <input
              value={query}
              onChange={(event) => setQuery(event.currentTarget.value)}
              placeholder="搜索转录内容"
              aria-label="搜索转录内容"
            />
          </label>
          <span>{autoPlayOnTranscriptClick ? "点击段落跳转并播放" : "点击段落跳转到对应时间"}</span>
        </div>
        <div className="synced-transcript-scroll" ref={scrollRef}>
          {transcriptLoading && segments.length === 0 ? <p className="transcript-state">正在读取本地转录结果……</p> : null}
          {viewLoading && segments.length === 0 ? <p className="transcript-state">正在切换转录视图……</p> : null}
          {transcriptError ? (
            <p className={`model-error transcript-state ${segments.length > 0 ? "is-nonblocking" : ""}`}>
              {segments.length > 0 ? `最新转录同步失败，当前内容已保留：${transcriptError}` : transcriptError}
            </p>
          ) : null}
          {viewError ? <p className="model-error transcript-state">{viewError}</p> : null}
          {visibleSegments.map((segment) => (
            <div
              className={`transcript-segment ${activeSegment?.id === segment.id ? "is-active" : ""} ${editingId === segment.id ? "is-editing" : ""}`}
              key={segment.id}
              ref={(node) => {
                if (node) rowRefs.current.set(segment.id, node);
                else rowRefs.current.delete(segment.id);
              }}
            >
              <button
                className="transcript-segment-main"
                type="button"
                aria-current={activeSegment?.id === segment.id ? "true" : undefined}
                onClick={() => seekTo(segment.startMs, autoPlayOnTranscriptClick)}
              >
                <time>[{formatTimestamp(segment.startMs)} - {formatTimestamp(segment.endMs)}]</time>
                <span className="transcript-text">
                  {effectiveDisplayMode === "translated" ? (
                    <span className="transcript-translation">{segment.translatedText || segment.text}</span>
                  ) : effectiveDisplayMode === "bilingual" ? (
                    <>
                      <span className="transcript-translation">{segment.translatedText || segment.text}</span>
                      {segment.translatedText ? <span className="transcript-original">{segment.text}</span> : null}
                    </>
                  ) : (
                    <span className="transcript-original">{segment.text}</span>
                  )}
                </span>
              </button>
              {viewMode === "standard" && editingId === segment.id ? (
                <div className="transcript-edit-box">
                  <textarea
                    value={editingText}
                    onChange={(event) => setEditingText(event.currentTarget.value)}
                    rows={3}
                    aria-label="编辑转录文本"
                  />
                  <div className="transcript-edit-actions">
                    <button type="button" disabled={savingEdit} onClick={() => void saveEditing()}>
                      {savingEdit ? "保存中…" : "保存"}
                    </button>
                    <button type="button" onClick={() => setEditingId(null)}>取消</button>
                  </div>
                </div>
              ) : viewMode === "standard" ? (
                <button
                  className="transcript-edit-trigger"
                  type="button"
                  aria-label="编辑该段转录"
                  onClick={() => startEditing(segment)}
                >
                  <Pencil size={13} aria-hidden="true" />
                </button>
              ) : null}
            </div>
          ))}
          {isRecognizing ? (
            <div className={`transcript-streaming-cursor ${segments.length === 0 ? "is-waiting" : ""}`} aria-label="正在实时识别">
              <span className="step-pulse" />
              <span className="cursor-text">{segments.length === 0 ? "正在准备识别……" : "正在实时识别，已识别内容会继续保留……"}</span>
            </div>
          ) : isRefining && segments.length > 0 ? (
            <div className="transcript-streaming-cursor is-refining" aria-label="正在校对转录">
              <Sparkles size={14} aria-hidden="true" />
              <span className="cursor-text">正在按时间轴生成并复核校正（标准）结果……</span>
            </div>
          ) : null}
          {!isRecognizing && !transcriptLoading && !transcriptError && segments.length === 0 ? (
            <p className="transcript-state">
              {job.status === "processing"
                ? (mediaPhaseActive ? "正在准备媒体，完成后自动开始语音转写……" : "正在处理语音内容，请稍候……")
                : "没有识别到有效语音内容。"}
            </p>
          ) : null}
          {!transcriptLoading && segments.length > 0 && visibleSegments.length === 0 ? (
            <p className="transcript-state">没有找到包含“{query}”的转录内容。</p>
          ) : null}
        </div>
      </section>
    </div>
  );
}

function LogContent({ job, completed, usesRealMediaPipeline, progress }: { job: Job; completed: boolean; usesRealMediaPipeline: boolean; progress: number }) {
  const steps = usesRealMediaPipeline ? buildRealSteps(job) : buildSteps(progress);

  return (
    <div className="log-list" aria-label="处理日志">
      {steps.map((step) => (
        <div className="log-row" key={step.id}>
          <span className="log-check">
            {step.state === "completed" ? (
              <Check size={13} strokeWidth={2.5} />
            ) : step.state === "active" ? (
              <Loader2 size={13} className="spin" />
            ) : (
              <span className="pending-dot" style={{ display: "inline-block", width: 6, height: 6, borderRadius: "50%", background: "rgba(255,255,255,0.3)" }} />
            )}
          </span>
          <div>
            <strong>{step.label}</strong>
            <span>{completed && step.id === "summarize" ? "已完成" : step.detail}</span>
          </div>
        </div>
      ))}
    </div>
  );
}

export function TaskDetailPage({
  job,
  onBack,
  onNavigateToModels,
  onComplete,
  onCancelMedia,
  onResetJob,
  onRetryMedia,
  onTranslate,
  onOrganize,
  onReorganize,
  onTranscriptEdited,
  autoPlayOnTranscriptClick,
  usesRealMediaPipeline,
  noteRevision,
}: TaskDetailPageProps) {
  const [activeTab, setActiveTab] = useState<TaskTab>("workspace");
  const [progress, setProgress] = useState(job.progress);
  const [isPaused, setIsPaused] = useState(job.status === "paused");
  const [isReorganizing, setIsReorganizing] = useState(false);
  const [realTranscript, setRealTranscript] = useState<TranscriptResult | null>(null);
  const [rawTranscript, setRawTranscript] = useState<TranscriptResult | null>(null);
  const realTranscriptRef = useRef<TranscriptResult | null>(null);
  const [asrPipelinePhase, setAsrPipelinePhase] = useState<AsrPipelinePhase | null>(null);
  const [asrPhaseMessage, setAsrPhaseMessage] = useState("");
  const [asrPhaseProgress, setAsrPhaseProgress] = useState<AsrPhaseProgress | null>(null);
  const [transcriptLoading, setTranscriptLoading] = useState(false);
  const [transcriptRefreshing, setTranscriptRefreshing] = useState(false);
  const [transcriptError, setTranscriptError] = useState("");
  const [media, setMedia] = useState<MediaPreparationResult | null>(null);
  const [mediaLoading, setMediaLoading] = useState(false);
  const [mediaError, setMediaError] = useState("");
  const [mediaStatus, setMediaStatus] = useState("");
  const [realNote, setRealNote] = useState<NoteResult | null>(null);
  const [noteLoading, setNoteLoading] = useState(false);
  const [noteError, setNoteError] = useState("");
  const [isExporting, setIsExporting] = useState(false);
  const [exportFeedback, setExportFeedback] = useState<{ kind: "success" | "error"; message: string } | null>(null);
  const [actionPending, setActionPending] = useState<"pause" | "cancel" | null>(null);
  const [cancelFeedback, setCancelFeedback] = useState<string | null>(null);
  const [liveSegments, setLiveSegments] = useState<{ startMs: number; text: string }[]>([]);
  const isComplete = !isReorganizing && (usesRealMediaPipeline ? job.status === "completed" : (job.status === "completed" || progress >= 100));
  const isTranscribed = job.status === "transcribed";
  const isWaiting = job.status === "waiting";
  const isFailed = job.status === "failed";
  const canTranslateTranscript = Boolean(realTranscript && !/^(zh|yue|chinese)/i.test(realTranscript.language ?? ""));
  const hasTranslations = Boolean(realTranscript?.segments.some((segment) => Boolean(segment.translatedText?.trim())));
  const isAsrPhase = ["recognition", "pause_alignment", "verification", "boundary_review", "word_alignment", "standardization", "semantic_segmentation"].includes(job.phase ?? "");
  const isMediaReady = isWaiting || isTranscribed || isComplete || isAsrPhase || job.phase === "translation" || job.phase === "summary";
  const isResultReady = isComplete || isTranscribed || (usesRealMediaPipeline && isMediaReady);
  const transcriptPhase = resolveTranscriptUiPhase(job, isComplete, isTranscribed, asrPipelinePhase);
  const transcriptLoadMilestone = isComplete
    ? "complete"
    : isTranscribed || job.phase === "translation" || job.phase === "summary"
      ? "transcribed"
      : job.status === "paused"
        ? "paused"
        : (job.status === "processing" && (isAsrPhase || !job.phase || job.phase === "media_normalize" || job.phase === "recognition"))
          ? "processing"
          : "none";

  useEffect(() => {
    realTranscriptRef.current = realTranscript;
  }, [realTranscript]);

  useEffect(() => {
    setRawTranscript(null);
    setRealTranscript(null);
    setLiveSegments([]);
    setRealNote(null);
    setNoteError("");
    setTranscriptError("");
    setAsrPipelinePhase(null);
    setAsrPhaseMessage("");
    setAsrPhaseProgress(null);
  }, [job.id]);

  // Raw 与 Standard 使用独立时间轴。Snapshot 永远按 payload.view 分流，
  // 不通过 progress 推断当前阶段，也不会因为进入复听/标准化而清空已有文本。
  useEffect(() => {
    if (!usesRealMediaPipeline || job.status !== "processing") return undefined;
    let active = true;
    const unlistenPromise = listen<AsrSnapshot>("asr-snapshot", ({ payload }) => {
      if (!active || payload.jobId !== job.id) return;

      const updateFromSnapshot = (
        prev: TranscriptResult | null,
        preserveTranslations: boolean,
      ): TranscriptResult | null => {
        const previousById = new Map((prev?.segments ?? []).map((segment) => [segment.id, segment]));
        const nextSegments = payload.segments?.length
          ? payload.segments.map((segment) => {
              const previous = previousById.get(segment.id);
              return preserveTranslations && previous?.translatedText && !segment.translatedText
                ? { ...segment, translatedText: previous.translatedText }
                : segment;
            })
          : (prev?.segments ?? []);
        const nextRepairs = payload.pauseRepairs ?? prev?.pauseRepairs;
        if (nextSegments.length === 0 && !nextRepairs?.length) return prev;
        return {
          jobId: job.id,
          modelId: payload.modelId || prev?.modelId || "funasr-nano",
          language: payload.language || prev?.language || "zh",
          translationLanguage: preserveTranslations ? prev?.translationLanguage : undefined,
          text: nextSegments.map((segment) => segment.text).join("\n"),
          segments: nextSegments,
          pauseRepairs: nextRepairs,
        };
      };

      if (payload.view === "raw") {
        setRawTranscript((prev) => updateFromSnapshot(prev, false));
        if (payload.segments?.length) {
          setTranscriptLoading(false);
          setTranscriptRefreshing(false);
          setTranscriptError("");
        }
      } else {
        setRealTranscript((prev) => updateFromSnapshot(prev, true));
        if (payload.segments?.length || payload.pauseRepairs?.length) {
          setTranscriptLoading(false);
          setTranscriptRefreshing(false);
          setTranscriptError("");
        }
      }
    });
    return () => {
      active = false;
      void unlistenPromise.then((fn) => fn());
    };
  }, [usesRealMediaPipeline, job.status, job.id]);

  // ASR pipeline stages are explicit lifecycle events. Progress is display-only and never
  // determines whether the UI is recognizing, aligning, verifying, or standardizing.
  useEffect(() => {
    if (!usesRealMediaPipeline || job.status !== "processing") return undefined;
    let active = true;
    const phaseUnlisten = listen<AsrPhaseEvent>("asr-phase", ({ payload }) => {
      if (!active || payload.jobId !== job.id) return;
      if (payload.phase === "recognition" && payload.state === "started") {
        // A rerun replaces the previous subtitle result.  Raw can stream during
        // recognition; Standard stays empty until the final canonical snapshot.
        setRawTranscript(null);
        setRealTranscript(null);
        setTranscriptLoading(true);
        setTranscriptRefreshing(false);
        setTranscriptError("");
      }
      setAsrPhaseMessage(payload.message);
      setAsrPipelinePhase((current) => {
        if (payload.state === "started") return payload.phase;
        return current === payload.phase ? null : current;
      });
    });
    const progressUnlisten = listen<AsrPhaseProgress>("asr-phase-progress", ({ payload }) => {
      if (!active || payload.jobId !== job.id) return;
      setAsrPhaseProgress(payload);
    });
    return () => {
      active = false;
      void phaseUnlisten.then((fn) => fn());
      void progressUnlisten.then((fn) => fn());
    };
  }, [usesRealMediaPipeline, job.status, job.id]);

  // 翻译阶段实时监听 translation-segment-update 事件，实时流式更新中文字幕
  useEffect(() => {
    if (!usesRealMediaPipeline) return undefined;
    let active = true;
    const unlistenPromise = listen<TranslationSegmentUpdate>("translation-segment-update", ({ payload }) => {
      if (!active || payload.jobId !== job.id) return;
      setRealTranscript((prev) => {
        if (!prev) return prev;
        const targetIdx = prev.segments.findIndex((s) => s.id === payload.segmentId);
        if (targetIdx < 0) return prev;
        const nextSegments = [...prev.segments];
        nextSegments[targetIdx] = {
          ...nextSegments[targetIdx],
          translatedText: payload.translatedText,
        };
        return {
          ...prev,
          segments: nextSegments,
        };
      });
    });
    return () => {
      active = false;
      void unlistenPromise.then((fn) => fn());
    };
  }, [usesRealMediaPipeline, job.id]);

  useEffect(() => {
    if (!usesRealMediaPipeline) return;
    setProgress(job.progress);
    setIsPaused(job.status === "paused");
  }, [job.progress, job.status, usesRealMediaPipeline]);

  useEffect(() => {
    if (transcriptLoadMilestone === "none") {
      setTranscriptLoading(false);
      setTranscriptRefreshing(false);
      setTranscriptError("");
      return undefined;
    }

    let active = true;
    if (isTranscribed) setActiveTab("workspace");
    const hasExistingTranscript = Boolean(realTranscriptRef.current?.segments.length);
    setTranscriptLoading(!hasExistingTranscript && transcriptLoadMilestone !== "processing");
    setTranscriptRefreshing(hasExistingTranscript);
    setTranscriptError("");

    const transcriptPromise = usesRealMediaPipeline
      ? runtime.loadTranscript(job.id)
      : Promise.resolve(buildDemoTranscript(job.id));
    void transcriptPromise
      .then((result) => {
        if (!active) return;
        setRealTranscript(result);
        if (result && result.segments.length > 0) {
          setRawTranscript(result);
        }
      })
      .catch((reason) => {
        if (!active) return;
        setTranscriptError(formatErrorMessage(reason));
      })
      .finally(() => {
        if (!active) return;
        setTranscriptLoading(false);
        setTranscriptRefreshing(false);
      });

    return () => {
      active = false;
    };
  }, [isTranscribed, job.id, transcriptLoadMilestone, usesRealMediaPipeline]);

  useEffect(() => {
    if (!usesRealMediaPipeline) {
      setMedia(null);
      setMediaLoading(false);
      setMediaError("");
      return undefined;
    }
    let active = true;
    if (isMediaReady) {
      setMediaLoading(true);
      void runtime.loadMedia(job.id)
        .then((result) => {
          if (active) {
            setMedia(result);
            setMediaError("");
          }
        })
        .catch((reason) => {
          if (active) setMediaError(formatErrorMessage(reason));
        })
        .finally(() => {
          if (active) setMediaLoading(false);
        });
    } else {
      setMediaLoading(false);
      setMediaError("");
    }
    return () => {
      active = false;
    };
  }, [isMediaReady, job.id, usesRealMediaPipeline]);

  const prepareMissingVideo = useCallback(() => {
    if (!usesRealMediaPipeline || mediaLoading) return;
    setMediaLoading(true);
    setMediaError("");
    setMediaStatus("正在准备下载本地视频……");
    void runtime.prepareMedia(job.id, job.sourceUrl, (progressUpdate) => {
      setMediaStatus(progressUpdate.message);
    })
      .then((result) => {
        setMedia(result);
        setMediaStatus("本地视频已准备完成");
      })
      .catch((reason) => setMediaError(formatErrorMessage(reason)))
      .finally(() => setMediaLoading(false));
  }, [job.id, job.sourceUrl, mediaLoading, usesRealMediaPipeline]);

  useEffect(() => {
    if (!isComplete || !usesRealMediaPipeline) return undefined;
    let active = true;
    setRealNote(null);
    setNoteLoading(true);
    setNoteError("");
    void runtime.loadNote(job.id)
      .then((result) => {
        if (active) setRealNote(result);
      })
      .catch((reason) => {
        if (active) setNoteError(formatErrorMessage(reason));
      })
      .finally(() => {
        if (active) setNoteLoading(false);
      });
    return () => {
      active = false;
    };
  }, [isComplete, job.id, noteRevision, usesRealMediaPipeline]);

  useEffect(() => {
    if (usesRealMediaPipeline) return undefined;
    const canAdvance = job.status === "processing" || job.status === "paused" || isReorganizing;
    if (!canAdvance || isPaused || progress >= 100) return undefined;

    const timer = window.setInterval(() => {
      setProgress((current) => Math.min(100, current + 2));
    }, 180);
    return () => window.clearInterval(timer);
  }, [isPaused, isReorganizing, job.status, progress, usesRealMediaPipeline]);

  useEffect(() => {
    if (progress !== 100) return;
    if (isReorganizing) {
      setIsReorganizing(false);
    } else if (job.status !== "completed" && !usesRealMediaPipeline) {
      onComplete(job.id);
    }
  }, [isReorganizing, job.id, job.status, onComplete, progress, usesRealMediaPipeline]);

  const steps = useMemo(
    () => (usesRealMediaPipeline
      ? isComplete ? realCompletedSteps : isTranscribed ? transcribedSteps : isWaiting ? mediaReadySteps : buildRealSteps(job)
      : isComplete ? completedSteps : buildSteps(progress)),
    [isComplete, isTranscribed, isWaiting, job, progress, usesRealMediaPipeline],
  );
  const currentStage = Math.min(processingCopy.length - 1, Math.floor(progress / 34));

  const handleSegmentEdited = useCallback((segmentId: string, text: string) => {
    setRealTranscript((prev) => {
      if (!prev) return prev;
      const segments = prev.segments.map((segment) =>
        segment.id === segmentId ? { ...segment, text, translatedText: undefined } : segment,
      );
      return {
        ...prev,
        translationLanguage: segments.some((segment) => Boolean(segment.translatedText?.trim())) ? prev.translationLanguage : undefined,
        segments,
        text: segments.map((segment) => segment.text).join("\n"),
      };
    });
    setRealNote(null);
    onTranscriptEdited();
  }, [onTranscriptEdited]);

  const handleRetryMedia = useCallback((opts?: { resume?: boolean }) => {
    if (!opts?.resume) {
      setRealTranscript(null);
      setRawTranscript(null);
      setLiveSegments([]);
      setRealNote(null);
      setNoteError("");
      setTranscriptError("");
      setAsrPipelinePhase(null);
      setAsrPhaseMessage("");
      setAsrPhaseProgress(null);
      setActiveTab("workspace");
    }
    onRetryMedia(opts);
  }, [onRetryMedia]);

  const handleResetJob = useCallback(async () => {
    setRealTranscript(null);
    setRawTranscript(null);
    setLiveSegments([]);
    setRealNote(null);
    setNoteError("");
    setTranscriptError("");
    setAsrPipelinePhase(null);
    setAsrPhaseMessage("");
    setAsrPhaseProgress(null);
    setActiveTab("workspace");
    if (onResetJob) {
      await onResetJob();
    } else {
      await onCancelMedia();
    }
  }, [onResetJob, onCancelMedia]);

  const cancelTask = async () => {
    if (actionPending) return;
    setActionPending("pause");
    setCancelFeedback("正在停止后台处理，已完成的转写与整理片段会保留");
    try {
      await onCancelMedia();
    } catch {
      setCancelFeedback("暂停失败，请稍后重试");
    } finally {
      setActionPending(null);
      setCancelFeedback(null);
    }
  };

  const exportMarkdown = async () => {
    const markdown = usesRealMediaPipeline ? realNote?.markdown : noteMarkdown;
    if (!markdown || isExporting) return;
    const safeTitle = job.title.replace(/[<>:"/\\|?*\u0000-\u001F]/g, "_").trim() || "video-notes";
    setIsExporting(true);
    setExportFeedback(null);
    try {
      const path = await runtime.exportMarkdown(`${safeTitle}.md`, markdown);
      if (path) {
        setExportFeedback({
          kind: "success",
          message: runtime.isDesktop() ? "Markdown 已保存" : "Markdown 已下载",
        });
      }
    } catch (reason) {
      setExportFeedback({
        kind: "error",
        message: reason instanceof Error ? reason.message : `导出失败：${String(reason)}`,
      });
    } finally {
      setIsExporting(false);
    }
  };

  return (
    <section className="task-detail-page is-result-view">
      <div className="task-content-column">
        <button className="back-button" type="button" onClick={onBack}>
          <ArrowLeft size={18} strokeWidth={1.9} aria-hidden="true" />
          返回任务
        </button>

        <header className="task-header">
          <div className="task-title-block">
            <h1>{job.title}</h1>
            <p>
              {job.platform === "bilibili" ? "哔哩哔哩" : "抖音"} · {job.duration}
              {media?.videoFile || !usesRealMediaPipeline ? (
                <span className="saved-media-meta"><Check size={13} />已保存到本地</span>
              ) : null}
            </p>
          </div>
          <div className="task-header-actions">
            {isComplete ? (
              <div className="export-action">
                {canTranslateTranscript ? (
                  <button className="secondary-button" type="button" onClick={onTranslate}>
                    <Sparkles size={16} />
                    {hasTranslations ? "更新翻译" : "翻译成中文"}
                  </button>
                ) : null}
                <button
                  className="secondary-button"
                  type="button"
                  disabled={isExporting || (usesRealMediaPipeline && !realNote)}
                  onClick={() => void exportMarkdown()}
                >
                  <Download size={17} aria-hidden="true" />
                  {isExporting ? "正在导出…" : "导出 Markdown"}
                </button>
                <button className="secondary-button" type="button" onClick={onReorganize}>
                  <RefreshCw size={17} />
                  重新生成笔记
                </button>
                {exportFeedback ? (
                  <span className={`export-feedback is-${exportFeedback.kind}`} role="status">
                    {exportFeedback.message}
                  </span>
                ) : null}
                {job.statusMessage ? (
                  <span className="cancel-feedback" role="status">{job.statusMessage}</span>
                ) : null}
              </div>
            ) : isTranscribed ? (
              <div className="export-action">
                {canTranslateTranscript ? (
                  <button className="secondary-button" type="button" onClick={onTranslate}>
                    <Sparkles size={16} />
                    {hasTranslations ? "更新翻译" : "翻译成中文"}
                  </button>
                ) : null}
                <button className="secondary-button" type="button" onClick={onOrganize}>
                  <RefreshCw size={17} />
                  生成笔记
                </button>
                {onNavigateToModels && job.errorCode === "MODEL_NOT_INSTALLED" ? (
                  <button className="secondary-button" type="button" onClick={onNavigateToModels}>
                    <Sparkles size={16} />
                    前往模型管理
                  </button>
                ) : null}
                {job.statusMessage ? (
                  <span className="cancel-feedback" role="status">{job.statusMessage}</span>
                ) : null}
              </div>
            ) : isFailed || (usesRealMediaPipeline && job.status === "paused") ? (
              <div className="export-action">
                {job.phase === "media_download" ? (
                  <button className="secondary-button" type="button" onClick={() => handleRetryMedia()}>
                    {job.status === "paused" ? <Play size={17} /> : <RefreshCw size={17} />}
                    {job.status === "paused" ? "继续下载" : "重新下载"}
                  </button>
                ) : isMossBackend(job.asrBackend) ? (
                  <div style={{ display: "flex", gap: "8px" }}>
                    <button className="secondary-button" type="button" onClick={() => handleRetryMedia({ resume: true })}>
                      <Play size={17} />
                      继续转写
                    </button>
                    <button className="secondary-button" type="button" onClick={() => handleRetryMedia({ resume: false })}>
                      <RefreshCw size={17} />
                      重新开始
                    </button>
                  </div>
                ) : (
                  <button className="secondary-button" type="button" onClick={() => handleRetryMedia({ resume: false })}>
                    <RefreshCw size={17} />
                    重新开始
                  </button>
                )}
                {onNavigateToModels && job.errorCode === "MODEL_NOT_INSTALLED" ? (
                  <button className="secondary-button" type="button" onClick={onNavigateToModels}>
                    <Sparkles size={16} />
                    前往模型管理
                  </button>
                ) : null}
                {job.errorMessage || job.statusMessage ? (
                  <span
                    className={`cancel-feedback ${isFailed || Boolean(job.errorMessage) ? "is-error" : ""}`}
                    role="status"
                  >
                    {job.errorMessage || job.statusMessage}
                  </span>
                ) : null}
              </div>
            ) : isWaiting ? (
              <div className="export-action">
                <button className="secondary-button" type="button" onClick={() => handleRetryMedia({ resume: false })}>
                  <Play size={17} />
                  开始转写
                </button>
                {onNavigateToModels && job.errorCode === "MODEL_NOT_INSTALLED" ? (
                  <button className="secondary-button" type="button" onClick={onNavigateToModels}>
                    <Sparkles size={16} />
                    前往模型管理
                  </button>
                ) : null}
                {job.statusMessage ? (
                  <span className="cancel-feedback" role="status">{job.statusMessage}</span>
                ) : null}
              </div>
            ) : (
              <div className="export-action">
                <div style={{ display: "flex", gap: "8px" }}>
                  <button
                    className="secondary-button"
                    type="button"
                    disabled={Boolean(actionPending)}
                    onClick={() => {
                      if (usesRealMediaPipeline) void cancelTask();
                      else setIsPaused((current) => !current);
                    }}
                  >
                    {usesRealMediaPipeline ? (
                      actionPending === "pause" ? <Loader2 size={17} className="spin" /> : <Pause size={17} />
                    ) : isPaused ? <RefreshCw size={17} /> : <Pause size={17} />}
                    {usesRealMediaPipeline
                      ? actionPending === "pause"
                        ? "正在暂停…"
                        : job.phase === "media_download"
                          ? "暂停下载"
                          : job.phase === "translation"
                            ? "取消翻译"
                            : job.phase === "summary"
                              ? "取消生成"
                              : "暂停转写"
                      : isPaused
                        ? "继续处理"
                        : "暂停任务"}
                  </button>
                  {usesRealMediaPipeline && (job.phase === "recognition" || !job.phase || job.phase === "media_download") ? (
                    <button
                      className="secondary-button"
                      type="button"
                      disabled={Boolean(actionPending)}
                      title="取消当前进度并重置任务"
                      onClick={async () => {
                        if (actionPending) return;
                        setActionPending("cancel");
                        setCancelFeedback("正在取消并重置任务…");
                        try {
                          await handleResetJob();
                        } finally {
                          setActionPending(null);
                          setCancelFeedback(null);
                        }
                      }}
                    >
                      {actionPending === "cancel" ? <Loader2 size={16} className="spin" /> : <RefreshCw size={16} />}
                      {actionPending === "cancel" ? "正在取消…" : "取消任务"}
                    </button>
                  ) : null}
                </div>
                {cancelFeedback ? (
                  <span className="cancel-feedback" role="status">{cancelFeedback}</span>
                ) : null}
              </div>
            )}
          </div>
        </header>

        <div className="task-tabs" role="tablist" aria-label="任务结果">
          {(isComplete
            ? [["workspace", "视频与转录"], ["note", "笔记"], ["log", "处理记录"]] as const
            : [["workspace", "视频与转录"], ["log", "处理记录"]] as const
          ).map(([id, label]) => (
            <button
              className={activeTab === id ? "is-active" : ""}
              type="button"
              role="tab"
              aria-selected={activeTab === id}
              key={id}
              onClick={() => setActiveTab(id)}
            >
              {label}
            </button>
          ))}
        </div>
        <div className={`document-scroll ${activeTab === "workspace" ? "is-workspace" : ""}`}>
          <div className="workspace-tab-panel" hidden={activeTab !== "workspace"}>
            <VideoTranscriptWorkspace
              active={activeTab === "workspace"}
              job={job}
              media={media}
              mediaLoading={mediaLoading}
              mediaError={mediaError}
              mediaStatus={mediaStatus}
              onPrepareVideo={usesRealMediaPipeline ? prepareMissingVideo : undefined}
              transcript={realTranscript}
              rawTranscript={rawTranscript}
              transcriptLoading={transcriptLoading}
              transcriptRefreshing={transcriptRefreshing}
              transcriptError={transcriptError}
              showPosterPreview={!usesRealMediaPipeline}
              autoPlayOnTranscriptClick={autoPlayOnTranscriptClick}
              onSegmentEdited={handleSegmentEdited}
              transcriptPhase={usesRealMediaPipeline ? transcriptPhase : (job.status === "processing" ? "recognizing" : isComplete ? "complete" : "idle")}
              transcriptStatusMessage={job.phase === "translation" || job.phase === "summary" ? job.statusMessage : (asrPhaseMessage || job.statusMessage)}
              asrPhaseProgress={usesRealMediaPipeline ? asrPhaseProgress : null}
            />
          </div>
          {activeTab === "note" && isComplete ? (
            <NoteContent result={realNote} loading={noteLoading} error={noteError} isReal={usesRealMediaPipeline} />
          ) : null}
          {activeTab === "log" ? <LogContent job={job} progress={progress} completed={isComplete} usesRealMediaPipeline={usesRealMediaPipeline} /> : null}
        </div>
      </div>
    </section>
  );
}

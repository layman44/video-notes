import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { initialJobs } from "../data";
import type { TranscriptViewMode } from "./preferences";
import {
  modelKindFromId,
  type AppError,
  type AsrModelStatus,
  type AsrPhaseEvent,
  type AsrPhaseProgress,
  type DataDirectorySettings,
  type Job,
  type MediaPreparationResult,
  type MediaProgress,
  type MediaToolsStatus,
  type ModelDownloadProgress,
  type NoteResult,
  type ReconciledJob,
  type SearchResultItem,
  type SearchResultResponse,
  type SourcePreview,
  type SummaryModelStatus,
  type SummaryProgress,
  type SystemProfile,
  type TranscriptResult,
  type TranslationModelStatus,
  type TranslationProgress,
} from "../types";

export function normalizeAppError(error: unknown): AppError {
  if (typeof error === "object" && error !== null) {
    const errObj = error as Record<string, unknown>;
    if (typeof errObj.message === "string" && errObj.message.trim()) {
      return {
        code: typeof errObj.code === "string" ? errObj.code : "UNKNOWN_ERROR",
        message: errObj.message,
        details: typeof errObj.details === "string" ? errObj.details : undefined,
      };
    }
  }
  if (typeof error === "string" && error.trim()) {
    return {
      code: "UNKNOWN_ERROR",
      message: error,
    };
  }
  if (error instanceof Error && error.message) {
    return {
      code: "UNKNOWN_ERROR",
      message: error.message,
    };
  }
  return {
    code: "UNKNOWN_ERROR",
    message: String(error || "未知错误"),
  };
}

export function formatErrorMessage(error: unknown, fallback = "操作失败"): string {
  if (!error) return fallback;
  const normalized = normalizeAppError(error);
  return normalized.message || fallback;
}

export const isTauri = () => typeof window !== "undefined" && Boolean(window.__TAURI_INTERNALS__);

const wait = (milliseconds: number) =>
  new Promise<void>((resolve) => window.setTimeout(resolve, milliseconds));

const SEARCH_PAGE_SIZE = 10;
const BROWSER_DEMO_SEARCH_TOTAL = 96;

let browserDemoModelInstalled = false;
let browserDemoSummaryModelInstalled = false;
let browserDemoTranslationModelInstalled = false;
let browserDemoDataDirectory = "C:\\Users\\Demo\\AppData\\Roaming\\com.videonotes.desktop";

function browserDemoPlatform(sourceUrl: string) {
  const host = new URL(sourceUrl).hostname.toLowerCase();
  if (host === "douyin.com" || host.endsWith(".douyin.com")) return "douyin" as const;
  if (host === "bilibili.com" || host.endsWith(".bilibili.com") || host === "b23.tv" || host.endsWith(".b23.tv")) {
    return "bilibili" as const;
  }
  throw new Error("未找到受支持的抖音或哔哩哔哩视频链接");
}

export const runtime = {
  isDesktop: isTauri,

  async listJobs(): Promise<Job[]> {
    if (isTauri()) {
      return invoke<Job[]>("list_jobs");
    }

    await wait(120);
    return initialJobs;
  },

  async reconcileJobs(): Promise<ReconciledJob[]> {
    if (isTauri()) {
      return invoke<ReconciledJob[]>("reconcile_jobs");
    }
    return [];
  },

  async parseSource(input: string): Promise<SourcePreview> {
    if (isTauri()) {
      return invoke<SourcePreview>("parse_video_input", { input });
    }

    await wait(650);
    const url = input.match(/https?:\/\/[^\s]+/)?.[0];
    if (!url) {
      throw new Error("未找到有效的视频链接");
    }

    const platform = browserDemoPlatform(url);
    return {
      title: platform === "douyin" ? "短视频内容整理任务" : "从零理解 RAG 的工作原理",
      platform,
      duration: platform === "douyin" ? "08:36" : "28:47",
      sourceUrl: url,
    };
  },

  async searchVideos(
    keyword: string,
    order?: string,
    duration?: number,
    page?: number
  ): Promise<SearchResultResponse> {
    if (isTauri()) {
      return invoke<SearchResultResponse>("search_videos", { keyword, order, duration, page });
    }
    await wait(400);
    const targetPage = Math.max(page ?? 1, 1);
    const firstResultIndex = (targetPage - 1) * SEARCH_PAGE_SIZE;
    const resultCount = Math.max(
      0,
      Math.min(SEARCH_PAGE_SIZE, BROWSER_DEMO_SEARCH_TOTAL - firstResultIndex),
    );
    const demoAuthors = ["科技UP主", "知识充电站", "代码实验室", "AI 学习社", "硬核研究所"];
    const demoCovers = [
      "https://images.unsplash.com/photo-1618005182384-a83a8bd57fbe?w=600",
      "https://images.unsplash.com/photo-1550751827-4bd374c3f58b?w=600",
    ];

    return {
      items: Array.from({ length: resultCount }, (_, index) => {
        const resultNumber = firstResultIndex + index + 1;
        const videoId = `BV1demo${String(resultNumber).padStart(3, "0")}`;
        return {
          id: videoId,
          title:
            index % 2 === 0
              ? `【演示 ${resultNumber}】关于“${keyword}”的深度解析与实战`
              : `【教程 ${resultNumber}】快速掌握 ${keyword} 核心技巧`,
          author: demoAuthors[index % demoAuthors.length],
          platform: "bilibili",
          duration: `${10 + (resultNumber % 20)}:${String((resultNumber * 7) % 60).padStart(2, "0")}`,
          coverUrl: demoCovers[index % demoCovers.length],
          videoUrl: `https://www.bilibili.com/video/${videoId}`,
          playCount: `${(14.8 - (resultNumber % 9) * 0.7).toFixed(1)}万`,
          pubDate: `2025-06-${String(Math.max(1, 28 - (resultNumber % 28))).padStart(2, "0")}`,
        } satisfies SearchResultItem;
      }),
      totalPages: Math.ceil(BROWSER_DEMO_SEARCH_TOTAL / SEARCH_PAGE_SIZE),
      totalCount: BROWSER_DEMO_SEARCH_TOTAL,
      page: targetPage,
    };
  },

  async saveJob(job: Job): Promise<void> {
    if (isTauri()) {
      await invoke("save_job", { job });
    }
  },

  async inspectDataDirectory(): Promise<DataDirectorySettings> {
    if (isTauri()) return invoke<DataDirectorySettings>("inspect_data_directory");
    return {
      currentPath: browserDemoDataDirectory,
      defaultPath: "C:\\Users\\Demo\\AppData\\Roaming\\com.videonotes.desktop",
      isDefault: browserDemoDataDirectory.includes("AppData\\Roaming"),
    };
  },

  async chooseDataDirectory(): Promise<DataDirectorySettings | null> {
    if (isTauri()) return invoke<DataDirectorySettings | null>("choose_data_directory");
    browserDemoDataDirectory = "D:\\VideoNotes";
    return this.inspectDataDirectory();
  },

  async resetDataDirectory(): Promise<DataDirectorySettings> {
    if (isTauri()) return invoke<DataDirectorySettings>("reset_data_directory");
    browserDemoDataDirectory = "C:\\Users\\Demo\\AppData\\Roaming\\com.videonotes.desktop";
    return this.inspectDataDirectory();
  },

  async openTaskDirectory(jobId: string): Promise<void> {
    if (isTauri()) await invoke("open_task_directory", { jobId });
  },

  async resetTaskMedia(jobId: string): Promise<void> {
    if (isTauri()) await invoke("reset_task_media", { jobId });
  },

  async resetTaskTranscript(jobId: string): Promise<void> {
    if (isTauri()) await invoke("reset_task_transcript", { jobId });
  },

  async deleteTask(jobId: string): Promise<void> {
    if (isTauri()) await invoke("delete_task", { jobId });
  },

  async exportTaskAudio(jobId: string, suggestedFilename: string): Promise<string | null> {
    if (isTauri()) {
      return invoke<string | null>("export_task_audio", { jobId, suggestedFilename });
    }
    await wait(160);
    return suggestedFilename;
  },

  async inspectMediaTools(): Promise<MediaToolsStatus> {
    if (isTauri()) {
      return invoke<MediaToolsStatus>("inspect_media_tools");
    }

    return {
      ready: true,
      ytDlp: { name: "yt-dlp", available: true, version: "浏览器演示" },
      ffmpeg: { name: "FFmpeg", available: true, version: "浏览器演示" },
      ffprobe: { name: "ffprobe", available: true, version: "浏览器演示" },
    };
  },

  async prepareMedia(
    jobId: string,
    sourceUrl: string,
    onProgress: (progress: MediaProgress) => void,
  ): Promise<MediaPreparationResult> {
    if (!isTauri()) {
      await wait(800);
      onProgress({ jobId, stage: "download", progress: 100, message: "音频获取完成" });
      await wait(500);
      onProgress({ jobId, stage: "normalize", progress: 100, message: "音频标准化完成" });
      return {
        taskDir: "browser-demo/task",
        sourceFile: "browser-demo/video.mp4",
        durationSeconds: 1727,
        chunks: [{ index: 0, path: "browser-demo/chunk-000.wav", startSeconds: 0, endSeconds: 1727 }],
      };
    }

    const unlisten = await listen<MediaProgress>("media-progress", ({ payload }) => {
      if (payload.jobId === jobId) onProgress(payload);
    });
    try {
      return await invoke<MediaPreparationResult>("prepare_media", { jobId, sourceUrl });
    } finally {
      unlisten();
    }
  },

  async loadMedia(jobId: string): Promise<MediaPreparationResult> {
    if (isTauri()) return invoke<MediaPreparationResult>("load_media", { jobId });
    return {
      taskDir: "browser-demo/task",
      sourceFile: "browser-demo/video.mp4",
      durationSeconds: 1727,
      chunks: [{ index: 0, path: "browser-demo/chunk-000.wav", startSeconds: 0, endSeconds: 1727 }],
    };
  },

  localAssetUrl(path: string): string {
    return isTauri() ? convertFileSrc(path) : path;
  },

  async cancelMedia(jobId: string): Promise<boolean> {
    if (!isTauri()) return true;
    return invoke<boolean>("cancel_media_preparation", { jobId });
  },

  async inspectAsrModel(): Promise<AsrModelStatus> {
    if (isTauri()) {
      return invoke<AsrModelStatus>("inspect_asr_model");
    }
    return {
      id: "funasr-nano",
      name: "Fun-ASR-Nano (GGUF + VAD + CTC + 标点)",
      installed: browserDemoModelInstalled,
      sizeLabel: "约 1.2 GiB",
      path: "browser-demo/models/funasr-nano",
    };
  },

  async inspectMossModel(): Promise<AsrModelStatus> {
    if (isTauri()) return invoke<AsrModelStatus>("inspect_moss_model");
    return {
      id: "moss-transcribe-diarize:q4",
      name: "MOSS-Transcribe-Diarize 0.9B q4（OpenASR）",
      backend: "openasr-moss-q4",
      installed: false,
      sizeLabel: "约 860 MiB",
      path: "browser-demo/models/moss-transcribe-diarize-q4_k.oasr",
    };
  },

  async downloadAsrModel(onProgress: (progress: ModelDownloadProgress) => void): Promise<AsrModelStatus> {
    if (!isTauri()) {
      for (const progress of [8, 24, 51, 78, 100]) {
        await wait(180);
        onProgress({
          modelId: "funasr-nano",
          downloadedBytes: progress,
          totalBytes: 100,
          progress,
          message: progress === 100 ? "语音模型安装完成" : "正在下载语音模型……",
        });
      }
      browserDemoModelInstalled = true;
      return this.inspectAsrModel();
    }
    const unlisten = await listen<ModelDownloadProgress>("model-download-progress", ({ payload }) => {
      if (modelKindFromId(payload.modelId) === "asr") {
        onProgress(payload);
      }
    });
    try {
      return await invoke<AsrModelStatus>("download_asr_model");
    } finally {
      unlisten();
    }
  },

  async deleteAsrModel(): Promise<void> {
    if (!isTauri()) {
      browserDemoModelInstalled = false;
      return;
    }
    await invoke("delete_asr_model");
  },

  async downloadMossModel(onProgress: (progress: ModelDownloadProgress) => void): Promise<AsrModelStatus> {
    if (!isTauri()) return this.inspectMossModel();
    const unlisten = await listen<ModelDownloadProgress>("model-download-progress", ({ payload }) => {
      if (modelKindFromId(payload.modelId) === "moss") onProgress(payload);
    });
    try {
      return await invoke<AsrModelStatus>("download_moss_model");
    } finally {
      unlisten();
    }
  },

  async deleteMossModel(): Promise<void> {
    if (isTauri()) await invoke("delete_moss_model");
  },

  async inspectSummaryModel(): Promise<SummaryModelStatus> {
    if (isTauri()) {
      return invoke<SummaryModelStatus>("inspect_summary_model");
    }
    return {
      id: "qwen3.5-2b-q4_k_m",
      name: "Qwen3.5 2B Q4_K_M (结构化总结)",
      installed: browserDemoSummaryModelInstalled,
      sizeLabel: "约 1.19 GiB",
      path: "browser-demo/models/Qwen3.5-2B-Q4_K_M.gguf",
    };
  },

  async downloadSummaryModel(onProgress: (progress: ModelDownloadProgress) => void): Promise<SummaryModelStatus> {
    if (!isTauri()) {
      for (const progress of [5, 19, 43, 71, 100]) {
        await wait(180);
        onProgress({
          modelId: "qwen3.5-2b-q4_k_m",
          downloadedBytes: progress,
          totalBytes: 100,
          progress,
          message: progress === 100 ? "内容整理模型安装完成" : "正在下载内容整理模型……",
        });
      }
      browserDemoSummaryModelInstalled = true;
      return this.inspectSummaryModel();
    }
    const unlisten = await listen<ModelDownloadProgress>("model-download-progress", ({ payload }) => {
      if (modelKindFromId(payload.modelId) === "summary") {
        onProgress(payload);
      }
    });
    try {
      return await invoke<SummaryModelStatus>("download_summary_model");
    } finally {
      unlisten();
    }
  },

  async deleteSummaryModel(): Promise<void> {
    if (!isTauri()) {
      browserDemoSummaryModelInstalled = false;
      return;
    }
    await invoke("delete_summary_model");
  },

  async inspectTranslationModel(): Promise<TranslationModelStatus> {
    if (isTauri()) {
      return invoke<TranslationModelStatus>("inspect_translation_model");
    }
    return {
      id: "milmmt-46-1b-q4_k_m",
      name: "MiLMMT 46 1B Q4_K_M (极速翻译)",
      installed: browserDemoTranslationModelInstalled,
      sizeLabel: "约 768 MiB",
      path: "browser-demo/models/MiLMMT-46-1B-v1.0.Q4_K_M.gguf",
    };
  },

  async downloadTranslationModel(onProgress: (progress: ModelDownloadProgress) => void): Promise<TranslationModelStatus> {
    if (!isTauri()) {
      for (const progress of [10, 35, 60, 85, 100]) {
        await wait(150);
        onProgress({
          modelId: "milmmt-46-1b-q4_k_m",
          downloadedBytes: progress,
          totalBytes: 100,
          progress,
          message: progress === 100 ? "专用翻译模型安装完成" : "正在下载专用翻译模型……",
        });
      }
      browserDemoTranslationModelInstalled = true;
      return this.inspectTranslationModel();
    }
    const unlisten = await listen<ModelDownloadProgress>("model-download-progress", ({ payload }) => {
      if (modelKindFromId(payload.modelId) === "translation") {
        onProgress(payload);
      }
    });
    try {
      return await invoke<TranslationModelStatus>("download_translation_model");
    } finally {
      unlisten();
    }
  },

  async deleteTranslationModel(): Promise<void> {
    if (!isTauri()) {
      browserDemoTranslationModelInstalled = false;
      return;
    }
    await invoke("delete_translation_model");
  },

  async openModelsDirectory(): Promise<void> {
    if (isTauri()) await invoke("open_models_directory");
  },

  async transcribeMedia(
    jobId: string,
    onPhase: (event: AsrPhaseEvent) => void,
    onProgress: (progress: AsrPhaseProgress) => void,
    resume?: boolean,
  ): Promise<TranscriptResult> {
    if (!isTauri()) {
      await wait(500);
      onPhase({ jobId, phase: "recognition", state: "started", message: "正在识别" });
      onProgress({ jobId, phase: "recognition", completed: 3200, total: 3200, unit: "milliseconds", message: "真实转录已完成" });
      onPhase({ jobId, phase: "recognition", state: "completed", message: "识别完成" });
      return {
        jobId,
        modelId: "qwen3-asr-0.6b",
        language: "zh",
        text: "这是浏览器演示转录。",
        segments: [{ id: "0-0", chunkIndex: 0, startMs: 0, endMs: 3200, text: "这是浏览器演示转录。" }],
      };
    }
    const unlistenPhase = await listen<AsrPhaseEvent>("asr-phase", ({ payload }) => {
      if (payload.jobId === jobId) onPhase(payload);
    });
    const unlistenProgress = await listen<AsrPhaseProgress>("asr-phase-progress", ({ payload }) => {
      if (payload.jobId === jobId) onProgress(payload);
    });
    try {
      return await invoke<TranscriptResult>("transcribe_media", { jobId, resume: Boolean(resume) });
    } finally {
      unlistenPhase();
      unlistenProgress();
    }
  },

  async loadTranscript(jobId: string): Promise<TranscriptResult> {
    if (isTauri()) return invoke<TranscriptResult>("load_transcript", { jobId });
    return {
      jobId,
      modelId: "browser-demo",
      language: "en",
      translationLanguage: "zh",
      text: "Why do we need RAG?\nThe RAG workflow has retrieval and generation stages.\nRetrieval finds the most relevant content.\nNext, we will see how retrieval and generation work together.\nA common mistake is to focus only on the vector database.",
      segments: [
        { id: "0-0", chunkIndex: 0, startMs: 0, endMs: 42_000, text: "Why do we need RAG? Large language models are powerful, but their knowledge can be outdated or incomplete.", translatedText: "为什么我们需要 RAG？大模型虽然很强大，但知识可能存在滞后或缺失。" },
        { id: "0-1", chunkIndex: 0, startMs: 131_000, endMs: 184_000, text: "The RAG workflow can be divided into retrieval and generation stages.", translatedText: "RAG 的整体流程可以拆成检索和生成两个阶段。" },
        { id: "0-2", chunkIndex: 0, startMs: 402_000, endMs: 459_000, text: "The retrieval stage finds the content most relevant to the question.", translatedText: "检索阶段会先找到与问题最相关的内容。" },
        { id: "0-3", chunkIndex: 0, startMs: 624_000, endMs: 688_000, text: "Next, let's see how retrieval and generation work together.", translatedText: "接下来看看检索与生成如何配合。" },
        { id: "0-4", chunkIndex: 0, startMs: 1_095_000, endMs: 1_160_000, text: "A common mistake is focusing only on the vector database while ignoring chunking and retrieval quality.", translatedText: "常见误区是只关注向量数据库，而忽略内容切分和召回质量。" },
      ],
    };
  },

  async loadTranscriptView(jobId: string, view: TranscriptViewMode): Promise<TranscriptResult> {
    if (view === "standard") return this.loadTranscript(jobId);
    if (!isTauri()) {
      const standard = await this.loadTranscript(jobId);
      if (view === "raw") {
        return {
          ...standard,
          translationLanguage: undefined,
          segments: standard.segments.map((segment) => ({ ...segment, translatedText: undefined })),
        };
      }
      return standard;
    }
    try {
      return await invoke<TranscriptResult>("load_transcript_view", { jobId, view });
    } catch (reason) {
      const message = reason instanceof Error ? reason.message : String(reason);
      if (/load_transcript_view|command/i.test(message)) {
        throw new Error("当前后端尚未启用 Raw / Standard 转录视图接口。");
      }
      throw reason;
    }
  },

  async updateTranscriptSegment(jobId: string, segmentId: string, text: string): Promise<void> {
    if (isTauri()) {
      await invoke("update_transcript_segment", { jobId, segmentId, text });
    }
  },

  async organizeNotes(
    job: Pick<Job, "id" | "title" | "sourceUrl" | "platform" | "duration">,
    onProgress: (progress: SummaryProgress) => void,
    force = false,
  ): Promise<NoteResult> {
    if (!isTauri()) {
      await wait(300);
      onProgress({ jobId: job.id, progress: 48, partIndex: 0, partCount: 1, message: "正在整理转录内容" });
      await wait(300);
      onProgress({ jobId: job.id, progress: 100, partIndex: 1, partCount: 1, message: "真实 Markdown 笔记已生成" });
      return this.loadNote(job.id);
    }
    const unlisten = await listen<SummaryProgress>("summary-progress", ({ payload }) => {
      if (payload.jobId === job.id) onProgress(payload);
    });
    try {
      return await invoke<NoteResult>("organize_notes", {
        jobId: job.id,
        title: job.title,
        sourceUrl: job.sourceUrl,
        platform: job.platform,
        duration: job.duration,
        force,
      });
    } finally {
      unlisten();
    }
  },

  async translateTranscript(
    jobId: string,
    onProgress: (progress: TranslationProgress) => void,
  ): Promise<void> {
    if (!isTauri()) {
      onProgress({ jobId, completed: 1, total: 1, message: "翻译完成" });
      return;
    }
    const unlisten = await listen<TranslationProgress>("translation-progress", ({ payload }) => {
      if (payload.jobId === jobId) onProgress(payload);
    });
    try {
      await invoke<void>("translate_transcript", { jobId });
    } finally {
      unlisten();
    }
  },

  async loadNote(jobId: string): Promise<NoteResult> {
    if (isTauri()) return invoke<NoteResult>("load_note", { jobId });
    return {
      jobId,
      modelId: "qwen3-4b-q4_k_m",
      title: "从零理解 RAG 的工作原理",
      sourceUrl: "https://www.bilibili.com/video/BV1RAGDEMO",
      platform: "bilibili",
      duration: "28:47",
      summary: "本视频系统介绍 RAG 的用途、检索与生成的配合方式，以及实现时最容易忽略的质量问题。",
      keyPoints: ["外部检索可以补充模型知识并降低幻觉。", "内容切分和召回质量决定了后续生成上限。", "评估需要兼顾正确性、相关性和可追溯性。"],
      chapters: [
        { timestampMs: 0, title: "为什么需要 RAG", content: "从模型知识时效性与幻觉问题出发，说明引入外部知识的价值。" },
        { timestampMs: 402_000, title: "检索与生成如何配合", content: "讲解内容切分、召回上下文和基于上下文生成答案的完整链路。" },
      ],
      markdown: "# 从零理解 RAG 的工作原理\n\n## 摘要\n\n本视频系统介绍 RAG 的核心流程。\n\n## 完整转录\n\n**[00:00]** 为什么我们需要 RAG？\n\n> 原文：Why do we need RAG?\n",
      transcriptSha256: "browser-demo",
      promptVersion: "notes-v3-bilingual",
    };
  },

  async exportMarkdown(suggestedFilename: string, markdown: string): Promise<string | null> {
    if (isTauri()) {
      return invoke<string | null>("export_markdown", { suggestedFilename, markdown });
    }
    const blob = new Blob([markdown], { type: "text/markdown;charset=utf-8" });
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement("a");
    anchor.href = url;
    anchor.download = suggestedFilename;
    anchor.style.display = "none";
    document.body.appendChild(anchor);
    anchor.click();
    anchor.remove();
    window.setTimeout(() => URL.revokeObjectURL(url), 1_000);
    return suggestedFilename;
  },

  async systemProfile(): Promise<SystemProfile> {
    if (isTauri()) {
      return invoke<SystemProfile>("system_profile");
    }

    return {
      memoryGb: 16,
      logicalCores: navigator.hardwareConcurrency || 8,
      recommendedThreads: Math.max(2, (navigator.hardwareConcurrency || 8) - 2),
      gpuMode: "cpu",
    };
  },
};

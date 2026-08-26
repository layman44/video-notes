import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { modelKindFromId, type AppError, type AsrBackend, type AsrModelStatus, type DataDirectorySettings, type EnqueueSourceInput, type MediaPreparationResult, type MediaToolsStatus, type ModelDownloadProgress, type NoteResult, type QueueItem, type SearchResultResponse, type SourcePreview, type SummaryModelStatus, type SummaryProgress, type TranscriptResult, type TranslationModelStatus, type TranslationProgress, type Video, type VideoPage, type VideoSourceLookup } from "../types";

export function normalizeAppError(error: unknown): AppError {
  if (typeof error === "object" && error !== null) {
    const value = error as Record<string, unknown>;
    if (typeof value.message === "string" && value.message.trim()) return { code: typeof value.code === "string" ? value.code : "UNKNOWN_ERROR", message: value.message, details: typeof value.details === "string" ? value.details : undefined };
  }
  if (typeof error === "string" && error.trim()) return { code: "UNKNOWN_ERROR", message: error };
  if (error instanceof Error && error.message) return { code: "UNKNOWN_ERROR", message: error.message };
  return { code: "UNKNOWN_ERROR", message: String(error || "操作失败") };
}
export function formatErrorMessage(error: unknown, fallback = "操作失败"): string { return error ? normalizeAppError(error).message || fallback : fallback; }
export const isTauri = () => typeof window !== "undefined" && Boolean(window.__TAURI_INTERNALS__);

const unavailable = <T,>(message: string): Promise<T> => Promise.reject(new Error(message));

export const runtime = {
  isDesktop: isTauri,
  async listVideosPage(request: { query?: string; platform?: string; page?: number; pageSize?: number } = {}): Promise<VideoPage> { return isTauri() ? invoke<VideoPage>("list_videos_page", request) : { items: [], total: 0, page: request.page || 1, pageSize: request.pageSize || 20 }; },
  async getVideo(videoId: string): Promise<Video | null> { return isTauri() ? invoke<Video | null>("get_video", { videoId }) : null; },
  async lookupVideosBySources(sources: Array<{ platform: string; sourceUrl: string }>): Promise<VideoSourceLookup[]> { return isTauri() ? invoke<VideoSourceLookup[]>("lookup_videos_by_sources", { sources }) : []; },
  async listQueueItems(): Promise<QueueItem[]> { return isTauri() ? invoke<QueueItem[]>("list_queue_items") : []; },
  async enqueueSources(inputs: EnqueueSourceInput[]): Promise<void> { if (!isTauri()) return unavailable("队列只能在桌面应用中运行"); await invoke("enqueue_sources", { inputs }); },
  async requeueVideo(videoId: string, asrBackend: AsrBackend, asrConfigJson: string): Promise<void> { if (isTauri()) await invoke("requeue_video", { videoId, asrBackend, asrConfigJson }); },
  async pauseQueueItem(id: string): Promise<void> { if (isTauri()) await invoke("pause_queue_item", { id }); },
  async resumeQueueItem(id: string): Promise<void> { if (isTauri()) await invoke("resume_queue_item", { id }); },
  async retryQueueItem(id: string): Promise<void> { if (isTauri()) await invoke("retry_queue_item", { id }); },
  async removeQueueItem(id: string): Promise<void> { if (isTauri()) await invoke("remove_queue_item", { id }); },
  async moveQueueItem(id: string, direction: "up" | "down" | "top"): Promise<void> { if (isTauri()) await invoke("move_queue_item", { id, direction }); },
  async deleteVideoResults(videoId: string): Promise<void> { if (isTauri()) await invoke("delete_video_results", { videoId }); },
  async deleteVideoCompletely(videoId: string): Promise<void> { if (isTauri()) await invoke("delete_video_completely", { videoId }); },
  async updateTranslationSegment(videoId: string, segmentId: string, text: string): Promise<void> { if (isTauri()) await invoke("update_translation_segment", { videoId, segmentId, text }); },
  async parseSource(input: string): Promise<SourcePreview> { return isTauri() ? invoke<SourcePreview>("parse_video_input", { input }) : unavailable("链接解析只能在桌面应用中运行"); },
  async searchVideos(keyword: string, order?: string, duration?: number, page?: number): Promise<SearchResultResponse> { return isTauri() ? invoke<SearchResultResponse>("search_videos", { keyword, order, duration, page }) : unavailable("搜索只能在桌面应用中运行"); },
  async inspectDataDirectory(): Promise<DataDirectorySettings> { return isTauri() ? invoke<DataDirectorySettings>("inspect_data_directory") : unavailable("数据目录只能在桌面应用中查看"); },
  async chooseDataDirectory(): Promise<DataDirectorySettings | null> { return isTauri() ? invoke<DataDirectorySettings | null>("choose_data_directory") : null; },
  async resetDataDirectory(): Promise<DataDirectorySettings> { return isTauri() ? invoke<DataDirectorySettings>("reset_data_directory") : unavailable("数据目录只能在桌面应用中修改"); },
  async exportVideoAudio(videoId: string, suggestedFilename: string): Promise<string | null> { return isTauri() ? invoke<string | null>("export_video_audio", { videoId, suggestedFilename }) : null; },
  async inspectMediaTools(): Promise<MediaToolsStatus> { return isTauri() ? invoke<MediaToolsStatus>("inspect_media_tools") : unavailable("媒体工具只能在桌面应用中检查"); },
  async loadMedia(videoId: string): Promise<MediaPreparationResult> { return isTauri() ? invoke<MediaPreparationResult>("load_video_media", { videoId }) : unavailable("媒体只能在桌面应用中读取"); },
  localAssetUrl(path: string): string { return isTauri() ? convertFileSrc(path) : path; },
  async inspectAsrModel(): Promise<AsrModelStatus> { return isTauri() ? invoke<AsrModelStatus>("inspect_asr_model") : unavailable("模型只能在桌面应用中检查"); },
  async inspectMossModel(): Promise<AsrModelStatus> { return isTauri() ? invoke<AsrModelStatus>("inspect_moss_model") : unavailable("模型只能在桌面应用中检查"); },
  async downloadAsrModel(onProgress: (progress: ModelDownloadProgress) => void): Promise<AsrModelStatus> { return this.downloadModel("download_asr_model", "asr", onProgress); },
  async deleteAsrModel(): Promise<void> { if (isTauri()) await invoke("delete_asr_model"); },
  async downloadMossModel(onProgress: (progress: ModelDownloadProgress) => void): Promise<AsrModelStatus> { return this.downloadModel("download_moss_model", "moss", onProgress); },
  async deleteMossModel(): Promise<void> { if (isTauri()) await invoke("delete_moss_model"); },
  async inspectSummaryModel(): Promise<SummaryModelStatus> { return isTauri() ? invoke<SummaryModelStatus>("inspect_summary_model") : unavailable("模型只能在桌面应用中检查"); },
  async downloadSummaryModel(onProgress: (progress: ModelDownloadProgress) => void): Promise<SummaryModelStatus> { return this.downloadModel("download_summary_model", "summary", onProgress); },
  async deleteSummaryModel(): Promise<void> { if (isTauri()) await invoke("delete_summary_model"); },
  async inspectTranslationModel(): Promise<TranslationModelStatus> { return isTauri() ? invoke<TranslationModelStatus>("inspect_translation_model") : unavailable("模型只能在桌面应用中检查"); },
  async downloadTranslationModel(onProgress: (progress: ModelDownloadProgress) => void): Promise<TranslationModelStatus> { return this.downloadModel("download_translation_model", "translation", onProgress); },
  async deleteTranslationModel(): Promise<void> { if (isTauri()) await invoke("delete_translation_model"); },
  async openModelsDirectory(): Promise<void> { if (isTauri()) await invoke("open_models_directory"); },
  async loadTranscript(videoId: string): Promise<TranscriptResult> { return isTauri() ? invoke<TranscriptResult>("load_video_transcript", { videoId }) : unavailable("转录只能在桌面应用中读取"); },
  async updateTranscriptSegment(videoId: string, segmentId: string, text: string): Promise<void> { if (isTauri()) await invoke("update_video_transcript_segment", { videoId, segmentId, text }); },
  async organizeNotes(video: Pick<Video, "id" | "title" | "sourceUrl" | "platform" | "duration">, onProgress: (progress: SummaryProgress) => void, force = false): Promise<NoteResult> { if (!isTauri()) return unavailable("笔记整理只能在桌面应用中运行"); const unlisten = await listen<SummaryProgress>("summary-progress", ({ payload }) => { if (payload.jobId === video.id) onProgress(payload); }); try { return await invoke<NoteResult>("organize_video_notes", { videoId: video.id, title: video.title, sourceUrl: video.sourceUrl, platform: video.platform, duration: video.duration, force }); } finally { unlisten(); } },
  async translateTranscript(videoId: string, onProgress: (progress: TranslationProgress) => void): Promise<void> { if (!isTauri()) return unavailable("翻译只能在桌面应用中运行"); const unlisten = await listen<TranslationProgress>("translation-progress", ({ payload }) => { if (payload.jobId === videoId) onProgress(payload); }); try { await invoke<void>("translate_video_transcript", { videoId }); } finally { unlisten(); } },
  async loadNote(videoId: string): Promise<NoteResult> { return isTauri() ? invoke<NoteResult>("load_video_note", { videoId }) : unavailable("笔记只能在桌面应用中读取"); },
  async exportMarkdown(suggestedFilename: string, markdown: string): Promise<string | null> { return isTauri() ? invoke<string | null>("export_markdown", { suggestedFilename, markdown }) : null; },
  async downloadModel(command: string, kind: string, onProgress: (progress: ModelDownloadProgress) => void): Promise<AsrModelStatus> { if (!isTauri()) return unavailable("模型只能在桌面应用中下载"); const unlisten = await listen<ModelDownloadProgress>("model-download-progress", ({ payload }) => { if (modelKindFromId(payload.modelId) === kind) onProgress(payload); }); try { return await invoke<AsrModelStatus>(command); } finally { unlisten(); } },
};

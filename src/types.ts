export type PageId = "home" | "search" | "queue" | "library" | "models" | "settings" | "video-detail";
export type Platform = "bilibili" | "douyin";
export interface SearchResultItem { id: string; title: string; author: string; platform: Platform | string; duration: string; coverUrl?: string | null; videoUrl: string; playCount?: string | null; pubDate?: string | null; }
export interface SearchResultResponse { items: SearchResultItem[]; totalPages: number; totalCount: number; page: number; }
export type SearchOrder = "totalrank" | "click" | "pubdate" | "stow" | "dm";
export type SearchDurationFilter = 0 | 1 | 2 | 3 | 4;

export type QueueState = "queued" | "running" | "paused" | "blocked" | "failed" | "completed" | "cancelled";
export type QueueStage = "download" | "normalize" | "transcribe";
export type ArtifactState = "unknown" | "missing" | "ready" | "processing" | "stale" | "failed";
export interface QueueItem { id: string; videoId: string; title: string; platform: Platform | string; duration: string; sourceUrl: string; author?: string; thumbnailUrl?: string | null; position: number; state: QueueState; stage?: QueueStage | null; progress?: number | null; progressCompleted?: number | null; progressTotal?: number | null; progressUnit?: string | null; attemptCount: number; statusMessage?: string | null; errorCode?: string | null; errorMessage?: string | null; createdAt?: string | null; updatedAt?: string | null; }
export interface Video { id: string; title: string; platform: Platform | string; duration: string; sourceUrl: string; author?: string; thumbnailUrl?: string | null; updatedAt?: string | null; createdAt?: string | null; transcriptStatus: ArtifactState; translationStatus: ArtifactState; noteStatus: ArtifactState; mediaStatus: "unknown" | "available" | "missing" | "deleted"; transcriptLanguage?: string | null; queueItemId?: string | null; }
export interface VideoPage { items: Video[]; total: number; page: number; pageSize: number; }
export interface VideoSourceLookup { platform: Platform | string; sourceUrl: string; video?: Video | null; }
export interface EnqueueSourceInput { title: string; platform: Platform | string; duration: string; sourceUrl: string; author?: string; thumbnailUrl?: string | null; asrBackend?: AsrBackend; asrConfigJson?: string; }
export interface AppError { code: string; message: string; details?: string; }
export interface SourcePreview extends EnqueueSourceInput {}

export type AsrBackend = "funasr-nano" | "openasr-moss-q4";
export const isMossBackend = (backend?: AsrBackend | string | null): boolean => backend === "openasr-moss-q4" || (typeof backend === "string" && (backend.startsWith("moss") || backend.startsWith("openasr")));
export function modelKindFromId(modelId: string): "asr" | "moss" | "translation" | "summary" { if (modelId.startsWith("moss") || modelId.startsWith("openasr")) return "moss"; if (modelId.startsWith("milmmt") || modelId.startsWith("translation")) return "translation"; if (modelId.startsWith("qwen") || modelId.startsWith("summary")) return "summary"; return "asr"; }
export interface MossAsrConfig { chunkSeconds: number; overlapSeconds: number; }
export interface AsrSettings { backend: AsrBackend; moss: MossAsrConfig; }
export interface TranscriptSegment { id: string; chunkIndex?: number; start?: number; end?: number; startMs: number; endMs: number; text: string; translatedText?: string; avgConfidence?: number; }
export interface TranscriptResult { jobId: string; modelId: string; language: string; translationLanguage?: string; text: string; segments: TranscriptSegment[]; pauseRepairs?: PauseBoundaryRepair[]; }
export interface PauseBoundaryRepair { boundaryOffset: number; removePunctuationOffset?: number | null; time: number; gap: number; confidence: number; context: string; }
export interface TranslationProgress { jobId: string; completed: number; total: number; message: string; }
export interface SummaryProgress { jobId: string; progress: number; partIndex: number; partCount: number; message: string; }
export interface NoteChapter { timestampMs: number; title: string; content: string; }
export interface NoteResult { jobId: string; modelId: string; title: string; sourceUrl: string; platform: string; duration: string; summary: string; keyPoints: string[]; chapters: NoteChapter[]; markdown: string; transcriptSha256: string; promptVersion: string; }
export interface AudioChunk { index: number; path: string; startSeconds: number; endSeconds: number; }
export interface MediaPreparationResult { taskDir: string; sourceFile: string; videoFile?: string; thumbnailFile?: string; metadataFile?: string; durationSeconds: number; chunks: AudioChunk[]; }
export interface AsrModelStatus { id: string; name: string; backend?: AsrBackend; installed: boolean; fileSize?: number; sizeLabel: string; path: string; }
export type SummaryModelStatus = AsrModelStatus;
export type TranslationModelStatus = AsrModelStatus;
export interface ModelReadiness { asr: boolean; summary: boolean; translation: boolean; }
export interface ModelDownloadProgress { modelId: string; downloadedBytes: number; totalBytes?: number; progress: number; message: string; }
export interface MediaToolStatus { name: string; available: boolean; path?: string; version?: string; }
export interface MediaToolsStatus { ready: boolean; ytDlp: MediaToolStatus; ffmpeg: MediaToolStatus; ffprobe: MediaToolStatus; }
export interface DataDirectorySettings { currentPath: string; defaultPath: string; isDefault: boolean; }

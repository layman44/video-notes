export type PageId = "home" | "tasks" | "models" | "settings" | "task-detail";

export type JobStatus = "completed" | "transcribed" | "processing" | "waiting" | "paused" | "failed";

export type Platform = "bilibili" | "douyin";

export type JobPhase =
  | "media_download"
  | "media_normalize"
  | "recognition"
  | "pause_alignment"
  | "verification"
  | "boundary_review"
  | "word_alignment"
  | "standardization"
  | "semantic_segmentation"
  | "translation"
  | "summary";

export interface Job {
  id: string;
  title: string;
  platform: Platform;
  duration: string;
  updatedAt: string;
  status: JobStatus;
  /** Legacy compatibility field. New workflow must not infer phase from this value. */
  progress: number;
  phase?: JobPhase;
  phaseCompleted?: number;
  phaseTotal?: number;
  phaseUnit?: string;
  sourceUrl: string;
  thumbnailUrl?: string;
  errorMessage?: string;
  statusMessage?: string;
}

export interface ReconciledJob {
  id: string;
  status: JobStatus;
  progress: number;
  phase?: JobPhase;
  phaseCompleted?: number;
  phaseTotal?: number;
  phaseUnit?: string;
  statusMessage?: string;
  errorMessage?: string;
}

export interface SourcePreview {
  title: string;
  platform: Platform;
  duration: string;
  sourceUrl: string;
  author?: string;
  thumbnailUrl?: string;
}

export interface MediaToolStatus {
  name: string;
  available: boolean;
  path?: string;
  version?: string;
}

export interface MediaToolsStatus {
  ready: boolean;
  ytDlp: MediaToolStatus;
  ffmpeg: MediaToolStatus;
  ffprobe: MediaToolStatus;
}

export interface DataDirectorySettings {
  currentPath: string;
  defaultPath: string;
  isDefault: boolean;
}

export interface MediaProgress {
  jobId: string;
  stage: "download" | "normalize" | "ready";
  progress: number;
  message: string;
}

export interface AsrPhaseProgress {
  jobId: string;
  phase: AsrPipelinePhase;
  completed: number;
  total?: number;
  unit: string;
  message: string;
}

export type AsrPipelinePhase =
  | "recognition"
  | "pause_alignment"
  | "verification"
  | "boundary_review"
  | "word_alignment"
  | "standardization"
  | "semantic_segmentation";

export interface AsrPhaseEvent {
  jobId: string;
  phase: AsrPipelinePhase;
  state: "started" | "completed";
  message: string;
}

export interface AsrSegment {
  jobId: string;
  chunkIndex: number;
  startMs: number;
  endMs: number;
  text: string;
}

export interface AsrSnapshot {
  jobId: string;
  segments: TranscriptSegment[];
  language?: string;
  processedUntil: number;
  pauseRepairs?: PauseBoundaryRepair[];
  /** Optional in newer backends: identifies whether snapshot segments are raw or canonical-standard. */
  view?: "raw" | "standard";
  /** True while the backend may still revise boundaries/timestamps as new evidence arrives. */
  provisional?: boolean;
}

export interface AsrModelStatus {
  id: string;
  name: string;
  installed: boolean;
  fileSize?: number;
  sizeLabel: string;
  path: string;
}

export type SummaryModelStatus = AsrModelStatus;
export type TranslationModelStatus = AsrModelStatus;

export interface ModelReadiness {
  asr: boolean;
  summary: boolean;
  translation: boolean;
}

export interface ModelDownloadProgress {
  modelId: string;
  downloadedBytes: number;
  totalBytes?: number;
  progress: number;
  message: string;
}

export interface TranscriptSegment {
  id: string;
  chunkIndex?: number;
  start?: number;
  end?: number;
  startMs: number;
  endMs: number;
  text: string;
  translatedText?: string;
  avgConfidence?: number;
}

export interface PauseBoundaryRepair {
  boundaryOffset: number;
  removePunctuationOffset?: number | null;
  time: number;
  gap: number;
  confidence: number;
  context: string;
}

export interface TranscriptResult {
  jobId: string;
  modelId: string;
  language: string;
  translationLanguage?: string;
  text: string;
  segments: TranscriptSegment[];
  pauseRepairs?: PauseBoundaryRepair[];
}

export interface TranslationSegmentUpdate {
  jobId: string;
  segmentId: string;
  translatedText: string;
}

export interface TranslationProgress {
  jobId: string;
  completed: number;
  total: number;
  message: string;
}

export interface SummaryProgress {
  jobId: string;
  progress: number;
  partIndex: number;
  partCount: number;
  message: string;
}

export interface NoteChapter {
  timestampMs: number;
  title: string;
  content: string;
}

export interface NoteResult {
  jobId: string;
  modelId: string;
  title: string;
  sourceUrl: string;
  platform: string;
  duration: string;
  summary: string;
  keyPoints: string[];
  chapters: NoteChapter[];
  markdown: string;
  transcriptSha256: string;
  promptVersion: string;
}

export interface AudioChunk {
  index: number;
  path: string;
  startSeconds: number;
  endSeconds: number;
}

export interface MediaPreparationResult {
  taskDir: string;
  sourceFile: string;
  videoFile?: string;
  thumbnailFile?: string;
  metadataFile?: string;
  durationSeconds: number;
  chunks: AudioChunk[];
}

export interface SystemProfile {
  memoryGb: number;
  logicalCores: number;
  recommendedThreads: number;
  gpuMode: "cpu";
}

export interface ProcessingStep {
  id: "download" | "transcribe" | "review" | "summarize";
  label: string;
  detail: string;
  state: "pending" | "active" | "completed";
}

export type TaskTab = "workspace" | "note" | "log";

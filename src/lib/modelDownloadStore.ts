import { listen } from "@tauri-apps/api/event";
import { isTauri, runtime } from "./runtime";
import type { AsrModelStatus, ModelDownloadProgress, ModelReadiness, SummaryModelStatus, TranslationModelStatus } from "../types";

export type ModelKind = "asr" | "summary" | "translation";

export interface ModelDownloadState {
  downloadingKind: ModelKind | null;
  progress: Record<ModelKind, number>;
  message: Record<ModelKind, string>;
  error: Record<ModelKind, string>;
  asrModel: AsrModelStatus | null;
  summaryModel: SummaryModelStatus | null;
  translationModel: TranslationModelStatus | null;
}

type Listener = () => void;

class ModelDownloadStore {
  private state: ModelDownloadState = {
    downloadingKind: null,
    progress: { asr: 0, summary: 0, translation: 0 },
    message: { asr: "", summary: "", translation: "" },
    error: { asr: "", summary: "", translation: "" },
    asrModel: null,
    summaryModel: null,
    translationModel: null,
  };

  private listeners = new Set<Listener>();
  private activeDownloadPromise: Record<ModelKind, Promise<unknown> | null> = { asr: null, summary: null, translation: null };
  private initialized = false;

  constructor() {
    this.initGlobalListener();
  }

  private initGlobalListener() {
    if (!isTauri() || this.initialized) return;
    this.initialized = true;

    void listen<ModelDownloadProgress>("model-download-progress", ({ payload }) => {
      const isAsr =
        payload.modelId === "funasr-nano" ||
        payload.modelId.includes("funasr") ||
        payload.modelId.includes("nano");
      const isTranslation =
        payload.modelId.includes("milmmt") ||
        payload.modelId.includes("translation");
      const isSummary =
        payload.modelId.includes("qwen") ||
        payload.modelId.includes("summary");
      const kind: ModelKind = isAsr ? "asr" : isTranslation ? "translation" : isSummary ? "summary" : "asr";

      this.update((draft) => {
        draft.progress[kind] = payload.progress;
        draft.message[kind] = payload.message;
        if (payload.progress < 100) {
          draft.downloadingKind = kind;
        } else {
          if (draft.downloadingKind === kind) draft.downloadingKind = null;
        }
      });
    });
  }

  public getState(): ModelDownloadState {
    return this.state;
  }

  public subscribe(listener: Listener): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  private update(updater: (draft: ModelDownloadState) => void) {
    const next = {
      ...this.state,
      progress: { ...this.state.progress },
      message: { ...this.state.message },
      error: { ...this.state.error },
    };
    updater(next);
    this.state = next;
    for (const listener of this.listeners) {
      listener();
    }
  }

  public async refresh(onStatusChange?: (readiness: ModelReadiness) => void) {
    try {
      const [asr, summary, translation] = await Promise.all([
        runtime.inspectAsrModel(),
        runtime.inspectSummaryModel(),
        runtime.inspectTranslationModel(),
      ]);
      this.update((draft) => {
        draft.asrModel = asr;
        draft.summaryModel = summary;
        draft.translationModel = translation;
      });
      onStatusChange?.({ asr: asr.installed, summary: summary.installed, translation: translation.installed });
      return { asr, summary, translation };
    } catch (reason) {
      const text = reason instanceof Error ? reason.message : String(reason);
      this.update((draft) => {
        draft.error.asr = text;
        draft.error.summary = text;
        draft.error.translation = text;
      });
    }
  }

  public async startDownload(kind: ModelKind, onStatusChange?: (readiness: ModelReadiness) => void) {
    if (this.activeDownloadPromise[kind]) {
      return this.activeDownloadPromise[kind];
    }

    this.update((draft) => {
      draft.downloadingKind = kind;
      draft.error[kind] = "";
      draft.progress[kind] = 0;
      draft.message[kind] = "正在准备下载……";
    });

    const onProgress = (update: { progress: number; message: string }) => {
      this.update((draft) => {
        draft.progress[kind] = update.progress;
        draft.message[kind] = update.message;
      });
    };

    const task = (async () => {
      try {
        if (kind === "asr") {
          await runtime.downloadAsrModel(onProgress);
        } else if (kind === "translation") {
          await runtime.downloadTranslationModel(onProgress);
        } else {
          await runtime.downloadSummaryModel(onProgress);
        }
        await this.refresh(onStatusChange);
      } catch (reason) {
        const text = reason instanceof Error ? reason.message : String(reason);
        if (text.includes("已经在下载中") || text.includes("正在下载中")) {
          this.update((draft) => {
            draft.downloadingKind = kind;
            draft.error[kind] = "";
          });
        } else {
          this.update((draft) => {
            draft.error[kind] = text;
            draft.downloadingKind = null;
          });
        }
      } finally {
        this.activeDownloadPromise[kind] = null;
        if (this.state.downloadingKind === kind && this.state.progress[kind] >= 100) {
          this.update((draft) => {
            draft.downloadingKind = null;
          });
        }
      }
    })();

    this.activeDownloadPromise[kind] = task;
    return task;
  }

  public async removeModel(kind: ModelKind, onStatusChange?: (readiness: ModelReadiness) => void) {
    this.update((draft) => {
      draft.error[kind] = "";
    });
    try {
      if (kind === "asr") await runtime.deleteAsrModel();
      else if (kind === "translation") await runtime.deleteTranslationModel();
      else await runtime.deleteSummaryModel();
      await this.refresh(onStatusChange);
      this.update((draft) => {
        draft.progress[kind] = 0;
        draft.message[kind] = "";
      });
    } catch (reason) {
      const text = reason instanceof Error ? reason.message : String(reason);
      this.update((draft) => {
        draft.error[kind] = text;
      });
    }
  }
}

export const modelDownloadStore = new ModelDownloadStore();

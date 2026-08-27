import { listen } from "@tauri-apps/api/event";
import { isTauri, normalizeAppError, runtime } from "./runtime";
import { modelKindFromId, type AsrModelStatus, type EmbeddingModelStatus, type ModelDownloadProgress, type ModelReadiness, type SummaryModelStatus, type TranslationModelStatus } from "../types";

export type ModelKind = "asr" | "moss" | "summary" | "translation" | "embedding";

export interface ModelDownloadState {
  downloadingKind: ModelKind | null;
  progress: Record<ModelKind, number>;
  message: Record<ModelKind, string>;
  error: Record<ModelKind, string>;
  asrModel: AsrModelStatus | null;
  mossModel: AsrModelStatus | null;
  summaryModel: SummaryModelStatus | null;
  translationModel: TranslationModelStatus | null;
  embeddingModel: EmbeddingModelStatus | null;
}

type Listener = () => void;

class ModelDownloadStore {
  private state: ModelDownloadState = {
    downloadingKind: null,
    progress: { asr: 0, moss: 0, summary: 0, translation: 0, embedding: 0 },
    message: { asr: "", moss: "", summary: "", translation: "", embedding: "" },
    error: { asr: "", moss: "", summary: "", translation: "", embedding: "" },
    asrModel: null,
    mossModel: null,
    summaryModel: null,
    translationModel: null,
    embeddingModel: null,
  };

  private listeners = new Set<Listener>();
  private activeDownloadPromise: Record<ModelKind, Promise<unknown> | null> = { asr: null, moss: null, summary: null, translation: null, embedding: null };
  private initialized = false;

  constructor() {
    this.initGlobalListener();
  }

  private initGlobalListener() {
    if (!isTauri() || this.initialized) return;
    this.initialized = true;

    void listen<ModelDownloadProgress>("model-download-progress", ({ payload }) => {
      const kind: ModelKind = modelKindFromId(payload.modelId);

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
      const [asr, moss, summary, translation, embedding] = await Promise.all([
        runtime.inspectAsrModel(),
        runtime.inspectMossModel(),
        runtime.inspectSummaryModel(),
        runtime.inspectTranslationModel(),
        runtime.inspectEmbeddingModel(),
      ]);
      this.update((draft) => {
        draft.asrModel = asr;
        draft.mossModel = moss;
        draft.summaryModel = summary;
        draft.translationModel = translation;
        draft.embeddingModel = embedding;
      });
      onStatusChange?.({
        asr: asr.installed || moss.installed,
        summary: summary.installed,
        translation: translation.installed,
        embedding: embedding.installed,
      });
      return { asr, moss, summary, translation, embedding };
    } catch (reason) {
      const text = reason instanceof Error ? reason.message : String(reason);
      this.update((draft) => {
        draft.error.asr = text;
        draft.error.summary = text;
        draft.error.translation = text;
        draft.error.embedding = text;
      });
    }
  }

  public async startDownload(kind: ModelKind, onStatusChange?: (readiness: ModelReadiness) => void) {
    if (this.activeDownloadPromise[kind]) {
      return this.activeDownloadPromise[kind];
    }

    console.log("[modelDownloadStore] startDownload invoked for kind:", kind);
    this.update((draft) => {
      draft.downloadingKind = kind;
      draft.error[kind] = "";
      draft.progress[kind] = 0;
      draft.message[kind] = "正在准备下载……";
    });

    const onProgress = (update: { progress: number; message: string }) => {
      console.log(`[modelDownloadStore] progress event for ${kind}:`, update);
      this.update((draft) => {
        draft.progress[kind] = update.progress;
        draft.message[kind] = update.message;
      });
    };

    const task = (async () => {
      try {
        console.log(`[modelDownloadStore] invoking runtime download for ${kind}...`);
        if (kind === "asr") {
          await runtime.downloadAsrModel(onProgress);
        } else if (kind === "moss") {
          await runtime.downloadMossModel(onProgress);
        } else if (kind === "translation") {
          await runtime.downloadTranslationModel(onProgress);
        } else if (kind === "embedding") {
          await runtime.downloadEmbeddingModel(onProgress);
        } else {
          await runtime.downloadSummaryModel(onProgress);
        }
        console.log(`[modelDownloadStore] download completed for ${kind}, refreshing store...`);
        await this.refresh(onStatusChange);
      } catch (reason) {
        console.error(`[modelDownloadStore] download failed for ${kind}:`, reason);
        const err = normalizeAppError(reason);
        if (err.code === "ALREADY_DOWNLOADING") {
          this.update((draft) => {
            draft.downloadingKind = kind;
            draft.error[kind] = "";
          });
        } else {
          this.update((draft) => {
            draft.error[kind] = err.message;
            draft.downloadingKind = null;
          });
        }
      } finally {
        this.activeDownloadPromise[kind] = null;
        this.update((draft) => {
          if (draft.downloadingKind === kind) {
            draft.downloadingKind = null;
          }
        });
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
      else if (kind === "moss") await runtime.deleteMossModel();
      else if (kind === "translation") await runtime.deleteTranslationModel();
      else if (kind === "embedding") await runtime.deleteEmbeddingModel();
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

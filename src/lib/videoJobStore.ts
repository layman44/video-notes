import { listen } from "@tauri-apps/api/event";
import { isTauri } from "./runtime";
import { invoke } from "@tauri-apps/api/core";
import type { NoteResult, SummaryProgress, TranslationProgress, Video } from "../types";

export type VideoActionKind = "organize" | "translate";

export interface VideoActiveJob {
  action: VideoActionKind;
  message: string;
  progress?: number;
}

type Listener = () => void;

class VideoJobStore {
  private jobs: Record<string, VideoActiveJob> = {};
  private listeners = new Set<Listener>();
  private activePromises = new Map<string, Promise<unknown>>();
  private initialized = false;

  constructor() {
    this.initGlobalListeners();
  }

  private initGlobalListeners() {
    if (!isTauri() || this.initialized) return;
    this.initialized = true;

    void listen<SummaryProgress>("summary-progress", ({ payload }) => {
      if (this.jobs[payload.jobId]?.action === "organize") {
        this.jobs = {
          ...this.jobs,
          [payload.jobId]: {
            action: "organize",
            message: payload.message,
            progress: payload.progress,
          },
        };
        this.emit();
      }
    });

    void listen<TranslationProgress>("translation-progress", ({ payload }) => {
      if (this.jobs[payload.jobId]?.action === "translate") {
        const percent = payload.total > 0 ? Math.round((payload.completed / payload.total) * 100) : undefined;
        this.jobs = {
          ...this.jobs,
          [payload.jobId]: {
            action: "translate",
            message: payload.message,
            progress: percent,
          },
        };
        this.emit();
      }
    });
  }

  public subscribe = (listener: Listener): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  public getJob(videoId: string): VideoActiveJob | undefined {
    return this.jobs[videoId];
  }

  public isBusy(videoId: string): boolean {
    return Boolean(this.jobs[videoId]);
  }

  private emit() {
    for (const listener of this.listeners) {
      listener();
    }
  }

  public async startOrganize(
    video: Pick<Video, "id" | "title" | "sourceUrl" | "platform" | "duration">,
    force = false,
  ): Promise<NoteResult> {
    const existing = this.activePromises.get(video.id);
    if (existing) {
      return existing as Promise<NoteResult>;
    }

    this.jobs = {
      ...this.jobs,
      [video.id]: {
        action: "organize",
        message: "正在准备生成总结……",
      },
    };
    this.emit();

    const promise = (async () => {
      try {
        const result = await invoke<NoteResult>("organize_video_notes", {
          videoId: video.id,
          title: video.title,
          sourceUrl: video.sourceUrl,
          platform: video.platform,
          duration: video.duration,
          force,
        });
        return result;
      } finally {
        this.activePromises.delete(video.id);
        const { [video.id]: _, ...rest } = this.jobs;
        this.jobs = rest;
        this.emit();
      }
    })();

    this.activePromises.set(video.id, promise);
    return promise;
  }

  public async startTranslate(videoId: string, force = false): Promise<void> {
    const existing = this.activePromises.get(videoId);
    if (existing) {
      return existing as Promise<void>;
    }

    this.jobs = {
      ...this.jobs,
      [videoId]: {
        action: "translate",
        message: "正在准备翻译……",
      },
    };
    this.emit();

    const promise = (async () => {
      try {
        await invoke<void>("translate_video_transcript", { videoId, force });
      } finally {
        this.activePromises.delete(videoId);
        const { [videoId]: _, ...rest } = this.jobs;
        this.jobs = rest;
        this.emit();
      }
    })();

    this.activePromises.set(videoId, promise);
    return promise;
  }
}

export const videoJobStore = new VideoJobStore();

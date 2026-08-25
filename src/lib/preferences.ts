export interface PlaybackPreferences {
  autoPlayOnTranscriptClick: boolean;
}

const STORAGE_KEY = "videonotes.playback-preferences.v1";

const defaultPreferences: PlaybackPreferences = {
  autoPlayOnTranscriptClick: false,
};

export function loadPlaybackPreferences(): PlaybackPreferences {
  if (typeof window === "undefined") return defaultPreferences;

  try {
    const stored = window.localStorage.getItem(STORAGE_KEY);
    if (!stored) return defaultPreferences;

    const parsed = JSON.parse(stored) as Partial<PlaybackPreferences>;
    return {
      autoPlayOnTranscriptClick: parsed.autoPlayOnTranscriptClick === true,
    };
  } catch {
    return defaultPreferences;
  }
}

export function savePlaybackPreferences(preferences: PlaybackPreferences) {
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(preferences));
  } catch {
    // The preference still applies for the current session when storage is unavailable.
  }
}

import type { AsrBackend, AsrSettings, MossAsrConfig } from "../types";

const ASR_SETTINGS_KEY = "videonotes.asr-settings.v1";
const defaultAsrSettings: AsrSettings = {
  backend: "funasr-nano",
  moss: { chunkSeconds: 30, overlapSeconds: 1 },
};

export function loadAsrSettings(): AsrSettings {
  if (typeof window === "undefined") return defaultAsrSettings;
  try {
    const parsed = JSON.parse(window.localStorage.getItem(ASR_SETTINGS_KEY) ?? "null") as Partial<AsrSettings> | null;
    const backend: AsrBackend = parsed?.backend === "openasr-moss-q4" ? "openasr-moss-q4" : "funasr-nano";
    const moss: MossAsrConfig = {
      chunkSeconds: Number.isFinite(parsed?.moss?.chunkSeconds) ? Math.min(120, Math.max(15, Number(parsed?.moss?.chunkSeconds))) : 30,
      overlapSeconds: Number.isFinite(parsed?.moss?.overlapSeconds) ? Math.min(5, Math.max(0, Number(parsed?.moss?.overlapSeconds))) : 1,
    };
    if (moss.overlapSeconds >= moss.chunkSeconds) moss.overlapSeconds = Math.min(1, moss.chunkSeconds / 3);
    return { backend, moss };
  } catch {
    return defaultAsrSettings;
  }
}

export function saveAsrSettings(settings: AsrSettings) {
  try {
    window.localStorage.setItem(ASR_SETTINGS_KEY, JSON.stringify(settings));
  } catch {
    // The preference still applies for the current session when storage is unavailable.
  }
}

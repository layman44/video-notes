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

/** Language rendering is independent from transcript quality/view selection. */
export type TranscriptDisplayMode = "translated" | "bilingual" | "original";

const TRANSCRIPT_DISPLAY_KEY = "videonotes.transcript-display.v1";

export function loadTranscriptDisplayMode(): TranscriptDisplayMode {
  if (typeof window === "undefined") return "translated";
  try {
    const stored = window.localStorage.getItem(TRANSCRIPT_DISPLAY_KEY);
    return stored === "bilingual" || stored === "original" ? stored : "translated";
  } catch {
    return "translated";
  }
}

export function saveTranscriptDisplayMode(mode: TranscriptDisplayMode) {
  try {
    window.localStorage.setItem(TRANSCRIPT_DISPLAY_KEY, mode);
  } catch {
    // The selected mode still applies for the current session when storage is unavailable.
  }
}

/**
 * Transcript fact/readability view. This is deliberately orthogonal to
 * TranscriptDisplayMode: e.g. `standard + bilingual` and `smooth + original`
 * are both valid combinations.
 */
export type TranscriptViewMode = "raw" | "standard" | "smooth";

const TRANSCRIPT_VIEW_KEY = "videonotes.transcript-view.v1";

export function loadTranscriptViewMode(): TranscriptViewMode {
  if (typeof window === "undefined") return "standard";
  try {
    const stored = window.localStorage.getItem(TRANSCRIPT_VIEW_KEY);
    return stored === "raw" || stored === "smooth" ? stored : "standard";
  } catch {
    return "standard";
  }
}

export function saveTranscriptViewMode(mode: TranscriptViewMode) {
  try {
    window.localStorage.setItem(TRANSCRIPT_VIEW_KEY, mode);
  } catch {
    // The selected mode still applies for the current session when storage is unavailable.
  }
}

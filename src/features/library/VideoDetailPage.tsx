import { ArrowLeft, ChevronDown, ChevronUp, Download, FileText, Languages, LayoutList, List, LoaderCircle, LocateFixed, Maximize2, Minimize2, Pause, Pencil, Play, PlayCircle, RefreshCw, RotateCcw, Save, Search, Sparkles, Trash2, Volume1, Volume2, VolumeX, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore, type MouseEvent } from "react";
import { listen } from "@tauri-apps/api/event";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import fallbackThumbnailUrl from "../../assets/rag-thumbnail.png";
import { isChineseLanguage, isChineseText } from "../../lib/language";
import { formatErrorMessage, runtime } from "../../lib/runtime";
import { toast } from "../../lib/toast";
import { videoJobStore } from "../../lib/videoJobStore";
import type { NoteResult, SemanticSearchResult, TranscriptResult, TranscriptSegment, Video } from "../../types";

interface VideoDetailPageProps { video: Video; onBack: () => void; onRefresh: () => Promise<void>; autoPlayOnTranscriptClick: boolean; }

function time(ms: number) {
  const total = Math.floor(Math.max(0, ms) / 1000);
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const seconds = total % 60;
  return hours ? hours + ":" + String(minutes).padStart(2, "0") + ":" + String(seconds).padStart(2, "0") : String(minutes).padStart(2, "0") + ":" + String(seconds).padStart(2, "0");
}

function parseTimeToMs(value: string): number | null {
  const trimmed = value.trim();
  if (!trimmed) return null;
  const parts = trimmed.split(":").map((p) => p.trim());
  if (parts.length === 2) {
    const minutes = Number(parts[0]);
    const seconds = Number(parts[1]);
    if (!Number.isFinite(minutes) || !Number.isFinite(seconds) || minutes < 0 || seconds < 0 || seconds >= 60) return null;
    return Math.round((minutes * 60 + seconds) * 1000);
  } else if (parts.length === 3) {
    const hours = Number(parts[0]);
    const minutes = Number(parts[1]);
    const seconds = Number(parts[2]);
    if (!Number.isFinite(hours) || !Number.isFinite(minutes) || !Number.isFinite(seconds) || hours < 0 || minutes < 0 || minutes >= 60 || seconds < 0 || seconds >= 60) return null;
    return Math.round((hours * 3600 + minutes * 60 + seconds) * 1000);
  } else if (parts.length === 1) {
    const seconds = Number(parts[0]);
    if (!Number.isFinite(seconds) || seconds < 0) return null;
    return Math.round(seconds * 1000);
  }
  return null;
}

function segmentAt(segments: TranscriptSegment[], currentMs: number) {
  let low = 0;
  let high = segments.length;
  while (low < high) {
    const middle = Math.floor((low + high) / 2);
    if (segments[middle].startMs <= currentMs) low = middle + 1;
    else high = middle;
  }
  return low > 0 ? segments[low - 1] : null;
}

function EditRow({
  segment,
  bilingual,
  onSave,
  onDelete,
}: {
  segment: TranscriptSegment;
  bilingual: boolean;
  onSave: (original: string, translation: string, bilingual: boolean, startMs?: number, endMs?: number) => Promise<void>;
  onDelete: (segment: TranscriptSegment) => void;
}) {
  const [editing, setEditing] = useState(false);
  const [original, setOriginal] = useState(segment.text);
  const [translation, setTranslation] = useState(segment.translatedText || "");
  const [startTimeStr, setStartTimeStr] = useState(time(segment.startMs));
  const [endTimeStr, setEndTimeStr] = useState(time(segment.endMs));
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    if (!editing) {
      setOriginal(segment.text);
      setTranslation(segment.translatedText || "");
      setStartTimeStr(time(segment.startMs));
      setEndTimeStr(time(segment.endMs));
    }
  }, [editing, segment.text, segment.translatedText, segment.startMs, segment.endMs]);

  const beginEdit = (event: MouseEvent<HTMLButtonElement>) => {
    event.stopPropagation();
    setError("");
    setEditing(true);
  };

  const cancel = (event: MouseEvent<HTMLButtonElement>) => {
    event.stopPropagation();
    setEditing(false);
  };

  const handleDelete = (event: MouseEvent<HTMLButtonElement>) => {
    event.stopPropagation();
    onDelete(segment);
  };

  const save = async (event: MouseEvent<HTMLButtonElement>) => {
    event.stopPropagation();
    const parsedStart = parseTimeToMs(startTimeStr);
    const parsedEnd = parseTimeToMs(endTimeStr);
    if (parsedStart === null || parsedEnd === null) {
      setError("时间格式不正确，请输入 mm:ss 或 hh:mm:ss");
      return;
    }
    if (parsedEnd <= parsedStart) {
      setError("结束时间必须大于起始时间");
      return;
    }
    if (!original.trim() || (bilingual && !translation.trim() && Boolean(segment.translatedText?.trim()))) return;
    setSaving(true);
    setError("");
    try {
      await onSave(original.trim(), translation.trim(), bilingual, parsedStart, parsedEnd);
      setEditing(false);
    } catch (reason) {
      setError(formatErrorMessage(reason));
    } finally {
      setSaving(false);
    }
  };

  const translationChanged = translation.trim() !== (segment.translatedText || "").trim();
  const timeChanged = startTimeStr.trim() !== time(segment.startMs) || endTimeStr.trim() !== time(segment.endMs);
  const textChanged = original.trim() !== segment.text.trim();
  const isDirty = textChanged || translationChanged || timeChanged;
  const saveDisabled = saving || !original.trim() || (bilingual && translationChanged && !translation.trim()) || !isDirty;

  return (
    <div
      className={"transcript-edit-group " + (editing ? "is-editing" : "")}
      onClick={editing ? (event) => event.stopPropagation() : undefined}
      onPointerDown={editing ? (event) => event.stopPropagation() : undefined}
      onKeyDown={editing ? (event) => event.stopPropagation() : undefined}
    >
      {editing ? (
        <>
          <div className="transcript-edit-time-row">
            <span className="transcript-edit-time-label">⏱ 起止时间:</span>
            <input
              type="text"
              className="transcript-time-input"
              value={startTimeStr}
              onChange={(e) => setStartTimeStr(e.target.value)}
              placeholder="00:00"
              aria-label="起始时间"
            />
            <span className="transcript-edit-time-sep">–</span>
            <input
              type="text"
              className="transcript-time-input"
              value={endTimeStr}
              onChange={(e) => setEndTimeStr(e.target.value)}
              placeholder="00:05"
              aria-label="结束时间"
            />
          </div>
          <textarea
            className="transcript-edit-input"
            value={original}
            onChange={(event) => setOriginal(event.target.value)}
            aria-label="编辑原文"
            rows={2}
          />
          {bilingual ? (
            <textarea
              className="transcript-edit-input transcript-edit-translation"
              value={translation}
              onChange={(event) => setTranslation(event.target.value)}
              aria-label="编辑译文"
              placeholder="译文"
              rows={2}
            />
          ) : null}
          <div className="transcript-edit-actions">
            <button
              type="button"
              className="transcript-save-btn"
              onClick={(event) => void save(event)}
              disabled={saveDisabled}
              title="保存修改"
            >
              <Save size={13} />
              保存
            </button>
            <button type="button" onClick={cancel} disabled={saving} title="取消编辑">
              取消
            </button>
            <button
              type="button"
              className="transcript-delete-action-btn"
              onClick={handleDelete}
              disabled={saving}
              title="删除此段字幕"
            >
              <Trash2 size={13} />
              删除此条
            </button>
          </div>
          {error ? <small className="transcript-edit-error">{error}</small> : null}
        </>
      ) : (
        <>
          <div className="transcript-edit-row">
            <span>{segment.text}</span>
          </div>
          {bilingual ? (
            <div className="transcript-edit-row transcript-translation-row">
              <span>{segment.translatedText || "尚未翻译"}</span>
            </div>
          ) : null}
          <div className="transcript-action-group">
            <button
              type="button"
              className="transcript-edit-button"
              onClick={beginEdit}
              title={bilingual ? "编辑原文、译文与起止时间" : "编辑原文与起止时间"}
              aria-label={bilingual ? "编辑原文、译文与起止时间" : "编辑原文与起止时间"}
            >
              <Pencil size={13} />
            </button>
            <button
              type="button"
              className="transcript-edit-button transcript-delete-button"
              onClick={handleDelete}
              title="删除此段字幕"
              aria-label="删除此段字幕"
            >
              <Trash2 size={13} />
            </button>
          </div>
        </>
      )}
    </div>
  );
}

export function VideoDetailPage({ video, onBack, onRefresh, autoPlayOnTranscriptClick }: VideoDetailPageProps) {
  const videoRef = useRef<HTMLVideoElement>(null);
  const playerShellRef = useRef<HTMLDivElement>(null);
  const transcriptRef = useRef<HTMLDivElement>(null);
  const segmentRefs = useRef(new Map<string, HTMLElement>());
  const lastProgressRenderMs = useRef(0);
  const [viewportHeight, setViewportHeight] = useState(() => window.innerHeight);
  const [media, setMedia] = useState<Awaited<ReturnType<typeof runtime.loadMedia>> | null>(null);
  const [transcript, setTranscript] = useState<TranscriptResult | null>(null);
  const [note, setNote] = useState<NoteResult | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [query, setQuery] = useState("");
  const [searchResults, setSearchResults] = useState<SemanticSearchResult[] | null>(null);
  const [searchIndex, setSearchIndex] = useState(0);
  const [showResultCards, setShowResultCards] = useState(false);
  const [targetSearchSegmentId, setTargetSearchSegmentId] = useState<string | null>(null);
  const [searching, setSearching] = useState(false);
  const [searchError, setSearchError] = useState("");
  const [translationMode, setTranslationMode] = useState<"original" | "bilingual">("bilingual");
  const [playing, setPlaying] = useState(false);
  const [controlsVisible, setControlsVisible] = useState(true);
  const controlsTimerRef = useRef<number | null>(null);

  const clearControlsTimer = useCallback(() => {
    if (controlsTimerRef.current !== null) {
      window.clearTimeout(controlsTimerRef.current);
      controlsTimerRef.current = null;
    }
  }, []);

  const scheduleControlsHide = useCallback(() => {
    clearControlsTimer();
    controlsTimerRef.current = window.setTimeout(() => {
      setControlsVisible(false);
    }, 2000);
  }, [clearControlsTimer]);

  const showControls = useCallback(() => {
    clearControlsTimer();
    setControlsVisible(true);
    if (playing) {
      scheduleControlsHide();
    }
  }, [playing, clearControlsTimer, scheduleControlsHide]);

  useEffect(() => {
    if (!playing) {
      clearControlsTimer();
      setControlsVisible(true);
    } else {
      scheduleControlsHide();
    }
  }, [playing, clearControlsTimer, scheduleControlsHide]);

  useEffect(() => {
    return () => {
      clearControlsTimer();
    };
  }, [clearControlsTimer]);
  const [muted, setMuted] = useState(false);
  const [volume, setVolume] = useState(() => {
    const saved = localStorage.getItem("videonotes_player_volume");
    if (saved !== null) {
      const num = Number(saved);
      if (Number.isFinite(num) && num >= 0 && num <= 1) return num;
    }
    return 1;
  });
  const clickTimerRef = useRef<number | null>(null);
  const [durationMs, setDurationMs] = useState(0);
  const [currentMs, setCurrentMs] = useState(0);
  const [videoQuality, setVideoQuality] = useState("未知清晰度");
  const [isFullscreen, setIsFullscreen] = useState(false);
  const currentJob = useSyncExternalStore(
    videoJobStore.subscribe,
    () => videoJobStore.getJob(video.id),
  );
  const action = currentJob ? currentJob.action : null;
  const translationProgress = currentJob?.action === "translate" ? currentJob.message : null;
  const summaryProgress = currentJob?.action === "organize" ? currentJob.message : null;
  const [activeSegmentId, setActiveSegmentId] = useState<string | null>(null);
  const [followPlayback, setFollowPlayback] = useState(true);

  useEffect(() => {
    let active = true;
    setLoading(true);
    setError("");
    setActiveSegmentId(null);
    setFollowPlayback(true);
    setTranslationMode("bilingual");
    setSearchResults(null);
    setSearchIndex(0);
    setShowResultCards(false);
    setTargetSearchSegmentId(null);
    setSearchError("");
    setQuery("");
    setCurrentMs(0);
    setDurationMs(0);
    setVideoQuality("未知清晰度");
    lastProgressRenderMs.current = 0;
    void Promise.all([
      runtime.loadMedia(video.id).catch(() => null),
      runtime.loadTranscript(video.id).catch(() => null),
      video.noteStatus === "ready" ? runtime.loadNote(video.id).catch(() => null) : Promise.resolve(null),
    ]).then(([loadedMedia, loadedTranscript, loadedNote]) => {
      if (!active) return;
      setMedia(loadedMedia);
      setTranscript(loadedTranscript);
      setNote(loadedNote);
      setLoading(false);
    }).catch((reason) => {
      if (active) { setError(formatErrorMessage(reason)); setLoading(false); }
    });
    return () => { active = false; };
  }, [video.id, video.noteStatus]);

  useEffect(() => {
    if (!runtime.isDesktop()) return undefined;
    let active = true;
    let unlisten: (() => void) | undefined;
    void listen<{ jobId: string; segmentId: string; translatedText: string }>("translation-segment-update", (event) => {
      if (!active || event.payload.jobId !== video.id) return;
      const { segmentId, translatedText } = event.payload;
      setTranscript((current) => {
        if (!current) return current;
        return {
          ...current,
          segments: current.segments.map((seg) => (seg.id === segmentId ? { ...seg, translatedText } : seg)),
        };
      });
    }).then((cleanup) => {
      unlisten = cleanup;
    });
    return () => { active = false; unlisten?.(); };
  }, [video.id]);

  useEffect(() => {
    const updateViewport = () => setViewportHeight(window.innerHeight);
    updateViewport();
    window.addEventListener("resize", updateViewport);
    return () => window.removeEventListener("resize", updateViewport);
  }, []);

  useEffect(() => {
    const updateFullscreen = () => setIsFullscreen(document.fullscreenElement === playerShellRef.current);
    document.addEventListener("fullscreenchange", updateFullscreen);
    return () => document.removeEventListener("fullscreenchange", updateFullscreen);
  }, []);

  const videoUrl = useMemo(() => media?.videoFile ? runtime.localAssetUrl(media.videoFile) : undefined, [media?.videoFile]);
  const segments = transcript?.segments ?? [];
  const matchingSegmentIds = useMemo(() => {
    if (!searchResults) return new Set<string>();
    return new Set(searchResults.flatMap((result) => result.segmentIds));
  }, [searchResults]);
  const canTranslate = Boolean(video.transcriptLanguage && !isChineseLanguage(video.transcriptLanguage));
  const translatedCount = segments.filter((segment) => Boolean(segment.translatedText?.trim())).length;
  const needsTranslationCount = canTranslate ? segments.filter((segment) => !segment.translatedText?.trim() && !isChineseText(segment.text)).length : 0;
  const remainingTranslationCount = needsTranslationCount;

  const seekToMs = (nextMs: number, shouldPlay = false) => {
    const current = videoRef.current;
    if (!current) return;
    const boundedMs = Math.max(0, Math.min(nextMs, durationMs || nextMs));
    current.currentTime = boundedMs / 1000;
    lastProgressRenderMs.current = boundedMs;
    setCurrentMs(boundedMs);
    setActiveSegmentId(segmentAt(segments, boundedMs)?.id ?? null);
    if (shouldPlay) void current.play();
  };

  const seek = (segment: TranscriptSegment) => {
    setFollowPlayback(true);
    seekToMs(segment.startMs, autoPlayOnTranscriptClick);
  };

  const jumpToSearchMatch = (index: number, resultsList: SemanticSearchResult[]) => {
    if (resultsList.length === 0) return;
    const target = resultsList[index];
    const segment = segments.find((item) => item.id === target.segmentIds[0]);
    setSearchIndex(index);
    setShowResultCards(false);
    setTargetSearchSegmentId(segment?.id ?? null);
    if (segment) seek(segment);
    else seekToMs(target.startMs, autoPlayOnTranscriptClick);
    setFollowPlayback(true);
  };

  const goToNextResult = () => {
    if (!searchResults || searchResults.length === 0) return;
    const next = (searchIndex + 1) % searchResults.length;
    jumpToSearchMatch(next, searchResults);
  };

  const goToPrevResult = () => {
    if (!searchResults || searchResults.length === 0) return;
    const prev = (searchIndex - 1 + searchResults.length) % searchResults.length;
    jumpToSearchMatch(prev, searchResults);
  };

  const runSearch = async () => {
    const nextQuery = query.trim();
    if (!nextQuery) { clearSearch(); return; }
    setSearching(true);
    setSearchError("");
    try {
      const response = await runtime.semanticSearchTranscript(video.id, nextQuery);
      setSearchResults(response.results);
      if (response.results.length > 0) {
        jumpToSearchMatch(0, response.results);
      } else {
        setSearchIndex(0);
        setShowResultCards(true);
      }
    } catch (reason) {
      setSearchResults([]);
      setShowResultCards(true);
      setSearchError(formatErrorMessage(reason, "语义定位失败，请稍后重试"));
    } finally { setSearching(false); }
  };

  const clearSearch = () => {
    setQuery("");
    setSearchResults(null);
    setSearchIndex(0);
    setShowResultCards(false);
    setTargetSearchSegmentId(null);
    setSearchError("");
    setFollowPlayback(true);
  };

  const seekResult = (result: SemanticSearchResult, index: number) => {
    if (searchResults) {
      jumpToSearchMatch(index, searchResults);
    } else {
      const segment = segments.find((item) => item.id === result.segmentIds[0]);
      if (segment) seek(segment);
      else seekToMs(result.startMs, autoPlayOnTranscriptClick);
    }
  };

  const updatePlayback = () => {
    const current = videoRef.current;
    if (!current) return;
    const nextMs = Math.max(0, current.currentTime * 1000);
    if (Math.abs(nextMs - lastProgressRenderMs.current) >= 250 || current.ended) {
      lastProgressRenderMs.current = nextMs;
      setCurrentMs(nextMs);
    }
    const next = segmentAt(segments, nextMs)?.id ?? null;
    setActiveSegmentId((previous) => previous === next ? previous : next);
  };

  const seekTo = (nextMs: number) => {
    seekToMs(nextMs);
  };

  const handleLoadedMetadata = () => {
    const current = videoRef.current;
    if (!current) return;
    current.volume = volume;
    current.muted = muted;
    const nextDurationMs = Number.isFinite(current.duration) ? current.duration * 1000 : 0;
    setDurationMs(nextDurationMs);
    setVideoQuality(current.videoHeight > 0 ? current.videoHeight + "p" : "未知清晰度");
    setCurrentMs(current.currentTime * 1000);
    lastProgressRenderMs.current = current.currentTime * 1000;
  };

  const changeVolume = (nextVolume: number) => {
    const clamped = Math.max(0, Math.min(1, nextVolume));
    setVolume(clamped);
    if (videoRef.current) {
      videoRef.current.volume = clamped;
      if (clamped > 0 && videoRef.current.muted) {
        videoRef.current.muted = false;
        setMuted(false);
      }
    }
    try {
      localStorage.setItem("videonotes_player_volume", String(clamped));
    } catch {}
  };

  const toggleMute = () => {
    if (!videoRef.current) return;
    const nextMuted = !videoRef.current.muted;
    videoRef.current.muted = nextMuted;
    setMuted(nextMuted);
    if (!nextMuted && volume === 0) {
      changeVolume(0.5);
    }
  };

  const toggleFullscreen = async () => {
    try {
      if (document.fullscreenElement) await document.exitFullscreen();
      else await playerShellRef.current?.requestFullscreen();
    } catch {
      // Fullscreen can be refused by the host window; keep the inline player usable.
    }
  };

  const handleVideoClick = (event: React.MouseEvent) => {
    event.preventDefault();
    if (clickTimerRef.current !== null) {
      window.clearTimeout(clickTimerRef.current);
      clickTimerRef.current = null;
    }
    clickTimerRef.current = window.setTimeout(() => {
      clickTimerRef.current = null;
      if (!videoRef.current) return;
      if (videoRef.current.paused) void videoRef.current.play();
      else videoRef.current.pause();
    }, 220);
  };

  const handleVideoDoubleClick = (event: React.MouseEvent) => {
    event.preventDefault();
    if (clickTimerRef.current !== null) {
      window.clearTimeout(clickTimerRef.current);
      clickTimerRef.current = null;
    }
    void toggleFullscreen();
  };

  useEffect(() => {
    const onFullscreenChange = () => {
      setIsFullscreen(Boolean(document.fullscreenElement));
    };
    document.addEventListener("fullscreenchange", onFullscreenChange);
    return () => {
      document.removeEventListener("fullscreenchange", onFullscreenChange);
      if (clickTimerRef.current !== null) {
        window.clearTimeout(clickTimerRef.current);
      }
    };
  }, []);

  useEffect(() => {
    if (!targetSearchSegmentId || showResultCards) return;
    const container = transcriptRef.current;
    const row = segmentRefs.current.get(targetSearchSegmentId);
    if (!container || !row) return;
    const containerRect = container.getBoundingClientRect();
    const rowRect = row.getBoundingClientRect();
    const targetTop = container.scrollTop + (rowRect.top - containerRect.top) - (container.clientHeight / 2 - rowRect.height / 2);
    container.scrollTo({ top: Math.max(0, targetTop), behavior: "smooth" });
  }, [targetSearchSegmentId, showResultCards]);

  useEffect(() => {
    if (showResultCards || !followPlayback || !activeSegmentId || !segments.some((segment) => segment.id === activeSegmentId)) return;
    const container = transcriptRef.current;
    const row = segmentRefs.current.get(activeSegmentId);
    if (!container || !row) return;
    const containerRect = container.getBoundingClientRect();
    const rowRect = row.getBoundingClientRect();
    const margin = 12;
    const visibleTop = Math.max(containerRect.top + margin, margin);
    const visibleBottom = Math.min(containerRect.bottom - margin, viewportHeight - margin);
    if (visibleBottom <= visibleTop) return;
    const maxScrollTop = Math.max(0, container.scrollHeight - container.clientHeight);
    if (rowRect.top < visibleTop) {
      const targetTop = Math.max(0, Math.min(maxScrollTop, container.scrollTop + rowRect.top - visibleTop));
      container.scrollTo({ top: targetTop, behavior: "auto" });
    } else if (rowRect.bottom > visibleBottom) {
      const targetTop = Math.max(0, Math.min(maxScrollTop, container.scrollTop + rowRect.bottom - visibleBottom));
      container.scrollTo({ top: targetTop, behavior: "auto" });
    }
  }, [activeSegmentId, followPlayback, segments, viewportHeight, showResultCards]);

  const [deleteCandidate, setDeleteCandidate] = useState<TranscriptSegment | null>(null);

  const saveSegment = async (
    segment: TranscriptSegment,
    original: string,
    translation: string,
    bilingual: boolean,
    startMs?: number,
    endMs?: number,
  ) => {
    const originalChanged = original !== segment.text.trim();
    const previousTranslation = (segment.translatedText || "").trim();
    const translationChanged = bilingual && translation !== previousTranslation;
    const timeChanged =
      (startMs !== undefined && startMs !== segment.startMs) ||
      (endMs !== undefined && endMs !== segment.endMs);
    if (!originalChanged && !translationChanged && !timeChanged) return;
    if (originalChanged || timeChanged) {
      await runtime.updateTranscriptSegment(video.id, segment.id, original, startMs, endMs);
    }
    if (translationChanged) {
      await runtime.updateTranslationSegment(video.id, segment.id, translation);
    }
    setNote(null);
    setTranscript(await runtime.loadTranscript(video.id));
    await onRefresh();
    toast.success("字幕修改已保存");
  };

  const confirmDeleteSegment = async () => {
    if (!deleteCandidate) return;
    const target = deleteCandidate;
    setDeleteCandidate(null);
    try {
      await runtime.deleteTranscriptSegment(video.id, target.id);
      setNote(null);
      setTranscript(await runtime.loadTranscript(video.id));
      await onRefresh();
      toast.success("已删除该段字幕");
    } catch (reason) {
      toast.error(`删除字幕失败: ${formatErrorMessage(reason)}`);
    }
  };
  const translate = async (force = false) => {
    try {
      await videoJobStore.startTranslate(video.id, force);
      await onRefresh();
      const updated = await runtime.loadTranscript(video.id);
      setTranscript(updated);
      toast.success(force ? "外语字幕重新翻译完成" : "外语字幕翻译完成");
    } catch (reason) {
      const msg = formatErrorMessage(reason);
      if (msg.includes("尚未安装") || msg.includes("请先前往")) {
        toast.warning(msg);
      } else {
        toast.error(`翻译失败: ${msg}`);
      }
    }
  };
  const organize = async () => {
    try {
      const noteResult = await videoJobStore.startOrganize(video, true);
      setNote(noteResult);
      await onRefresh();
      toast.success("结构化笔记整理完成");
    } catch (reason) {
      const msg = formatErrorMessage(reason);
      if (msg.includes("尚未安装") || msg.includes("请先前往")) {
        toast.warning(msg);
      } else {
        toast.error(`笔记整理失败: ${msg}`);
      }
    }
  };
  const exportNote = async () => { if (!note) return; const path = await runtime.exportMarkdown(video.title.replace(/[\\/:*?"<>|]/g, "_") + ".md", note.markdown); if (path) toast.success(`Markdown 笔记已导出至：${path}`); };

  if (loading) return <section className="standard-page page-frame"><div className="detail-loading"><LoaderCircle className="spin" size={25} />正在读取视频内容……</div></section>;

  const progressMax = durationMs || 0;
  return <section className="video-detail-page page-frame">
    <header className="video-detail-header"><button type="button" className="back-button" onClick={onBack}><ArrowLeft size={17} />返回视频库</button><div><h1>{video.title}</h1><p>{video.platform} · {video.duration} · {video.author || "未知作者"}</p></div></header>
    {error ? <div className="entry-message"><span className="error-message">{error}</span></div> : null}
    <div className="video-workspace">
      <section className="video-workspace-player-pane" aria-label="视频播放器">
        <div
          className={`detail-player ${!controlsVisible && playing ? "is-controls-hidden" : ""}`}
          ref={playerShellRef}
          onClick={handleVideoClick}
          onDoubleClick={handleVideoDoubleClick}
          onMouseMove={showControls}
          onMouseLeave={() => {
            clearControlsTimer();
            if (playing) setControlsVisible(false);
          }}
          onWheel={(e) => {
            const delta = e.deltaY < 0 ? 0.05 : -0.05;
            changeVolume(volume + delta);
            showControls();
          }}
        >
          <video
            ref={videoRef}
            src={videoUrl}
            poster={media?.thumbnailFile ? runtime.localAssetUrl(media.thumbnailFile) : video.thumbnailUrl || fallbackThumbnailUrl}
            onPlay={() => setPlaying(true)}
            onPause={() => setPlaying(false)}
            onEnded={() => setPlaying(false)}
            onLoadedMetadata={handleLoadedMetadata}
            onTimeUpdate={updatePlayback}
            controls={false}
          />
          <div
            className={`detail-player-controls ${!controlsVisible && playing ? "is-hidden" : ""}`}
            onClick={(e) => {
              e.stopPropagation();
              showControls();
            }}
            onDoubleClick={(e) => e.stopPropagation()}
          >
            <button
              type="button"
              onClick={() => {
                if (!videoRef.current) return;
                if (videoRef.current.paused) void videoRef.current.play();
                else videoRef.current.pause();
              }}
              aria-label={playing ? "暂停" : "播放"}
            >
              {playing ? <Pause size={17} /> : <Play size={17} fill="currentColor" />}
            </button>
            <div
              className="detail-player-volume"
              onWheel={(e) => {
                e.stopPropagation();
                const delta = e.deltaY < 0 ? 0.05 : -0.05;
                changeVolume(volume + delta);
              }}
            >
              <div className="detail-player-volume-popover">
                <span className="detail-player-volume-text">{muted ? "静音" : `${Math.round(volume * 100)}%`}</span>
                <div className="detail-player-volume-track-wrap">
                  <input
                    className="detail-player-volume-slider"
                    type="range"
                    min="0"
                    max="1"
                    step="0.01"
                    value={muted ? 0 : volume}
                    onChange={(event) => changeVolume(Number(event.target.value))}
                    aria-label="视频音量调节"
                  />
                </div>
              </div>
              <button
                type="button"
                onClick={toggleMute}
                aria-label={muted ? "取消静音" : "静音"}
                title={muted ? "取消静音" : `音量 ${Math.round(volume * 100)}%`}
              >
                {muted || volume === 0 ? <VolumeX size={17} /> : volume <= 0.5 ? <Volume1 size={17} /> : <Volume2 size={17} />}
              </button>
            </div>
            <span className="detail-player-time">{time(currentMs)} / {time(durationMs)}</span>
            <input
              className="detail-player-progress"
              type="range"
              min="0"
              max={progressMax}
              step="100"
              value={Math.min(currentMs, progressMax || currentMs)}
              onChange={(event) => seekTo(Number(event.target.value))}
              aria-label="视频播放进度"
              aria-valuetext={time(currentMs) + " / " + time(durationMs)}
              disabled={!durationMs}
            />
            <span className="detail-player-quality" aria-label={"视频清晰度 " + videoQuality}>{videoQuality}</span>
            <button
              type="button"
              onClick={() => void toggleFullscreen()}
              aria-label={isFullscreen ? "退出全屏" : "进入全屏"}
              title={isFullscreen ? "退出全屏 (双击视频恢复)" : "进入全屏 (双击视频最大化)"}
            >
              {isFullscreen ? <Minimize2 size={16} /> : <Maximize2 size={16} />}
            </button>
          </div>
        </div>
        <p className="detail-player-caption"><span className="detail-player-status-dot" />本地视频 · {video.platform} · {video.duration}</p>
        <section className="video-summary-pane" aria-label="视频总结">
          <div className="video-summary-header"><div className="video-summary-title"><List size={17} /><span>视频总结</span></div><div className="video-summary-actions"><button type="button" className="secondary-button compact-button" disabled={action !== null} onClick={() => void organize()}>{action === "organize" ? <LoaderCircle size={15} className="spin" /> : <RefreshCw size={15} />}{action === "organize" ? (summaryProgress || "正在整理总结...") : (note ? "重新生成总结" : "生成总结")}</button>{note ? <button type="button" className="secondary-button compact-button" onClick={() => void exportNote()}><Download size={15} />导出 Markdown</button> : null}</div></div>
          {note ? <div className="video-summary-content"><p>{note.summary}</p>{note.keyPoints.length ? <ul>{note.keyPoints.map((point) => <li key={point}>{point}</li>)}</ul> : null}</div> : <div className="video-summary-empty"><FileText size={19} /><span>还没有总结</span><small>根据当前校正字幕生成摘要和要点。</small></div>}
        </section>
      </section>

      <section className="video-workspace-content-pane" aria-label="视频内容">
        <div className="workspace-transcript-pane">
          <div className="workspace-pane-toolbar">
            <div className="detail-search-box">
              <label className="detail-search">
                <Search size={15} />
                <input
                  value={query}
                  onChange={(event) => {
                    setQuery(event.target.value);
                    if (!event.target.value.trim() && searchResults) {
                      clearSearch();
                    }
                  }}
                  onKeyDown={(event) => {
                    if (event.key === "Enter") {
                      event.preventDefault();
                      if (event.shiftKey) {
                        if (searchResults && searchResults.length > 0) goToPrevResult();
                        else void runSearch();
                      } else {
                        if (searchResults && searchResults.length > 0) goToNextResult();
                        else void runSearch();
                      }
                    } else if (event.key === "Escape") {
                      clearSearch();
                    }
                  }}
                  placeholder="搜索字幕内容 (Enter 查找/下一个)"
                  aria-label="搜索字幕内容"
                />
                {query ? (
                  <button
                    type="button"
                    className="search-clear-btn"
                    onClick={clearSearch}
                    title="清除搜索 (Esc)"
                    aria-label="清除搜索"
                  >
                    <X size={13} />
                  </button>
                ) : null}
              </label>

              <button
                type="button"
                className="semantic-search-submit"
                onClick={() => void runSearch()}
                disabled={searching || !query.trim()}
                title="定位相关片段"
              >
                {searching ? <LoaderCircle size={14} className="spin" /> : <Sparkles size={14} />}
                定位
              </button>

              {searchResults && searchResults.length > 0 ? (
                <div className="search-nav-controls" role="group" aria-label="搜索结果导航">
                  <span className="search-nav-counter">
                    {searchIndex + 1} / {searchResults.length}
                  </span>
                  <button
                    type="button"
                    className="search-nav-btn"
                    onClick={goToPrevResult}
                    title="上一处 (Shift+Enter)"
                    aria-label="上一处"
                  >
                    <ChevronUp size={14} />
                  </button>
                  <button
                    type="button"
                    className="search-nav-btn"
                    onClick={goToNextResult}
                    title="下一处 (Enter)"
                    aria-label="下一处"
                  >
                    <ChevronDown size={14} />
                  </button>
                  <button
                    type="button"
                    className={`search-nav-toggle-cards ${showResultCards ? "is-active" : ""}`}
                    onClick={() => setShowResultCards(!showResultCards)}
                    title={showResultCards ? "切换到全文字幕" : "查看全部匹配摘要卡片"}
                    aria-label={showResultCards ? "切换到全文字幕" : "查看全部匹配摘要卡片"}
                  >
                    <LayoutList size={14} />
                  </button>
                </div>
              ) : null}
            </div>

            <div className="workspace-toolbar-actions">
              {canTranslate ? (
                <div className="translation-switch" role="group" aria-label="字幕显示方式">
                  <button
                    type="button"
                    className={translationMode === "original" ? "is-active" : ""}
                    aria-pressed={translationMode === "original"}
                    onClick={() => setTranslationMode("original")}
                  >
                    原文
                  </button>
                  <button
                    type="button"
                    className={translationMode === "bilingual" ? "is-active" : ""}
                    aria-pressed={translationMode === "bilingual"}
                    onClick={() => setTranslationMode("bilingual")}
                  >
                    双语
                  </button>
                </div>
              ) : null}
              <button
                type="button"
                className={"transcript-follow-toggle " + (followPlayback ? "is-active" : "")}
                aria-pressed={followPlayback}
                onClick={() => setFollowPlayback((value) => !value)}
                title={followPlayback ? "关闭跟随播放" : "开启跟随播放"}
              >
                <LocateFixed size={15} />
                跟随
              </button>
            </div>
          </div>

          <div className="workspace-pane-hint">
            <div className="workspace-pane-status">
              <span>
                {searchResults
                  ? showResultCards
                    ? `共找到 ${searchResults.length} 处相关片段（卡片视图）`
                    : `找到 ${searchResults.length} 处相关 · 第 ${searchIndex + 1} 处`
                  : `${segments.length} 段 · 校正字幕${canTranslate ? " · 已翻译 " + translatedCount + "/" + segments.length : ""}`}
              </span>
              {searchResults ? (
                <button type="button" className="semantic-search-clear" onClick={clearSearch}>
                  <X size={13} />
                  清除搜索
                </button>
              ) : canTranslate && translationMode === "bilingual" ? (
                <button
                  type="button"
                  className="translation-action"
                  disabled={action !== null || segments.length === 0}
                  onClick={() => void translate(remainingTranslationCount === 0)}
                  title={remainingTranslationCount === 0 ? "重新翻译所有分段" : undefined}
                >
                  {action === "translate" ? (
                    <>
                      <LoaderCircle size={13} className="spin" />
                      {translationProgress || "正在翻译..."}
                    </>
                  ) : remainingTranslationCount === 0 ? (
                    <>
                      <RotateCcw size={13} />
                      重新翻译
                    </>
                  ) : (
                    <>
                      <Languages size={13} />
                      {translatedCount === 0 ? "开始翻译" : "继续翻译"}
                    </>
                  )}
                </button>
              ) : null}
            </div>
            <span>{searchResults && showResultCards ? "点击卡片定位全文" : "点击字幕跳转"}</span>
          </div>

          {searchError ? <div className="semantic-search-error" role="alert">{searchError}</div> : null}

          <div
            className="detail-transcript"
            ref={transcriptRef}
            onWheel={() => setFollowPlayback(false)}
            onTouchStart={() => setFollowPlayback(false)}
            onPointerDown={() => setFollowPlayback(false)}
            onKeyDown={(event) => {
              if (["ArrowUp", "ArrowDown", "PageUp", "PageDown", "Home", "End", " ", "Enter"].includes(event.key)) {
                setFollowPlayback(false);
              }
            }}
          >
            {showResultCards && searchResults ? (
              searchResults.length === 0 ? (
                <div className="workspace-empty-state">
                  <FileText size={25} />
                  <p>{searchError ? "定位失败" : "没有找到相关片段"}</p>
                  <small>换个说法再试试，或清除搜索查看完整字幕。</small>
                </div>
              ) : (
                <div className="semantic-search-results">
                  {searchResults.map((result, index) => (
                    <article
                      key={result.chunkId}
                      className={"semantic-result-card " + (index === searchIndex ? "is-featured" : "")}
                      role="button"
                      tabIndex={0}
                      onClick={() => seekResult(result, index)}
                      onKeyDown={(event) => {
                        if (event.key === "Enter" || event.key === " ") {
                          event.preventDefault();
                          seekResult(result, index);
                        }
                      }}
                    >
                      <div className="semantic-result-meta">
                        <span className="semantic-result-time">
                          {time(result.startMs)}–{time(result.endMs)}
                        </span>
                        {index === 0 ? <span className="semantic-result-badge">最相关</span> : null}
                        <button
                          type="button"
                          className="semantic-result-play"
                          onClick={(event) => {
                            event.stopPropagation();
                            seekResult(result, index);
                          }}
                        >
                          <PlayCircle size={16} />
                          定位并播放
                        </button>
                      </div>
                      <p>{result.snippet}</p>
                    </article>
                  ))}
                </div>
              )
            ) : segments.length === 0 ? (
              <div className="workspace-empty-state">
                <FileText size={25} />
                <p>还没有字幕</p>
              </div>
            ) : (
              segments.map((segment) => {
                const active = activeSegmentId === segment.id;
                const isTarget = targetSearchSegmentId === segment.id;
                const isMatch = matchingSegmentIds.has(segment.id);
                const bilingual = canTranslate && translationMode === "bilingual";
                return (
                  <article
                    className={
                      "detail-segment " +
                      (active ? "is-active " : "") +
                      (isTarget ? "is-search-target " : "") +
                      (isMatch && !isTarget ? "is-search-match " : "")
                    }
                    aria-current={active ? "true" : undefined}
                    aria-label={"跳转到 " + time(segment.startMs)}
                    role="button"
                    tabIndex={0}
                    onClick={() => seek(segment)}
                    onKeyDown={(event) => {
                      if (event.key === "Enter" || event.key === " ") {
                        event.preventDefault();
                        seek(segment);
                      }
                    }}
                    ref={(element) => {
                      if (element) segmentRefs.current.set(segment.id, element);
                      else segmentRefs.current.delete(segment.id);
                    }}
                    key={segment.id}
                  >
                    <button
                      type="button"
                      className="segment-time"
                      onClick={(event) => {
                        event.stopPropagation();
                        seek(segment);
                      }}
                      title="点击跳转播放"
                    >
                      <span className="segment-time-start">{time(segment.startMs)}</span>
                      <span className="segment-time-sep">–</span>
                      <span className="segment-time-end">{time(segment.endMs)}</span>
                    </button>
                    <div className="segment-copy">
                      <EditRow
                        segment={segment}
                        bilingual={bilingual}
                        onSave={(original, translation, editBilingual, startMs, endMs) =>
                          saveSegment(segment, original, translation, editBilingual, startMs, endMs)
                        }
                        onDelete={(seg) => setDeleteCandidate(seg)}
                      />
                    </div>
                  </article>
                );
              })
            )}
          </div>
        </div>
      </section>
      {deleteCandidate ? (
        <ConfirmDialog
          title="删除字幕片段"
          message={`确定删除此段字幕吗？\n时间：${time(deleteCandidate.startMs)} – ${time(deleteCandidate.endMs)}\n文本：${deleteCandidate.text}`}
          highlightText={time(deleteCandidate.startMs) + " – " + time(deleteCandidate.endMs)}
          confirmText="确定删除"
          isDanger
          onConfirm={() => void confirmDeleteSegment()}
          onCancel={() => setDeleteCandidate(null)}
        />
      ) : null}
    </div>
  </section>;
}

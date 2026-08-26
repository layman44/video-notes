import { ArrowLeft, Download, FileText, Languages, List, LoaderCircle, LocateFixed, Maximize2, Minimize2, Pause, Pencil, Play, RefreshCw, Save, Search, Volume2, VolumeX } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type MouseEvent } from "react";
import fallbackThumbnailUrl from "../../assets/rag-thumbnail.png";
import { isChineseLanguage } from "../../lib/language";
import { formatErrorMessage, runtime } from "../../lib/runtime";
import type { NoteResult, TranscriptResult, TranscriptSegment, Video } from "../../types";

interface VideoDetailPageProps { video: Video; onBack: () => void; onRefresh: () => Promise<void>; autoPlayOnTranscriptClick: boolean; }

function time(ms: number) {
  const total = Math.floor(Math.max(0, ms) / 1000);
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const seconds = total % 60;
  return hours ? hours + ":" + String(minutes).padStart(2, "0") + ":" + String(seconds).padStart(2, "0") : String(minutes).padStart(2, "0") + ":" + String(seconds).padStart(2, "0");
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

function EditRow({ segment, bilingual, onSave }: { segment: TranscriptSegment; bilingual: boolean; onSave: (original: string, translation: string, bilingual: boolean) => Promise<void> }) {
  const [editing, setEditing] = useState(false);
  const [original, setOriginal] = useState(segment.text);
  const [translation, setTranslation] = useState(segment.translatedText || "");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => {
    if (!editing) {
      setOriginal(segment.text);
      setTranslation(segment.translatedText || "");
    }
  }, [editing, segment.text, segment.translatedText]);
  const beginEdit = (event: MouseEvent<HTMLButtonElement>) => {
    event.stopPropagation();
    setError("");
    setEditing(true);
  };
  const cancel = (event: MouseEvent<HTMLButtonElement>) => {
    event.stopPropagation();
    setEditing(false);
  };
  const save = async (event: MouseEvent<HTMLButtonElement>) => {
    event.stopPropagation();
    if (!original.trim() || (bilingual && !translation.trim() && Boolean(segment.translatedText?.trim()))) return;
    setSaving(true);
    setError("");
    try {
      await onSave(original.trim(), translation.trim(), bilingual);
      setEditing(false);
    } catch (reason) {
      setError(formatErrorMessage(reason));
    } finally {
      setSaving(false);
    }
  };
  const translationChanged = translation.trim() !== (segment.translatedText || "").trim();
  const saveDisabled = saving || !original.trim() || (bilingual && translationChanged && !translation.trim());
  return <div className={"transcript-edit-group " + (editing ? "is-editing" : "")} onClick={editing ? (event) => event.stopPropagation() : undefined} onPointerDown={editing ? (event) => event.stopPropagation() : undefined} onKeyDown={editing ? (event) => event.stopPropagation() : undefined}>
    {editing ? <>
      <textarea className="transcript-edit-input" value={original} onChange={(event) => setOriginal(event.target.value)} aria-label="编辑原文" rows={2} />
      {bilingual ? <textarea className="transcript-edit-input transcript-edit-translation" value={translation} onChange={(event) => setTranslation(event.target.value)} aria-label="编辑译文" placeholder="译文" rows={2} /> : null}
      <div className="transcript-edit-actions"><button type="button" onClick={(event) => void save(event)} disabled={saveDisabled} title="保存"><Save size={14} />保存</button><button type="button" onClick={cancel} disabled={saving} title="取消">取消</button></div>
      {error ? <small className="transcript-edit-error">{error}</small> : null}
    </> : <>
      <div className="transcript-edit-row"><span>{segment.text}</span></div>
      {bilingual ? <div className="transcript-edit-row transcript-translation-row"><span>{segment.translatedText || "尚未翻译"}</span></div> : null}
      <button type="button" className="transcript-edit-button" onClick={beginEdit} title={bilingual ? "编辑原文和译文" : "编辑原文"} aria-label={bilingual ? "编辑原文和译文" : "编辑原文"}><Pencil size={14} /></button>
    </>}
  </div>;
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
  const [translationMode, setTranslationMode] = useState<"original" | "bilingual">("bilingual");
  const [playing, setPlaying] = useState(false);
  const [muted, setMuted] = useState(false);
  const [durationMs, setDurationMs] = useState(0);
  const [currentMs, setCurrentMs] = useState(0);
  const [videoQuality, setVideoQuality] = useState("未知清晰度");
  const [isFullscreen, setIsFullscreen] = useState(false);
  const [action, setAction] = useState<"translate" | "organize" | null>(null);
  const [activeSegmentId, setActiveSegmentId] = useState<string | null>(null);
  const [followPlayback, setFollowPlayback] = useState(true);

  useEffect(() => {
    let active = true;
    setLoading(true);
    setError("");
    setActiveSegmentId(null);
    setFollowPlayback(true);
    setTranslationMode("bilingual");
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
  const visible = useMemo(() => {
    const filter = query.trim();
    return segments.filter((segment) => !filter || segment.text.includes(filter) || segment.translatedText?.includes(filter));
  }, [segments, query]);
  const canTranslate = Boolean(video.transcriptLanguage && !isChineseLanguage(video.transcriptLanguage));
  const translatedCount = segments.filter((segment) => Boolean(segment.translatedText?.trim())).length;
  const remainingTranslationCount = Math.max(0, segments.length - translatedCount);

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
    const nextDurationMs = Number.isFinite(current.duration) ? current.duration * 1000 : 0;
    setDurationMs(nextDurationMs);
    setVideoQuality(current.videoHeight > 0 ? current.videoHeight + "p" : "未知清晰度");
    setCurrentMs(current.currentTime * 1000);
    lastProgressRenderMs.current = current.currentTime * 1000;
  };

  const toggleFullscreen = async () => {
    try {
      if (document.fullscreenElement) await document.exitFullscreen();
      else await playerShellRef.current?.requestFullscreen();
    } catch {
      // Fullscreen can be refused by the host window; keep the inline player usable.
    }
  };

  useEffect(() => {
    if (!followPlayback || !activeSegmentId || !visible.some((segment) => segment.id === activeSegmentId)) return;
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
  }, [activeSegmentId, followPlayback, visible, viewportHeight]);

  const saveSegment = async (segment: TranscriptSegment, original: string, translation: string, bilingual: boolean) => {
    const originalChanged = original !== segment.text.trim();
    const previousTranslation = (segment.translatedText || "").trim();
    const translationChanged = bilingual && translation !== previousTranslation;
    if (!originalChanged && !translationChanged) return;
    if (originalChanged) await runtime.updateTranscriptSegment(video.id, segment.id, original);
    if (translationChanged) await runtime.updateTranslationSegment(video.id, segment.id, translation);
    setNote(null);
    setTranscript(await runtime.loadTranscript(video.id));
    await onRefresh();
  };
  const translate = async () => { setAction("translate"); try { await runtime.translateTranscript(video.id, () => undefined); await onRefresh(); setTranscript(await runtime.loadTranscript(video.id)); } catch (reason) { window.alert(formatErrorMessage(reason)); } finally { setAction(null); } };
  const organize = async () => { setAction("organize"); try { setNote(await runtime.organizeNotes(video, () => undefined, true)); await onRefresh(); } catch (reason) { window.alert(formatErrorMessage(reason)); } finally { setAction(null); } };
  const exportNote = async () => { if (!note) return; const path = await runtime.exportMarkdown(video.title.replace(/[\\/:*?"<>|]/g, "_") + ".md", note.markdown); if (path) window.alert("Markdown 已导出"); };

  if (loading) return <section className="standard-page page-frame"><div className="detail-loading"><LoaderCircle className="spin" size={25} />正在读取视频内容……</div></section>;

  const progressMax = durationMs || 0;
  return <section className="video-detail-page page-frame">
    <header className="video-detail-header"><button type="button" className="back-button" onClick={onBack}><ArrowLeft size={17} />返回视频库</button><div><h1>{video.title}</h1><p>{video.platform} · {video.duration} · {video.author || "未知作者"}</p></div></header>
    {error ? <div className="entry-message"><span className="error-message">{error}</span></div> : null}
    <div className="video-workspace">
      <section className="video-workspace-player-pane" aria-label="视频播放器">
        <div className="detail-player" ref={playerShellRef}>
          <video ref={videoRef} src={videoUrl} poster={media?.thumbnailFile ? runtime.localAssetUrl(media.thumbnailFile) : video.thumbnailUrl || fallbackThumbnailUrl} onPlay={() => setPlaying(true)} onPause={() => setPlaying(false)} onEnded={() => setPlaying(false)} onLoadedMetadata={handleLoadedMetadata} onTimeUpdate={updatePlayback} controls={false} />
          <div className="detail-player-controls">
            <button type="button" onClick={() => { if (!videoRef.current) return; if (videoRef.current.paused) void videoRef.current.play(); else videoRef.current.pause(); }} aria-label={playing ? "暂停" : "播放"}>{playing ? <Pause size={17} /> : <Play size={17} fill="currentColor" />}</button>
            <button type="button" onClick={() => { if (!videoRef.current) return; videoRef.current.muted = !videoRef.current.muted; setMuted(videoRef.current.muted); }} aria-label={muted ? "取消静音" : "静音"}>{muted ? <VolumeX size={17} /> : <Volume2 size={17} />}</button>
            <span className="detail-player-time">{time(currentMs)} / {time(durationMs)}</span>
            <input className="detail-player-progress" type="range" min="0" max={progressMax} step="100" value={Math.min(currentMs, progressMax || currentMs)} onChange={(event) => seekTo(Number(event.target.value))} aria-label="视频播放进度" aria-valuetext={time(currentMs) + " / " + time(durationMs)} disabled={!durationMs} />
            <span className="detail-player-quality" aria-label={"视频清晰度 " + videoQuality}>{videoQuality}</span>
            <button type="button" onClick={() => void toggleFullscreen()} aria-label={isFullscreen ? "退出全屏" : "进入全屏"} title={isFullscreen ? "退出全屏" : "进入全屏"}>{isFullscreen ? <Minimize2 size={16} /> : <Maximize2 size={16} />}</button>
          </div>
        </div>
        <p className="detail-player-caption"><span className="detail-player-status-dot" />本地视频 · {video.platform} · {video.duration}</p>
        <section className="video-summary-pane" aria-label="视频总结">
          <div className="video-summary-header"><div className="video-summary-title"><List size={17} /><span>视频总结</span></div><div className="video-summary-actions"><button type="button" className="secondary-button compact-button" disabled={action !== null} onClick={() => void organize()}>{action === "organize" ? <LoaderCircle size={15} className="spin" /> : <RefreshCw size={15} />}{note ? "重新生成总结" : "生成总结"}</button>{note ? <button type="button" className="secondary-button compact-button" onClick={() => void exportNote()}><Download size={15} />导出 Markdown</button> : null}</div></div>
          {note ? <div className="video-summary-content"><p>{note.summary}</p>{note.keyPoints.length ? <ul>{note.keyPoints.map((point) => <li key={point}>{point}</li>)}</ul> : null}</div> : <div className="video-summary-empty"><FileText size={19} /><span>还没有总结</span><small>根据当前校正字幕生成摘要和要点。</small></div>}
        </section>
      </section>

      <section className="video-workspace-content-pane" aria-label="视频内容">
        <div className="workspace-transcript-pane">
          <div className="workspace-pane-toolbar">
            <label className="detail-search"><Search size={15} /><input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="搜索字幕内容" aria-label="搜索字幕内容" /></label>
            <div className="workspace-toolbar-actions">
              {canTranslate ? <div className="translation-switch" role="group" aria-label="字幕显示方式"><button type="button" className={translationMode === "original" ? "is-active" : ""} aria-pressed={translationMode === "original"} onClick={() => setTranslationMode("original")}>原文</button><button type="button" className={translationMode === "bilingual" ? "is-active" : ""} aria-pressed={translationMode === "bilingual"} onClick={() => setTranslationMode("bilingual")}>双语</button></div> : null}
              <button type="button" className={"transcript-follow-toggle " + (followPlayback ? "is-active" : "")} aria-pressed={followPlayback} onClick={() => setFollowPlayback((value) => !value)} title={followPlayback ? "关闭跟随播放" : "开启跟随播放"}><LocateFixed size={15} />跟随</button>
            </div>
          </div>
          <div className="workspace-pane-hint"><div className="workspace-pane-status"><span>{segments.length} 段 · 校正字幕{canTranslate ? " · 已翻译 " + translatedCount + "/" + segments.length : ""}</span>{canTranslate && translationMode === "bilingual" && remainingTranslationCount > 0 ? <button type="button" className="translation-action" disabled={action !== null || segments.length === 0} onClick={() => void translate()}>{action === "translate" ? <LoaderCircle size={13} className="spin" /> : <Languages size={13} />}{translatedCount === 0 ? "开始翻译" : "继续翻译"}</button> : null}</div><span>点击字幕跳转</span></div>
          <div className="detail-transcript" ref={transcriptRef} onWheel={() => setFollowPlayback(false)} onTouchStart={() => setFollowPlayback(false)} onPointerDown={() => setFollowPlayback(false)} onKeyDown={(event) => { if (["ArrowUp", "ArrowDown", "PageUp", "PageDown", "Home", "End", " ", "Enter"].includes(event.key)) setFollowPlayback(false); }}>
            {visible.length === 0 ? <div className="workspace-empty-state"><FileText size={25} /><p>{query.trim() ? "没有匹配的字幕" : "还没有字幕"}</p></div> : visible.map((segment) => { const active = activeSegmentId === segment.id; const bilingual = canTranslate && translationMode === "bilingual"; return <article className={"detail-segment " + (active ? "is-active" : "")} aria-current={active ? "true" : undefined} aria-label={"跳转到 " + time(segment.startMs)} role="button" tabIndex={0} onClick={() => seek(segment)} onKeyDown={(event) => { if (event.key === "Enter" || event.key === " ") { event.preventDefault(); seek(segment); } }} ref={(element) => { if (element) segmentRefs.current.set(segment.id, element); else segmentRefs.current.delete(segment.id); }} key={segment.id}><button type="button" className="segment-time" onClick={(event) => { event.stopPropagation(); seek(segment); }}>{time(segment.startMs)}</button><div className="segment-copy"><EditRow segment={segment} bilingual={bilingual} onSave={(original, translation, editBilingual) => saveSegment(segment, original, translation, editBilingual)} /></div></article>; })}
          </div>
        </div>
      </section>
    </div>
  </section>;
}

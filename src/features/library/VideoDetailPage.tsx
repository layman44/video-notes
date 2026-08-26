import { ArrowLeft, BookOpen, Download, FileText, Languages, List, LoaderCircle, LocateFixed, Pause, Pencil, Play, RefreshCw, Save, Search, Volume2, VolumeX } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
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

function EditRow({ segment, translated, onSave }: { segment: TranscriptSegment; translated: boolean; onSave: (text: string) => Promise<void> }) {
  const [editing, setEditing] = useState(false);
  const sourceValue = translated ? segment.translatedText || "" : segment.text;
  const [value, setValue] = useState(sourceValue);
  const [saving, setSaving] = useState(false);
  useEffect(() => { if (!editing) setValue(sourceValue); }, [editing, sourceValue]);
  const save = async () => { setSaving(true); try { await onSave(value.trim()); setEditing(false); } finally { setSaving(false); } };
  return <div className="transcript-edit-row">{editing ? <><textarea value={value} onChange={(event) => setValue(event.target.value)} aria-label={translated ? "编辑译文" : "编辑原文"} rows={2} /><button type="button" onClick={() => void save()} disabled={saving || !value.trim()} title="保存"><Save size={15} /></button><button type="button" onClick={() => setEditing(false)} title="取消">取消</button></> : <><span>{translated ? segment.translatedText || "尚未翻译" : segment.text}</span><button type="button" onClick={() => setEditing(true)} title={translated ? "编辑译文" : "编辑原文"}><Pencil size={14} /></button></>}</div>;
}

export function VideoDetailPage({ video, onBack, onRefresh, autoPlayOnTranscriptClick }: VideoDetailPageProps) {
  const videoRef = useRef<HTMLVideoElement>(null);
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
  const [tab, setTab] = useState<"transcript" | "note" | "summary">("transcript");
  const [displayTranslation, setDisplayTranslation] = useState(true);
  const [playing, setPlaying] = useState(false);
  const [muted, setMuted] = useState(false);
  const [durationMs, setDurationMs] = useState(0);
  const [currentMs, setCurrentMs] = useState(0);
  const [action, setAction] = useState<"translate" | "organize" | null>(null);
  const [activeSegmentId, setActiveSegmentId] = useState<string | null>(null);
  const [followPlayback, setFollowPlayback] = useState(true);

  useEffect(() => {
    let active = true;
    setLoading(true);
    setError("");
    setActiveSegmentId(null);
    setFollowPlayback(true);
    setCurrentMs(0);
    setDurationMs(0);
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

  const videoUrl = useMemo(() => media?.videoFile ? runtime.localAssetUrl(media.videoFile) : undefined, [media?.videoFile]);
  const segments = transcript?.segments ?? [];
  const visible = useMemo(() => {
    const filter = query.trim();
    return segments.filter((segment) => !filter || segment.text.includes(filter) || segment.translatedText?.includes(filter));
  }, [segments, query]);
  const canTranslate = Boolean(video.transcriptLanguage && !isChineseLanguage(video.transcriptLanguage));
  const hasTranslation = segments.some((segment) => Boolean(segment.translatedText?.trim()));

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
    setCurrentMs(current.currentTime * 1000);
    lastProgressRenderMs.current = current.currentTime * 1000;
  };

  useEffect(() => {
    if (tab !== "transcript" || !followPlayback || !activeSegmentId || !visible.some((segment) => segment.id === activeSegmentId)) return;
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
  }, [activeSegmentId, followPlayback, tab, visible, viewportHeight]);

  const editOriginal = async (segment: TranscriptSegment, text: string) => { await runtime.updateTranscriptSegment(video.id, segment.id, text); setNote(null); const latest = await runtime.loadTranscript(video.id); setTranscript(latest); await onRefresh(); };
  const editTranslation = async (segment: TranscriptSegment, text: string) => { await runtime.updateTranslationSegment(video.id, segment.id, text); setNote(null); const latest = await runtime.loadTranscript(video.id); setTranscript(latest); await onRefresh(); };
  const translate = async () => { setAction("translate"); try { await runtime.translateTranscript(video.id, () => undefined); await onRefresh(); setTranscript(await runtime.loadTranscript(video.id)); } catch (reason) { window.alert(formatErrorMessage(reason)); } finally { setAction(null); } };
  const organize = async () => { setAction("organize"); try { setNote(await runtime.organizeNotes(video, () => undefined, true)); await onRefresh(); } catch (reason) { window.alert(formatErrorMessage(reason)); } finally { setAction(null); } };
  const exportNote = async () => { if (!note) return; const path = await runtime.exportMarkdown(video.title.replace(/[\\/:*?"<>|]/g, "_") + ".md", note.markdown); if (path) window.alert("Markdown 已导出"); };

  if (loading) return <section className="standard-page page-frame"><div className="detail-loading"><LoaderCircle className="spin" size={25} />正在读取视频内容……</div></section>;

  const progressMax = durationMs || 0;
  return <section className="video-detail-page page-frame">
    <header className="video-detail-header"><button type="button" className="back-button" onClick={onBack}><ArrowLeft size={17} />返回视频库</button><div><h1>{video.title}</h1><p>{video.platform} · {video.duration} · {video.author || "未知作者"}</p></div><div className="video-detail-header-actions">{note ? <button type="button" className="secondary-button" onClick={() => void exportNote()}><Download size={16} />导出 Markdown</button> : null}</div></header>
    {error ? <div className="entry-message"><span className="error-message">{error}</span></div> : null}
    <div className="video-workspace">
      <section className="video-workspace-player-pane" aria-label="视频播放器">
        <div className="detail-player">
          <video ref={videoRef} src={videoUrl} poster={media?.thumbnailFile ? runtime.localAssetUrl(media.thumbnailFile) : video.thumbnailUrl || fallbackThumbnailUrl} onPlay={() => setPlaying(true)} onPause={() => setPlaying(false)} onEnded={() => setPlaying(false)} onLoadedMetadata={handleLoadedMetadata} onTimeUpdate={updatePlayback} controls={false} />
          <div className="detail-player-controls">
            <button type="button" onClick={() => { if (!videoRef.current) return; if (videoRef.current.paused) void videoRef.current.play(); else videoRef.current.pause(); }} aria-label={playing ? "暂停" : "播放"}>{playing ? <Pause size={17} /> : <Play size={17} fill="currentColor" />}</button>
            <button type="button" onClick={() => { if (!videoRef.current) return; videoRef.current.muted = !videoRef.current.muted; setMuted(videoRef.current.muted); }} aria-label={muted ? "取消静音" : "静音"}>{muted ? <VolumeX size={17} /> : <Volume2 size={17} />}</button>
            <input className="detail-player-progress" type="range" min="0" max={progressMax} step="100" value={Math.min(currentMs, progressMax || currentMs)} onChange={(event) => seekTo(Number(event.target.value))} aria-label="视频播放进度" aria-valuetext={time(currentMs) + " / " + time(durationMs)} disabled={!durationMs} />
            <span className="detail-player-time">{time(currentMs)} / {time(durationMs)}</span>
            <span className="detail-player-source">本地视频</span>
          </div>
        </div>
        <p className="detail-player-caption"><span className="detail-player-status-dot" />本地视频 · {video.platform} · {video.duration}</p>
      </section>

      <section className="video-workspace-content-pane" aria-label="视频内容">
        <div className="workspace-tabs" role="tablist" aria-label="视频内容类型">
          <button type="button" role="tab" aria-selected={tab === "transcript"} className={tab === "transcript" ? "is-active" : ""} onClick={() => setTab("transcript")}><FileText size={16} />字幕</button>
          <button type="button" role="tab" aria-selected={tab === "note"} className={tab === "note" ? "is-active" : ""} onClick={() => setTab("note")}><BookOpen size={16} />笔记</button>
          <button type="button" role="tab" aria-selected={tab === "summary"} className={tab === "summary" ? "is-active" : ""} onClick={() => setTab("summary")}><List size={16} />总结</button>
        </div>

        {tab === "transcript" ? <div className="workspace-transcript-pane">
          <div className="workspace-pane-toolbar">
            <label className="detail-search"><Search size={15} /><input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="搜索字幕内容" aria-label="搜索字幕内容" /></label>
            <div className="workspace-toolbar-actions">
              {hasTranslation ? <div className="translation-switch" role="group" aria-label="字幕显示方式"><button type="button" className={!displayTranslation ? "is-active" : ""} aria-pressed={!displayTranslation} onClick={() => setDisplayTranslation(false)}>原文</button><button type="button" className={displayTranslation ? "is-active" : ""} aria-pressed={displayTranslation} onClick={() => setDisplayTranslation(true)}>双语</button></div> : canTranslate ? <button type="button" className="secondary-button compact-button" disabled={action !== null} onClick={() => void translate()}>{action === "translate" ? <LoaderCircle size={15} className="spin" /> : <Languages size={15} />}开始翻译</button> : null}
              <button type="button" className={"transcript-follow-toggle " + (followPlayback ? "is-active" : "")} aria-pressed={followPlayback} onClick={() => setFollowPlayback((value) => !value)} title={followPlayback ? "关闭跟随播放" : "开启跟随播放"}><LocateFixed size={15} />跟随</button>
            </div>
          </div>
          <div className="workspace-pane-hint"><span>{segments.length} 段 · 校正字幕{hasTranslation ? " · 已有译文" : canTranslate ? " · 可翻译为中文" : ""}</span><span>点击时间跳转</span></div>
          <div className="detail-transcript" ref={transcriptRef} onWheel={() => setFollowPlayback(false)} onTouchStart={() => setFollowPlayback(false)} onPointerDown={() => setFollowPlayback(false)} onKeyDown={(event) => { if (["ArrowUp", "ArrowDown", "PageUp", "PageDown", "Home", "End", " ", "Enter"].includes(event.key)) setFollowPlayback(false); }}>
            {visible.length === 0 ? <div className="workspace-empty-state"><FileText size={25} /><p>{query.trim() ? "没有匹配的字幕" : "还没有字幕"}</p></div> : visible.map((segment) => { const active = activeSegmentId === segment.id; return <article className={"detail-segment " + (active ? "is-active" : "")} aria-current={active ? "true" : undefined} ref={(element) => { if (element) segmentRefs.current.set(segment.id, element); else segmentRefs.current.delete(segment.id); }} key={segment.id}><button type="button" className="segment-time" onClick={() => seek(segment)}>{time(segment.startMs)}</button><div className="segment-copy"><EditRow segment={segment} translated={false} onSave={(text) => editOriginal(segment, text)} />{displayTranslation && hasTranslation ? <EditRow segment={segment} translated onSave={(text) => editTranslation(segment, text)} /> : null}</div></article>; })}
          </div>
        </div> : <div className="workspace-document-pane">
          <div className="workspace-document-actions"><button type="button" className="primary-button" disabled={action !== null} onClick={() => void organize()}>{action === "organize" ? <LoaderCircle size={16} className="spin" /> : <RefreshCw size={16} />} {note ? "重新整理" : "整理笔记"}</button>{note ? <button type="button" className="secondary-button" onClick={() => void exportNote()}><Download size={16} />导出 Markdown</button> : null}</div>
          <div className="workspace-document-scroll">
            {note ? tab === "note" ? <><div className="workspace-document-heading"><BookOpen size={18} /><span>章节笔记</span></div>{note.chapters.length ? note.chapters.map((chapter) => <section className="workspace-chapter" key={String(chapter.timestampMs) + "-" + chapter.title}><button type="button" onClick={() => { setFollowPlayback(true); seekToMs(chapter.timestampMs, autoPlayOnTranscriptClick); }}>{time(chapter.timestampMs)}</button><h3>{chapter.title}</h3><p>{chapter.content}</p></section>) : <div className="workspace-empty-state"><p>暂无章节笔记</p></div>}</> : <><div className="workspace-document-heading"><List size={18} /><span>内容总结</span></div><section className="workspace-summary-block"><h2>摘要</h2><p>{note.summary}</p></section><section className="workspace-summary-block"><h2>核心要点</h2><ul>{note.keyPoints.map((point) => <li key={point}>{point}</li>)}</ul></section></> : <div className="workspace-empty-state"><FileText size={27} /><h2>{tab === "note" ? "还没有笔记" : "还没有总结"}</h2><p>点击上方按钮，根据当前校正字幕生成笔记和总结。</p></div>}
          </div>
        </div>}
      </section>
    </div>
  </section>;
}

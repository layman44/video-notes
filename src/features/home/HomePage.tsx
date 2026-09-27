import { Clipboard, Film, Link2, LoaderCircle, ListChecks, Library, Plus, Search } from "lucide-react";
import { useEffect, useState } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { VideoThumbnail } from "../../components/VideoThumbnail";
import type { QueueItem, SourcePreview, Video } from "../../types";
import { runtime } from "../../lib/runtime";

interface HomePageProps {
  queueItems: QueueItem[];
  videos: Video[];
  videoCount: number;
  onEnqueue: (source: SourcePreview) => Promise<void>;
  onEnqueueBatch?: (sources: SourcePreview[]) => Promise<void>;
  onSearch?: (query: string) => void;
  onOpenQueue: () => void;
  onOpenLibrary: () => void;
  onOpenVideo: (video: Video) => void;
}

function isDirectSource(value: string): boolean {
  const trimmed = value.trim();
  if (!trimmed) return false;
  if (/^https?:\/\//i.test(trimmed) || /^file:\/\//i.test(trimmed)) return true;
  if (/https?:\/\/[^\s]+/i.test(trimmed)) return true;
  if (/^BV[\w]+$/i.test(trimmed)) return true;
  if (/^"?[a-zA-Z]:[\\/]/i.test(trimmed)) return true;
  if (/\.(mp4|mkv|mov|avi|webm|flv|m4v|wmv|ts|mp3|m4a|wav|aac|flac|ogg|opus|wma)"?$/i.test(trimmed)) return true;
  return false;
}

export function HomePage({ queueItems, videos, videoCount, onEnqueue, onEnqueueBatch, onSearch, onOpenQueue, onOpenLibrary, onOpenVideo }: HomePageProps) {
  const [input, setInput] = useState("");
  const [error, setError] = useState("");
  const [isParsing, setIsParsing] = useState(false);
  const [isDragOver, setIsDragOver] = useState(false);

  const isDirect = isDirectSource(input);

  const handleSubmit = async () => {
    const trimmed = input.trim();
    if (!trimmed) {
      setError("请输入视频链接、搜索关键词，或直接将视频拖入此处");
      return;
    }
    setError("");
    if (isDirect) {
      setIsParsing(true);
      try {
        const preview = await runtime.parseSource(trimmed);
        await onEnqueue(preview);
        setInput("");
      } catch (reason) {
        setError(reason instanceof Error ? reason.message : "暂时无法解析该链接或文件");
      } finally {
        setIsParsing(false);
      }
    } else {
      if (onSearch) {
        onSearch(trimmed);
      }
    }
  };

  const pasteFromClipboard = async () => {
    try {
      const text = await navigator.clipboard.readText();
      setInput(text);
      setError("");
    } catch {
      setError("无法读取剪贴板，请使用 Ctrl+V 粘贴");
    }
  };

  useEffect(() => {
    if (!runtime.isDesktop()) return undefined;
    let active = true;
    let unlisten: (() => void) | undefined;
    void getCurrentWebview().onDragDropEvent(async (event) => {
      if (!active) return;
      if (event.payload.type === "enter" || event.payload.type === "over") {
        setIsDragOver(true);
      } else if (event.payload.type === "leave") {
        setIsDragOver(false);
      } else if (event.payload.type === "drop") {
        setIsDragOver(false);
        const paths = event.payload.paths;
        if (!paths || paths.length === 0) return;
        setIsParsing(true);
        setError("");
        try {
          const previews: SourcePreview[] = [];
          for (const path of paths) {
            try {
              const preview = await runtime.parseSource(path);
              previews.push(preview);
            } catch {
              // 忽略批量拖拽中不支持的文件类型
            }
          }
          if (previews.length > 0) {
            if (onEnqueueBatch) {
              await onEnqueueBatch(previews);
            } else {
              for (const preview of previews) {
                await onEnqueue(preview);
              }
            }
          } else {
            setError("所拖放的文件不支持或无法解析");
          }
        } catch (reason) {
          setError(reason instanceof Error ? reason.message : "导入拖放文件失败");
        } finally {
          if (active) setIsParsing(false);
        }
      }
    }).then((fn) => {
      if (active) unlisten = fn;
      else fn();
    });
    return () => {
      active = false;
      unlisten?.();
    };
  }, [onEnqueue, onEnqueueBatch]);

  return (
    <section className="home-page page-frame">
      <div className="home-intro">
        <h1>把视频变成可检索的笔记</h1>
        <p>支持粘贴视频链接（B站、抖音、YouTube 等主流平台）或导入本地音视频文件，转写与整理均在本机离线完成。</p>
      </div>
      <div className="source-entry">
        <div
          className={`url-field ${error ? "has-error" : ""} ${isDragOver ? "is-drag-over" : ""}`}
          onDragEnter={(e) => { e.preventDefault(); setIsDragOver(true); }}
          onDragOver={(e) => { e.preventDefault(); setIsDragOver(true); }}
          onDragLeave={(e) => { e.preventDefault(); setIsDragOver(false); }}
        >
          {isDragOver ? (
            <div className="dropzone-hint">
              <Film size={21} strokeWidth={1.8} aria-hidden="true" />
              <span>松开鼠标，直接导入音视频到转录队列</span>
            </div>
          ) : (
            <>
              {isDirect ? (
                <Link2 size={21} strokeWidth={1.8} aria-hidden="true" />
              ) : (
                <Search size={21} strokeWidth={1.8} aria-hidden="true" />
              )}
              <input
                value={input}
                onChange={(event) => { setInput(event.target.value); if (error) setError(""); }}
                onKeyDown={(event) => { if (event.key === "Enter") void handleSubmit(); }}
                aria-label="视频链接或搜索关键词"
                placeholder="粘贴视频链接、搜索关键词，或直接将音视频拖入此处…"
                autoFocus
              />
            </>
          )}
        </div>
        <button
          className="primary-button parse-button"
          type="button"
          onClick={() => void handleSubmit()}
          disabled={isParsing || isDragOver}
        >
          {isParsing ? (
            <LoaderCircle className="spin" size={18} aria-hidden="true" />
          ) : isDirect || !input.trim() ? (
            <Plus size={18} aria-hidden="true" />
          ) : (
            <Search size={18} aria-hidden="true" />
          )}
          {isParsing ? "正在解析" : isDirect || !input.trim() ? "加入队列" : "搜索视频"}
        </button>
        <div className="entry-message" role="status" aria-live="polite">
          {error ? <span className="error-message">{error}</span> : null}
        </div>
        <div className="source-actions">
          <button className="clipboard-button" type="button" onClick={() => void pasteFromClipboard()}>
            <Clipboard size={16} strokeWidth={1.8} aria-hidden="true" />从剪贴板粘贴
          </button>
        </div>
      </div>
      <section className="home-summary-grid" aria-label="内容概览">
        <button className="home-summary-card" type="button" onClick={onOpenQueue}>
          <span className="home-summary-icon"><ListChecks size={19} /></span>
          <span><strong>{queueItems.filter((item) => ["queued", "running", "paused", "blocked", "failed"].includes(item.state)).length}</strong><small>队列中的视频</small></span>
          <span className="home-summary-link">查看队列 →</span>
        </button>
        <button className="home-summary-card" type="button" onClick={onOpenLibrary}>
          <span className="home-summary-icon"><Library size={19} /></span>
          <span><strong>{videoCount}</strong><small>视频库内容</small></span>
          <span className="home-summary-link">打开视频库 →</span>
        </button>
      </section>
      <section className="recent-section">
        <div className="section-heading-row">
          <h2>最近进入视频库</h2>
          {videoCount > 0 ? <button className="view-all-link" type="button" onClick={onOpenLibrary}>查看全部 ({videoCount}) →</button> : null}
        </div>
        {videos.length > 0 ? (
          <div className="library-card-list">
            {videos.slice(0, 4).map((video) => (
              <button className="library-card" type="button" key={video.id} onClick={() => onOpenVideo(video)}>
                <VideoThumbnail className="library-card-thumb" src={video.thumbnailUrl} />
                <span className="library-card-copy">
                  <strong title={video.title}>{video.title}</strong>
                  <small>{video.platform === "local" ? "本地媒体" : video.platform} · {video.duration}</small>
                </span>
              </button>
            ))}
          </div>
        ) : (
          <div className="home-empty-state">
            <Library size={24} />
            <p>完成转录的视频会出现在这里</p>
          </div>
        )}
      </section>
    </section>
  );
}

import { Clipboard, Link2, LoaderCircle, ListChecks, Library } from "lucide-react";
import { useState } from "react";
import type { QueueItem, SourcePreview, Video } from "../../types";
import { runtime } from "../../lib/runtime";

interface HomePageProps { queueItems: QueueItem[]; videos: Video[]; onEnqueue: (source: SourcePreview) => Promise<void>; onOpenQueue: () => void; onOpenLibrary: () => void; onOpenVideo: (video: Video) => void; }

export function HomePage({ queueItems, videos, onEnqueue, onOpenQueue, onOpenLibrary, onOpenVideo }: HomePageProps) {
  const [input, setInput] = useState("");
  const [error, setError] = useState("");
  const [isParsing, setIsParsing] = useState(false);
  const parseInput = async () => {
    if (!input.trim()) { setError("请先粘贴视频链接或分享文本"); return; }
    setError(""); setIsParsing(true);
    try { const preview = await runtime.parseSource(input.trim()); await onEnqueue(preview); setInput(""); }
    catch (reason) { setError(reason instanceof Error ? reason.message : "暂时无法解析该链接"); }
    finally { setIsParsing(false); }
  };
  const pasteFromClipboard = async () => { try { setInput(await navigator.clipboard.readText()); setError(""); } catch { setError("无法读取剪贴板，请使用 Ctrl+V 粘贴"); } };
  return <section className="home-page page-frame">
    <div className="home-intro"><h1>把视频变成可检索的笔记</h1><p>粘贴抖音或哔哩哔哩链接，下载与转写均在本机按队列完成。</p></div>
    <div className="source-entry">
      <div className={`url-field ${error ? "has-error" : ""}`}><Link2 size={21} strokeWidth={1.8} aria-hidden="true" /><input value={input} onChange={(event) => { setInput(event.target.value); if (error) setError(""); }} onKeyDown={(event) => { if (event.key === "Enter") void parseInput(); }} aria-label="视频链接或分享文本" placeholder="粘贴视频链接或分享文本" autoFocus /></div>
      <button className="primary-button parse-button" type="button" onClick={() => void parseInput()} disabled={isParsing}>{isParsing ? <LoaderCircle className="spin" size={18} aria-hidden="true" /> : null}{isParsing ? "正在解析" : "加入队列"}</button>
      <button className="clipboard-button" type="button" onClick={() => void pasteFromClipboard()}><Clipboard size={16} strokeWidth={1.8} aria-hidden="true" />从剪贴板粘贴</button>
      <div className="entry-message" role="status" aria-live="polite">{error ? <span className="error-message">{error}</span> : null}</div>
    </div>
    <section className="home-summary-grid" aria-label="内容概览">
      <button className="home-summary-card" type="button" onClick={onOpenQueue}><span className="home-summary-icon"><ListChecks size={19} /></span><span><strong>{queueItems.filter((item) => ["queued", "running", "paused", "blocked", "failed"].includes(item.state)).length}</strong><small>队列中的视频</small></span><span className="home-summary-link">查看队列 →</span></button>
      <button className="home-summary-card" type="button" onClick={onOpenLibrary}><span className="home-summary-icon"><Library size={19} /></span><span><strong>{videos.length}</strong><small>视频库内容</small></span><span className="home-summary-link">打开视频库 →</span></button>
    </section>
    <section className="recent-section"><div className="section-heading-row"><h2>最近进入视频库</h2>{videos.length > 0 ? <button className="view-all-link" type="button" onClick={onOpenLibrary}>查看全部 ({videos.length}) →</button> : null}</div>
      {videos.length > 0 ? <div className="library-card-list">{videos.slice(0, 5).map((video) => <button className="library-card" type="button" key={video.id} onClick={() => onOpenVideo(video)}><span className="library-card-thumb">{video.thumbnailUrl ? <img src={video.thumbnailUrl} alt="" /> : <Library size={20} />}</span><span className="library-card-copy"><strong>{video.title}</strong><small>{video.platform} · {video.duration}</small></span></button>)}</div> : <div className="home-empty-state"><Library size={24} /><p>完成转录的视频会出现在这里</p></div>}
    </section>
  </section>;
}

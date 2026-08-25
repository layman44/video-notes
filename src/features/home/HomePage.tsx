import { Clipboard, Link2, LoaderCircle } from "lucide-react";
import { useState } from "react";
import { JobTable, type JobActionHandlers } from "../../components/JobTable";
import { runtime } from "../../lib/runtime";
import type { Job, SourcePreview } from "../../types";

interface HomePageProps {
  jobs: Job[];
  onOpenJob: (job: Job) => void;
  onStart: (preview: SourcePreview) => void;
  onViewAll?: () => void;
  jobActions: JobActionHandlers;
}

export function HomePage({ jobs, onOpenJob, onStart, onViewAll, jobActions }: HomePageProps) {
  const [input, setInput] = useState("");
  const [error, setError] = useState("");
  const [isParsing, setIsParsing] = useState(false);

  const parseInput = async () => {
    if (!input.trim()) {
      setError("请先粘贴视频链接或分享文本");
      return;
    }

    setError("");
    setIsParsing(true);
    try {
      const preview = await runtime.parseSource(input.trim());
      onStart(preview);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "暂时无法解析该链接");
    } finally {
      setIsParsing(false);
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

  return (
    <section className="home-page page-frame">
      <div className="home-intro">
        <h1>把视频变成可检索的笔记</h1>
        <p>粘贴抖音或哔哩哔哩链接，转写与整理均在本机完成。</p>
      </div>

      <div className="source-entry">
        <div className={`url-field ${error ? "has-error" : ""}`}>
          <Link2 size={21} strokeWidth={1.8} aria-hidden="true" />
          <input
            value={input}
            onChange={(event) => {
              setInput(event.target.value);
              if (error) setError("");
            }}
            onKeyDown={(event) => {
              if (event.key === "Enter") void parseInput();
            }}
            aria-label="视频链接或分享文本"
            placeholder="粘贴视频链接或分享文本"
            autoFocus
          />
        </div>
        <button className="primary-button parse-button" type="button" onClick={() => void parseInput()} disabled={isParsing}>
          {isParsing ? <LoaderCircle className="spin" size={18} aria-hidden="true" /> : null}
          {isParsing ? "正在解析" : "开始解析"}
        </button>
        <button className="clipboard-button" type="button" onClick={() => void pasteFromClipboard()}>
          <Clipboard size={16} strokeWidth={1.8} aria-hidden="true" />
          从剪贴板粘贴
        </button>
        <div className="entry-message" role="status" aria-live="polite">
          {error ? <span className="error-message">{error}</span> : null}
        </div>
      </div>

      <section className="recent-section">
        <div className="section-heading-row">
          <h2>最近任务</h2>
          {onViewAll && jobs.length > 5 ? (
            <button className="view-all-link" type="button" onClick={onViewAll}>
              查看全部 ({jobs.length}) →
            </button>
          ) : null}
        </div>
        <JobTable jobs={jobs.slice(0, 5)} onOpen={onOpenJob} {...jobActions} />
      </section>
    </section>
  );
}

import { ArrowDown, ArrowUp, CheckCircle2, ChevronLeft, ChevronRight, CircleAlert, CirclePause, CirclePlay, Clock, LoaderCircle, Pause, Pin, Play, RotateCcw, Trash2, XCircle } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { loadAsrSettings } from "../../lib/preferences";
import { runtime } from "../../lib/runtime";
import { toast } from "../../lib/toast";
import type { QueueItem, QueueState } from "../../types";

interface QueuePageProps { items: QueueItem[]; onRefresh: () => Promise<void>; }
type Filter = "active" | QueueState | "all";
const filters: Array<{ id: Filter; label: string }> = [{ id: "active", label: "进行中" }, { id: "queued", label: "等待" }, { id: "paused", label: "暂停" }, { id: "blocked", label: "等待模型" }, { id: "failed", label: "失败" }, { id: "completed", label: "完成" }, { id: "all", label: "全部" }];
function progress(item: QueueItem) { if (item.state === "completed") return 100; const completed = item.progressCompleted ?? item.progress; const total = item.progressTotal ?? 100; if (completed == null || total <= 0) return 0; return Math.round(Math.max(0, Math.min(100, completed / total * 100))); }
function stateLabel(state: QueueState) { return ({ queued: "等待中", running: "处理中", paused: "已暂停", blocked: "等待模型", failed: "处理失败", completed: "已完成", cancelled: "已取消" })[state]; }
function stageLabel(stage?: QueueItem["stage"]) { return ({ download: "下载", normalize: "音频处理", transcribe: "转录" })[stage ?? "download"] ?? "等待"; }

export function QueuePage({ items, onRefresh }: QueuePageProps) {
  const [filter, setFilter] = useState<Filter>("active");
  const [page, setPage] = useState(1);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [cancelCandidate, setCancelCandidate] = useState<QueueItem | null>(null);
  const visible = useMemo(() => { const filtered = filter === "all" ? items : filter === "active" ? items.filter((item) => ["queued", "running", "paused", "blocked", "failed"].includes(item.state)) : items.filter((item) => item.state === filter); return [...filtered].sort((a, b) => a.position - b.position); }, [filter, items]);
  const pageCount = Math.max(1, Math.ceil(visible.length / 10));
  useEffect(() => { setPage((current) => Math.min(Math.max(1, current), pageCount)); }, [pageCount]);
  const pageItems = visible.slice((page - 1) * 10, page * 10);
  const setFilterAndReset = (next: Filter) => { setFilter(next); setPage(1); };
  const run = async (id: string, action: () => Promise<void>) => { setBusyId(id); try { await action(); await onRefresh(); } catch (reason) { toast.error(reason instanceof Error ? reason.message : "操作失败"); } finally { setBusyId(null); } };

  const handleRemove = (item: QueueItem) => {
    if (item.state === "running") {
      setCancelCandidate(item);
    } else {
      void run(item.id, async () => {
        await runtime.removeQueueItem(item.id);
        toast.info(`已从队列移除「${item.title}」`);
      });
    }
  };

  const confirmCancelRunning = () => {
    if (!cancelCandidate) return;
    const item = cancelCandidate;
    setCancelCandidate(null);
    void run(item.id, async () => {
      await runtime.removeQueueItem(item.id);
      toast.info(`已终止并移除正在运行的任务「${item.title}」`);
    });
  };

  return <section className="standard-page page-frame queue-page"><header className="page-header"><div><h1>队列</h1><p>支持后台多任务并发下载，转录任务单线程独占按序进行。</p></div><span className="queue-count-badge">{items.filter((item) => ["queued", "running", "paused", "blocked", "failed"].includes(item.state)).length} 个待处理</span></header>
    <div className="queue-filter-tabs" role="tablist" aria-label="队列筛选">{filters.map((item) => <button type="button" role="tab" aria-selected={filter === item.id} className={filter === item.id ? "is-active" : ""} key={item.id} onClick={() => setFilterAndReset(item.id)}>{item.label}<span>{item.id === "active" ? items.filter((entry) => ["queued", "running", "paused", "blocked", "failed"].includes(entry.state)).length : item.id === "all" ? items.length : items.filter((entry) => entry.state === item.id).length}</span></button>)}</div>
    <div className="queue-body-scroll">
      <div className="queue-list" aria-live="polite">{pageItems.length === 0 ? <div className="empty-state"><ListChecksIcon /><h2>这里还没有视频</h2><p>从首页粘贴链接，或在搜索结果中批量加入队列。</p></div> : pageItems.map((item, itemIndex) => { const percent = progress(item); const isBusy = busyId === item.id; return <article className="queue-item-card" key={item.id}><div className="queue-item-position">{(page - 1) * 10 + itemIndex + 1}</div><div className="queue-item-thumb">{item.thumbnailUrl ? <img src={item.thumbnailUrl} alt="" referrerPolicy="no-referrer" /> : <CirclePlay size={21} />}</div><div className="queue-item-main"><div className="queue-item-heading"><h2 title={item.title}>{item.title}</h2><span className={`queue-state queue-state-${item.state}`}>{item.state === "running" ? <LoaderCircle size={11} className="spin" /> : item.state === "queued" ? <Clock size={11} /> : item.state === "paused" ? <CirclePause size={11} /> : item.state === "failed" || item.state === "blocked" ? <CircleAlert size={11} /> : item.state === "completed" ? <CheckCircle2 size={11} /> : item.state === "cancelled" ? <XCircle size={11} /> : null}{stateLabel(item.state)}</span></div><div className="queue-item-meta"><span>{item.platform}</span><span>{item.duration}</span>{item.attemptCount > 0 ? <span>第 {item.attemptCount} 次尝试</span> : null}{item.stage ? <span>当前阶段：{stageLabel(item.stage)}</span> : null}</div>{item.statusMessage || item.errorMessage ? <p className={item.state === "blocked" ? "queue-item-warning" : item.state === "paused" ? "queue-item-paused" : item.state === "failed" ? "queue-item-error" : "queue-item-message"}>{item.errorMessage || item.statusMessage}</p> : null}{percent > 0 ? <div className="queue-progress"><div className="queue-progress-track"><span style={{ width: `${percent}%` }} /></div><strong>{percent}%</strong></div> : null}</div><div className="queue-item-actions">{item.state === "running" ? <button type="button" title="暂停" aria-label={`暂停 ${item.title}`} disabled={isBusy} onClick={() => void run(item.id, async () => { await runtime.pauseQueueItem(item.id); toast.info(`已暂停「${item.title}」`); })}><Pause size={16} /></button> : null}{item.state === "paused" ? <button type="button" title="继续" aria-label={`继续 ${item.title}`} disabled={isBusy} onClick={() => void run(item.id, async () => { await runtime.resumeQueueItem(item.id); toast.info(`已恢复「${item.title}」`); })}><Play size={16} /></button> : null}{["failed", "blocked", "cancelled"].includes(item.state) ? <button type="button" title="重试" aria-label={`重试 ${item.title}`} disabled={isBusy} onClick={() => void run(item.id, async () => { const settings = loadAsrSettings(); await runtime.retryQueueItem(item.id, settings.backend, JSON.stringify(settings.moss)); toast.info(`已重新加入处理「${item.title}」`); })}><RotateCcw size={16} /></button> : null}{["queued", "paused", "failed", "blocked", "cancelled", "completed"].includes(item.state) || item.state === "running" ? <button type="button" title="移除队列" aria-label={`从队列移除 ${item.title}`} disabled={isBusy} onClick={() => handleRemove(item)}><Trash2 size={16} /></button> : null}{item.state === "queued" ? <><button type="button" title="置顶" aria-label={`置顶 ${item.title}`} disabled={isBusy || visible[0]?.id === item.id} onClick={() => void run(item.id, async () => { await runtime.moveQueueItem(item.id, "top"); toast.info(`已将「${item.title}」置顶`); })}><Pin size={16} /></button><button type="button" title="上移" aria-label={`上移 ${item.title}`} disabled={isBusy} onClick={() => void run(item.id, () => runtime.moveQueueItem(item.id, "up"))}><ArrowUp size={16} /></button><button type="button" title="下移" aria-label={`下移 ${item.title}`} disabled={isBusy} onClick={() => void run(item.id, () => runtime.moveQueueItem(item.id, "down"))}><ArrowDown size={16} /></button></> : null}</div></article>; })}</div>
      {pageCount > 1 ? <div className="content-pagination"><div className="pagination-info">第 {page} / {pageCount} 页</div><div className="pagination-controls"><button type="button" className="pagination-nav-button" disabled={page <= 1} onClick={() => setPage((value) => value - 1)}><ChevronLeft size={15} />上一页</button><button type="button" className="pagination-nav-button" disabled={page >= pageCount} onClick={() => setPage((value) => value + 1)}>下一页<ChevronRight size={15} /></button></div></div> : null}
    </div>
    {cancelCandidate ? (
      <ConfirmDialog
        title="中断并移除任务"
        message="该任务当前正在处理中，确认中断转录并从队列中移除吗？"
        highlightText={cancelCandidate.title}
        confirmText="确定中断并移除"
        isDanger
        onConfirm={confirmCancelRunning}
        onCancel={() => setCancelCandidate(null)}
      />
    ) : null}
  </section>;
}
function ListChecksIcon() { return <CirclePlay size={34} strokeWidth={1.5} aria-hidden="true" />; }

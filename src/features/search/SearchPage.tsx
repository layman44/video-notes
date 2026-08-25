import {
  ArrowDown,
  ArrowUp,
  Check,
  ChevronLeft,
  ChevronRight,
  Clock,
  ExternalLink,
  Film,
  History,
  LoaderCircle,
  Minus,
  Play,
  Plus,
  Search,
  Trash2,
  X,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { runtime } from "../../lib/runtime";
import type {
  Job,
  SearchDurationFilter,
  SearchOrder,
  SearchResultItem,
  SearchResultResponse,
} from "../../types";

interface SearchPageProps {
  jobs: Job[];
  onBatchAddJobs: (items: SearchResultItem[]) => void;
  onStartSingleJob: (item: SearchResultItem) => void;
  onOpenJob: (job: Job) => void;
}

type ClientSortKey = "title" | "author" | "duration" | "playCount" | "pubDate";

const orderOptions: Array<{ id: SearchOrder; label: string }> = [
  { id: "totalrank", label: "综合排序" },
  { id: "click", label: "最多播放" },
  { id: "pubdate", label: "最新发布" },
  { id: "stow", label: "最多收藏" },
  { id: "dm", label: "最多弹幕" },
];

const durationOptions: Array<{ id: SearchDurationFilter; label: string }> = [
  { id: 0, label: "全部时长" },
  { id: 1, label: "< 10分钟" },
  { id: 2, label: "10-30分钟" },
  { id: 3, label: "30-60分钟" },
  { id: 4, label: "> 60分钟" },
];

// 模块级缓存：保证离开搜索页再切换回来时，搜索结果、关键词、分页与勾选状态 100% 完整保留
interface SavedSearchSession {
  keyword: string;
  order: SearchOrder;
  durationFilter: SearchDurationFilter;
  page: number;
  totalPages: number;
  totalCount: number;
  rawResults: SearchResultItem[];
  hasSearched: boolean;
  selectedIds: string[];
}

let savedSearchSession: SavedSearchSession | null = null;

const SEARCH_HISTORY_KEY = "videonotes_search_history_v1";

function loadSearchHistory(): string[] {
  try {
    const raw = localStorage.getItem(SEARCH_HISTORY_KEY);
    if (raw) {
      const parsed = JSON.parse(raw);
      if (Array.isArray(parsed)) {
        return parsed.filter((item): item is string => typeof item === "string" && item.trim().length > 0).slice(0, 8);
      }
    }
  } catch {}
  return [];
}

function saveSearchHistory(history: string[]): void {
  try {
    localStorage.setItem(SEARCH_HISTORY_KEY, JSON.stringify(history.slice(0, 8)));
  } catch {}
}

// 辅助解析时长字符串（如 "15:20" 或 "01:10:05"）为总秒数以支持排序
function parseDurationSecs(str?: string): number {
  if (!str) return 0;
  const parts = str.split(":").map((p) => parseInt(p, 10) || 0);
  if (parts.length === 3) return parts[0] * 3600 + parts[1] * 60 + parts[2];
  if (parts.length === 2) return parts[0] * 60 + parts[1];
  return 0;
}

// 辅助解析播放量（如 "12.5万"、"1.2亿"、"5400"）为纯数值以支持排序
function parsePlayCountNum(str?: string | null): number {
  if (!str) return 0;
  if (str.includes("亿")) return parseFloat(str.replace("亿", "")) * 100_000_000;
  if (str.includes("万")) return parseFloat(str.replace("万", "")) * 10_000;
  return parseFloat(str) || 0;
}

// 辅助匹配搜索结果是否已存在于任务列表中（支持完整链接与 BV 号双重匹配）
function findMatchingJob(item: SearchResultItem, jobs: Job[]): Job | undefined {
  const cleanItemUrl = item.videoUrl.trim();
  const bvMatch = cleanItemUrl.match(/BV[a-zA-Z0-9]+/i) || item.id.match(/BV[a-zA-Z0-9]+/i);
  return jobs.find((j) => {
    if (j.sourceUrl === cleanItemUrl || j.sourceUrl === item.id) return true;
    if (bvMatch && j.sourceUrl.includes(bvMatch[0])) return true;
    return false;
  });
}

export function SearchPage({ jobs, onBatchAddJobs, onStartSingleJob, onOpenJob }: SearchPageProps) {
  const [keyword, setKeyword] = useState(() => savedSearchSession?.keyword ?? "");
  const [order, setOrder] = useState<SearchOrder>(() => savedSearchSession?.order ?? "totalrank");
  const [durationFilter, setDurationFilter] = useState<SearchDurationFilter>(() => savedSearchSession?.durationFilter ?? 0);
  const [page, setPage] = useState(() => savedSearchSession?.page ?? 1);
  const [totalPages, setTotalPages] = useState(() => savedSearchSession?.totalPages ?? 1);
  const [totalCount, setTotalCount] = useState(() => savedSearchSession?.totalCount ?? 0);

  const [isSearching, setIsSearching] = useState(false);
  const [rawResults, setRawResults] = useState<SearchResultItem[]>(() => savedSearchSession?.rawResults ?? []);
  const [hasSearched, setHasSearched] = useState(() => savedSearchSession?.hasSearched ?? false);
  const [error, setError] = useState("");
  const [selectedIds, setSelectedIds] = useState<Set<string>>(() => new Set(savedSearchSession?.selectedIds ?? []));
  const [cooldownRemaining, setCooldownRemaining] = useState(0);

  const [searchHistory, setSearchHistory] = useState<string[]>(loadSearchHistory);

  // 表格即时列头排序（升序 / 降序）
  const [clientSortKey, setClientSortKey] = useState<ClientSortKey | null>(null);
  const [clientSortDir, setClientSortDir] = useState<"asc" | "desc">("desc");

  const searchCache = useRef<Map<string, { data: SearchResultResponse; time: number }>>(new Map());

  // 状态同步至模块缓存，使路由切换不丢失
  useEffect(() => {
    savedSearchSession = {
      keyword,
      order,
      durationFilter,
      page,
      totalPages,
      totalCount,
      rawResults,
      hasSearched,
      selectedIds: Array.from(selectedIds),
    };
  }, [keyword, order, durationFilter, page, totalPages, totalCount, rawResults, hasSearched, selectedIds]);

  const saveHistoryKeyword = (kw: string) => {
    const clean = kw.trim();
    if (!clean || clean.startsWith("http") || clean.startsWith("BV") || clean.startsWith("bv")) return;
    setSearchHistory((prev) => {
      const next = [clean, ...prev.filter((k) => k !== clean)].slice(0, 8);
      saveSearchHistory(next);
      return next;
    });
  };

  const handleDeleteHistoryItem = (itemToDelete: string) => {
    setSearchHistory((prev) => {
      const next = prev.filter((k) => k !== itemToDelete);
      saveSearchHistory(next);
      return next;
    });
  };

  const handleClearAllHistory = () => {
    setSearchHistory([]);
    saveSearchHistory([]);
  };

  const handleHistoryTagClick = (kw: string) => {
    setKeyword(kw);
    setPage(1);
    saveHistoryKeyword(kw);
    void executeSearch(kw, order, durationFilter, 1);
  };

  // 冷却倒计时
  useEffect(() => {
    if (cooldownRemaining <= 0) return;
    const timer = window.setInterval(() => {
      setCooldownRemaining((prev) => {
        if (prev <= 1) {
          clearInterval(timer);
          return 0;
        }
        return prev - 1;
      });
    }, 1000);
    return () => clearInterval(timer);
  }, [cooldownRemaining]);

  const executeSearch = async (
    targetKeyword: string,
    targetOrder: SearchOrder,
    targetDuration: SearchDurationFilter,
    targetPage: number,
    forceRefresh = false
  ) => {
    const query = targetKeyword.trim();
    if (!query) {
      setError("请先输入要搜索的视频关键词或链接");
      return;
    }

    const cacheKey = `${query}:${targetOrder}:${targetDuration}:${targetPage}`;
    const cached = searchCache.current.get(cacheKey);
    if (!forceRefresh && cached && Date.now() - cached.time < 5 * 60 * 1000) {
      setRawResults(cached.data.items);
      setTotalPages(cached.data.totalPages);
      setTotalCount(cached.data.totalCount);
      setPage(cached.data.page);
      setHasSearched(true);
      setError("");
      return;
    }

    setError("");
    setIsSearching(true);
    setSelectedIds(new Set());

    // 2 到 5 秒随机频控
    const randomCooldown = Math.floor(Math.random() * 4) + 2;

    try {
      if (
        query.startsWith("http://") ||
        query.startsWith("https://") ||
        query.startsWith("BV") ||
        query.startsWith("bv")
      ) {
        try {
          const direct = await runtime.parseSource(query);
          const directItem: SearchResultItem = {
            id: direct.sourceUrl,
            title: direct.title,
            author: direct.author ?? "作者",
            platform: direct.platform,
            duration: direct.duration,
            coverUrl: direct.thumbnailUrl,
            videoUrl: direct.sourceUrl,
            playCount: "直达解析",
            pubDate: "刚刚",
          };
          setRawResults([directItem]);
          setTotalPages(1);
          setTotalCount(1);
          setPage(1);
          setHasSearched(true);
          return;
        } catch {
          // 降级继续关键词搜索
        }
      }

      const res = await runtime.searchVideos(
        query,
        targetOrder,
        targetDuration === 0 ? undefined : targetDuration,
        targetPage
      );

      setRawResults(res.items);
      setTotalPages(Math.min(res.totalPages || 1, 50)); // B站通常提供前50页
      setTotalCount(res.totalCount || res.items.length);
      setPage(res.page || targetPage);
      setHasSearched(true);

      searchCache.current.set(cacheKey, { data: res, time: Date.now() });
      setCooldownRemaining(randomCooldown);
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setError(msg || "搜索失败，请稍后重试");
      setCooldownRemaining(randomCooldown);
    } finally {
      setIsSearching(false);
    }
  };

  const handleFormSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (cooldownRemaining > 0) return;
    setPage(1);
    saveHistoryKeyword(keyword);
    void executeSearch(keyword, order, durationFilter, 1);
  };

  const handleOrderChange = (newOrder: SearchOrder) => {
    if (newOrder === order) return;
    setOrder(newOrder);
    setPage(1);
    if (keyword.trim()) {
      void executeSearch(keyword, newOrder, durationFilter, 1);
    }
  };

  const handleDurationChange = (newDur: SearchDurationFilter) => {
    if (newDur === durationFilter) return;
    setDurationFilter(newDur);
    setPage(1);
    if (keyword.trim()) {
      void executeSearch(keyword, order, newDur, 1);
    }
  };

  const handlePageChange = (newPage: number) => {
    if (newPage === page || newPage < 1 || newPage > totalPages) return;
    setPage(newPage);
    if (keyword.trim()) {
      void executeSearch(keyword, order, durationFilter, newPage);
    }
  };

  // 点击列头切换客户端快速排序
  const handleSortClick = (key: ClientSortKey) => {
    if (clientSortKey === key) {
      if (clientSortDir === "desc") {
        setClientSortDir("asc");
      } else {
        setClientSortKey(null);
      }
    } else {
      setClientSortKey(key);
      setClientSortDir("desc");
    }
  };

  const displayedResults = useMemo(() => {
    if (!clientSortKey) return rawResults;
    const sorted = [...rawResults];
    sorted.sort((a, b) => {
      let valA: any = "";
      let valB: any = "";

      if (clientSortKey === "title") {
        valA = a.title;
        valB = b.title;
        return clientSortDir === "asc"
          ? valA.localeCompare(valB, "zh-CN")
          : valB.localeCompare(valA, "zh-CN");
      }
      if (clientSortKey === "author") {
        valA = a.author;
        valB = b.author;
        return clientSortDir === "asc"
          ? valA.localeCompare(valB, "zh-CN")
          : valB.localeCompare(valA, "zh-CN");
      }
      if (clientSortKey === "duration") {
        valA = parseDurationSecs(a.duration);
        valB = parseDurationSecs(b.duration);
        return clientSortDir === "asc" ? valA - valB : valB - valA;
      }
      if (clientSortKey === "playCount") {
        valA = parsePlayCountNum(a.playCount);
        valB = parsePlayCountNum(b.playCount);
        return clientSortDir === "asc" ? valA - valB : valB - valA;
      }
      if (clientSortKey === "pubDate") {
        valA = a.pubDate ?? "";
        valB = b.pubDate ?? "";
        return clientSortDir === "asc" ? valA.localeCompare(valB) : valB.localeCompare(valA);
      }
      return 0;
    });
    return sorted;
  }, [rawResults, clientSortKey, clientSortDir]);

  const allDisplayedSelected =
    displayedResults.length > 0 && selectedIds.size === displayedResults.length;
  const someDisplayedSelected = selectedIds.size > 0 && !allDisplayedSelected;

  const toggleSelect = (id: string, event?: React.MouseEvent) => {
    if (event) event.stopPropagation();
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      return next;
    });
  };

  const selectAll = () => {
    if (selectedIds.size === displayedResults.length) {
      setSelectedIds(new Set());
    } else {
      setSelectedIds(new Set(displayedResults.map((r) => r.id)));
    }
  };

  const handleBatchAdd = () => {
    const selectedItems = displayedResults.filter((r) => selectedIds.has(r.id));
    if (selectedItems.length === 0) return;
    onBatchAddJobs(selectedItems);
    setSelectedIds(new Set());
  };

  const renderSortIcon = (key: ClientSortKey) => {
    if (clientSortKey !== key) return null;
    return clientSortDir === "asc" ? (
      <ArrowUp size={12} className="sort-icon-active" />
    ) : (
      <ArrowDown size={12} className="sort-icon-active" />
    );
  };

  const getAriaSort = (key: ClientSortKey) => {
    if (clientSortKey !== key) return "none" as const;
    return clientSortDir === "asc" ? ("ascending" as const) : ("descending" as const);
  };

  return (
    <section className="standard-page page-frame search-page">
      <header className="page-header">
        <div>
          <h1>搜索</h1>
          <p>搜索哔哩哔哩公开视频，支持按播放量、发布时间排序与分页浏览。</p>
        </div>
      </header>

      {/* 搜索栏 */}
      <div className="search-entry-section">
        <form className="search-entry-form" onSubmit={handleFormSubmit}>
          <div className={`url-field search-url-field ${error ? "has-error" : ""}`}>
            <Search size={21} strokeWidth={1.8} aria-hidden="true" />
            <input
              value={keyword}
              onChange={(e) => {
                setKeyword(e.target.value);
                if (error) setError("");
              }}
              placeholder="搜索视频关键词或粘贴 BV 号（例如：深度学习入门、Python 进阶、黑神话...）"
              autoFocus
            />
            {keyword ? (
              <button
                type="button"
                className="search-clear-inline"
                onClick={() => setKeyword("")}
                aria-label="清空输入"
              >
                ✕
              </button>
            ) : null}
          </div>
          <button
            className="primary-button search-action-button"
            type="submit"
            disabled={isSearching || cooldownRemaining > 0}
          >
            {isSearching ? (
              <LoaderCircle className="spin" size={18} aria-hidden="true" />
            ) : cooldownRemaining > 0 ? (
              <Clock size={18} aria-hidden="true" />
            ) : (
              <Search size={18} aria-hidden="true" />
            )}
            <span>
              {isSearching
                ? "正在搜索"
                : cooldownRemaining > 0
                ? `冷却中 (${cooldownRemaining}s)`
                : "开始搜索"}
            </span>
          </button>
        </form>

        <div className="search-support-row">
          {searchHistory.length > 0 ? (
            <div className="search-history-row">
              <span className="search-history-label">
                <History size={14} aria-hidden="true" />
                <span>最近搜索</span>
              </span>
              <div className="search-history-tags">
                {searchHistory.map((item) => (
                  <span key={item} className="search-history-tag">
                    <button
                      type="button"
                      className="history-tag-text"
                      onClick={() => handleHistoryTagClick(item)}
                      title={`搜索“${item}”`}
                    >
                      {item}
                    </button>
                    <button
                      type="button"
                      className="history-tag-delete"
                      onClick={(e) => {
                        e.stopPropagation();
                        handleDeleteHistoryItem(item);
                      }}
                      aria-label={`删除历史记录 ${item}`}
                      title="删除此记录"
                    >
                      <X size={13} />
                    </button>
                  </span>
                ))}
                <button
                  type="button"
                  className="search-history-clear"
                  onClick={handleClearAllHistory}
                  title="清空搜索历史"
                >
                  <Trash2 size={13} aria-hidden="true" />
                  <span>清空</span>
                </button>
              </div>
            </div>
          ) : (
            <span className="search-support-hint">支持关键词、BV 号或视频链接</span>
          )}
        </div>

        {error ? (
          <div className="entry-message" role="status">
            <span className="error-message">{error}</span>
          </div>
        ) : null}
      </div>

      <div className="search-filter-panel" aria-label="搜索筛选条件">
        <div className="filter-group">
          <span className="filter-group-label">排序方式</span>
          <div className="filter-group-options">
            {orderOptions.map((opt) => (
              <button
                key={opt.id}
                type="button"
                className={`filter-group-btn ${order === opt.id ? "is-active" : ""}`}
                onClick={() => handleOrderChange(opt.id)}
                aria-pressed={order === opt.id}
              >
                {opt.label}
              </button>
            ))}
          </div>
        </div>

        <div className="filter-group">
          <span className="filter-group-label">视频时长</span>
          <div className="filter-group-options">
            {durationOptions.map((dur) => (
              <button
                key={dur.id}
                type="button"
                className={`filter-group-btn ${durationFilter === dur.id ? "is-active" : ""}`}
                onClick={() => handleDurationChange(dur.id)}
                aria-pressed={durationFilter === dur.id}
              >
                {dur.label}
              </button>
            ))}
          </div>
        </div>
      </div>

      <section className="search-results-section" aria-busy={isSearching}>
        <div className="search-results-heading">
          <h2>搜索结果</h2>
          {hasSearched ? (
            <span className="search-count-hint">
              <strong>{totalCount || displayedResults.length}</strong> 条相关视频 · 第 {page} / {totalPages} 页
            </span>
          ) : null}
        </div>

        {displayedResults.length > 0 ? (
          <div className="search-table-container" role="table" aria-label="搜索结果">
            <div className="search-table-header" role="row">
              <div className="col-checkbox" role="columnheader">
                <button
                  type="button"
                  className={`checkbox-btn ${
                    allDisplayedSelected ? "is-checked" : someDisplayedSelected ? "is-mixed" : ""
                  }`}
                  aria-label={allDisplayedSelected ? "取消全选" : "全选"}
                  aria-pressed={allDisplayedSelected ? true : someDisplayedSelected ? "mixed" : false}
                  onClick={selectAll}
                  title="全选或取消全选"
                >
                  <span className="checkbox-indicator" aria-hidden="true">
                    {allDisplayedSelected ? <Check size={13} strokeWidth={3} /> : null}
                    {someDisplayedSelected ? <Minus size={13} strokeWidth={3} /> : null}
                  </span>
                </button>
              </div>
              <div className="col-video" role="columnheader" aria-sort={getAriaSort("title")}>
                <button
                  type="button"
                  className="search-sort-button"
                  onClick={() => handleSortClick("title")}
                  title="按标题排序"
                >
                  <span>视频</span>
                  {renderSortIcon("title")}
                </button>
              </div>
              <div className="col-author" role="columnheader" aria-sort={getAriaSort("author")}>
                <button
                  type="button"
                  className="search-sort-button"
                  onClick={() => handleSortClick("author")}
                  title="按 UP 主排序"
                >
                  <span>UP 主</span>
                  {renderSortIcon("author")}
                </button>
              </div>
              <div className="col-duration" role="columnheader" aria-sort={getAriaSort("duration")}>
                <button
                  type="button"
                  className="search-sort-button"
                  onClick={() => handleSortClick("duration")}
                  title="按时长排序"
                >
                  <span>时长</span>
                  {renderSortIcon("duration")}
                </button>
              </div>
              <div className="col-plays" role="columnheader" aria-sort={getAriaSort("playCount")}>
                <button
                  type="button"
                  className="search-sort-button"
                  onClick={() => handleSortClick("playCount")}
                  title="按播放量排序"
                >
                  <span>播放量</span>
                  {renderSortIcon("playCount")}
                </button>
              </div>
              <div className="col-date" role="columnheader" aria-sort={getAriaSort("pubDate")}>
                <button
                  type="button"
                  className="search-sort-button"
                  onClick={() => handleSortClick("pubDate")}
                  title="按发布时间排序"
                >
                  <span>发布时间</span>
                  {renderSortIcon("pubDate")}
                </button>
              </div>
              <div className="col-actions" role="columnheader">操作</div>
            </div>

            <div className="search-table-body">
              {displayedResults.map((item) => {
                const isSelected = selectedIds.has(item.id);
                return (
                  <div
                    key={item.id}
                    className={`search-row ${isSelected ? "is-selected" : ""}`}
                    onClick={() => toggleSelect(item.id)}
                    role="row"
                    aria-selected={isSelected}
                  >
                    <div
                      className="col-checkbox"
                      onClick={(e) => toggleSelect(item.id, e)}
                      role="cell"
                    >
                      <button
                        type="button"
                        className={`checkbox-btn ${isSelected ? "is-checked" : ""}`}
                        aria-label={isSelected ? "取消选择" : "勾选"}
                        aria-pressed={isSelected}
                      >
                        <span className="checkbox-indicator" aria-hidden="true">
                          {isSelected ? <Check size={13} strokeWidth={3} /> : null}
                        </span>
                      </button>
                    </div>

                    <div className="col-video" role="cell">
                      <div className="search-thumb-wrapper">
                        {item.coverUrl ? (
                          <img
                            src={item.coverUrl}
                            alt=""
                            className="search-thumb-img"
                            loading="lazy"
                            referrerPolicy="no-referrer"
                          />
                        ) : (
                          <div className="search-thumb-placeholder">
                            <Film size={20} />
                          </div>
                        )}
                      </div>
                      <div className="search-title-info">
                        <span className="search-video-title" title={item.title}>
                          {item.title}
                        </span>
                        <a
                          href={item.videoUrl}
                          target="_blank"
                          rel="noreferrer"
                          className="search-video-link"
                          onClick={(e) => e.stopPropagation()}
                          title="在浏览器中打开原视频"
                        >
                          <span>{item.id.startsWith("http") ? "原视频链接" : item.id}</span>
                          <ExternalLink size={12} />
                        </a>
                      </div>
                    </div>

                    <div className="col-author" role="cell">
                      <span className="search-author-name" title={item.author}>
                        <span>{item.author}</span>
                      </span>
                    </div>

                    <div className="col-duration" role="cell">
                      <span className="search-meta-pill">
                        <span>{item.duration}</span>
                      </span>
                    </div>

                    <div className="col-plays" role="cell">
                      {item.playCount ? (
                        <span className="search-meta-pill">
                          <span>{item.playCount}</span>
                        </span>
                      ) : (
                        <span className="text-subtle">--</span>
                      )}
                    </div>

                    <div className="col-date" role="cell">
                      {item.pubDate ? (
                        <span className="search-meta-pill">
                          <span>{item.pubDate}</span>
                        </span>
                      ) : (
                        <span className="text-subtle">--</span>
                      )}
                    </div>

                    <div className="col-actions" role="cell" onClick={(e) => e.stopPropagation()}>
                      {(() => {
                        const existingJob = findMatchingJob(item, jobs);
                        if (!existingJob) {
                          return (
                            <button
                              type="button"
                              className="table-action-button primary"
                              onClick={() => onStartSingleJob(item)}
                              title="立即创建任务并开始转写"
                            >
                              <Plus size={14} />
                              <span>立即转写</span>
                            </button>
                          );
                        }
                        if (existingJob.status === "completed") {
                          return (
                            <button
                              type="button"
                              className="table-action-button success"
                              onClick={() => onOpenJob(existingJob)}
                              title="笔记已生成，点击查看"
                            >
                              <Check size={14} />
                              <span>查看笔记</span>
                            </button>
                          );
                        }
                        if (existingJob.status === "transcribed") {
                          return (
                            <button
                              type="button"
                              className="table-action-button success"
                              onClick={() => onOpenJob(existingJob)}
                              title="转录已完成，点击查看"
                            >
                              <Check size={14} />
                              <span>查看转录</span>
                            </button>
                          );
                        }
                        if (existingJob.status === "processing") {
                          return (
                            <button
                              type="button"
                              className="table-action-button active"
                              onClick={() => onOpenJob(existingJob)}
                              title="当前正在处理中，点击查看进度"
                            >
                              <LoaderCircle size={14} className="spin" />
                              <span>查看进度</span>
                            </button>
                          );
                        }
                        if (existingJob.status === "paused") {
                          return (
                            <button
                              type="button"
                              className="table-action-button warning"
                              onClick={() => onOpenJob(existingJob)}
                              title="任务已暂停，点击查看详情并继续"
                            >
                              <Play size={14} />
                              <span>已暂停 · 继续</span>
                            </button>
                          );
                        }
                        // status === "waiting" or "failed"
                        return (
                          <button
                            type="button"
                            className="table-action-button primary"
                            onClick={() => onStartSingleJob(item)}
                            title="已在待处理队列中，点击开始"
                          >
                            <Play size={14} />
                            <span>待处理 · 开始</span>
                          </button>
                        );
                      })()}
                    </div>
                  </div>
                );
              })}
            </div>

            {/* 分页组件：与 JobTable 完全一致 */}
            {totalPages > 1 ? (
              <div className="job-table-pagination search-pagination-bar">
                <div className="pagination-info">
                  第 {page} / {totalPages} 页
                </div>
                <div className="pagination-controls">
                  <button
                    type="button"
                    className="pagination-nav-button"
                    disabled={page <= 1 || isSearching}
                    onClick={() => handlePageChange(page - 1)}
                    aria-label="上一页"
                  >
                    <ChevronLeft size={15} />
                    <span>上一页</span>
                  </button>
                  <div className="pagination-pages">
                    {Array.from({ length: totalPages }, (_, i) => i + 1).map((p) => {
                      if (
                        totalPages <= 7 ||
                        p === 1 ||
                        p === totalPages ||
                        (p >= page - 1 && p <= page + 1)
                      ) {
                        return (
                          <button
                            type="button"
                            key={p}
                            className={`page-number-button ${p === page ? "is-active" : ""}`}
                            onClick={() => handlePageChange(p)}
                            disabled={isSearching}
                          >
                            {p}
                          </button>
                        );
                      }
                      if (p === 2 && page > 3) {
                        return (
                          <span key="ellipsis-left" className="pagination-ellipsis">
                            …
                          </span>
                        );
                      }
                      if (p === totalPages - 1 && page < totalPages - 2) {
                        return (
                          <span key="ellipsis-right" className="pagination-ellipsis">
                            …
                          </span>
                        );
                      }
                      return null;
                    })}
                  </div>
                  <button
                    type="button"
                    className="pagination-nav-button"
                    disabled={page >= totalPages || isSearching}
                    onClick={() => handlePageChange(page + 1)}
                    aria-label="下一页"
                  >
                    <span>下一页</span>
                    <ChevronRight size={15} />
                  </button>
                </div>
              </div>
            ) : null}
          </div>
        ) : hasSearched && !isSearching ? (
          <div className="search-empty-state" role="status">
            <Search size={30} strokeWidth={1.6} aria-hidden="true" />
            <h3>没有找到相关视频</h3>
            <p>请尝试更换关键词、排序方式或视频时长。</p>
          </div>
        ) : !hasSearched ? (
          <div className="search-empty-state">
            <Search size={30} strokeWidth={1.6} aria-hidden="true" />
            <h3>开始搜索公开视频</h3>
            <p>输入关键词、BV 号或视频链接，搜索结果会显示在这里。</p>
            <span>例如：RAG、Python、产品设计</span>
          </div>
        ) : null}
      </section>

      {/* 底部悬浮批量操作栏 */}
      {selectedIds.size > 0 ? (
        <div className="search-batch-bar">
          <div className="batch-bar-content">
            <div className="batch-bar-info">
              已勾选 <strong className="batch-count">{selectedIds.size}</strong> 个视频
            </div>
            <div className="batch-bar-actions">
              <button
                type="button"
                className="clipboard-button"
                onClick={() => setSelectedIds(new Set())}
              >
                取消选择
              </button>
              <button
                type="button"
                className="primary-button batch-submit-button"
                onClick={handleBatchAdd}
              >
                <Plus size={16} />
                <span>批量加入任务列表 ({selectedIds.size})</span>
              </button>
            </div>
          </div>
        </div>
      ) : null}
    </section>
  );
}

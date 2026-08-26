import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { AppShell } from "./components/AppShell";
import { HomePage } from "./features/home/HomePage";
import { ModelsPage } from "./features/models/ModelsPage";
import { SearchPage } from "./features/search/SearchPage";
import { SettingsPage } from "./features/settings/SettingsPage";
import { QueuePage } from "./features/queue/QueuePage";
import { VideoLibraryPage } from "./features/library/VideoLibraryPage";
import { VideoDetailPage } from "./features/library/VideoDetailPage";
import { loadAsrSettings, loadPlaybackPreferences, savePlaybackPreferences } from "./lib/preferences";
import { normalizeAppError, runtime } from "./lib/runtime";
import type { EnqueueSourceInput, ModelReadiness, PageId, QueueItem, SourcePreview, Video } from "./types";

export default function App() {
  const [activePage, setActivePage] = useState<PageId>("home");
  const [recentVideos, setRecentVideos] = useState<Video[]>([]);
  const [libraryTotal, setLibraryTotal] = useState(0);
  const [libraryRevision, setLibraryRevision] = useState(0);
  const [queueItems, setQueueItems] = useState<QueueItem[]>([]);
  const [selectedVideoId, setSelectedVideoId] = useState<string | null>(null);
  const [selectedVideo, setSelectedVideo] = useState<Video | null>(null);
  const [modelReadiness, setModelReadiness] = useState<ModelReadiness | null>(null);
  const [autoPlayOnTranscriptClick, setAutoPlayOnTranscriptClick] = useState(() => loadPlaybackPreferences().autoPlayOnTranscriptClick);

  const refreshOverview = useCallback(async () => { const page = await runtime.listVideosPage({ page: 1, pageSize: 4 }); setRecentVideos(page.items); setLibraryTotal(page.total); }, []);
  const refreshQueue = useCallback(async () => { setQueueItems(await runtime.listQueueItems()); }, []);
  const refreshContent = useCallback(async () => { await Promise.all([refreshOverview(), refreshQueue()]); }, [refreshOverview, refreshQueue]);
  const refreshModels = useCallback(async () => { try { const [asr, moss, summary, translation] = await Promise.all([runtime.inspectAsrModel(), runtime.inspectMossModel(), runtime.inspectSummaryModel(), runtime.inspectTranslationModel()]); setModelReadiness({ asr: asr.installed || moss.installed, summary: summary.installed, translation: translation.installed }); } catch { setModelReadiness({ asr: false, summary: false, translation: false }); } }, []);
  useEffect(() => { void Promise.all([refreshContent(), refreshModels()]).catch((reason) => window.alert(normalizeAppError(reason).message)); }, [refreshContent, refreshModels]);
  useEffect(() => { if (!runtime.isDesktop()) return undefined; let active = true; let unlistenQueue: (() => void) | undefined; let unlistenLibrary: (() => void) | undefined; void Promise.all([listen("queue-updated", () => { if (active) void refreshQueue(); }), listen("library-updated", () => { if (active) { setLibraryRevision((value) => value + 1); void refreshOverview(); } })]).then(([queueUnlisten, libraryUnlisten]) => { if (!active) { queueUnlisten(); libraryUnlisten(); return; } unlistenQueue = queueUnlisten; unlistenLibrary = libraryUnlisten; }); return () => { active = false; unlistenQueue?.(); unlistenLibrary?.(); }; }, [refreshOverview, refreshQueue]);
  const enqueue = useCallback(async (sources: EnqueueSourceInput[]) => { const settings = loadAsrSettings(); await runtime.enqueueSources(sources.map((source) => ({ ...source, asrBackend: settings.backend, asrConfigJson: JSON.stringify(settings.moss) }))); await refreshContent(); setActivePage("queue"); }, [refreshContent]);
  const enqueueOne = useCallback((source: SourcePreview) => enqueue([source]), [enqueue]);
  const requeueVideo = useCallback(async (videoId: string) => { const settings = loadAsrSettings(); await runtime.requeueVideo(videoId, settings.backend, JSON.stringify(settings.moss)); await refreshContent(); }, [refreshContent]);
  const openVideo = useCallback((video: Video) => { setSelectedVideo(video); setSelectedVideoId(video.id); setActivePage("video-detail"); }, []);
  const refreshSelectedVideo = useCallback(async () => { if (!selectedVideoId) return; const latest = await runtime.getVideo(selectedVideoId); setSelectedVideo(latest); await refreshOverview(); }, [refreshOverview, selectedVideoId]);
  useEffect(() => { if (!selectedVideoId) setSelectedVideo(null); }, [selectedVideoId]);
  const updateAutoPlay = useCallback((enabled: boolean) => { setAutoPlayOnTranscriptClick(enabled); savePlaybackPreferences({ autoPlayOnTranscriptClick: enabled }); }, []);
  return <AppShell activePage={activePage} modelReadiness={modelReadiness} onNavigate={setActivePage}>{activePage === "home" ? <HomePage queueItems={queueItems} videos={recentVideos} videoCount={libraryTotal} onEnqueue={enqueueOne} onOpenQueue={() => setActivePage("queue")} onOpenLibrary={() => setActivePage("library")} onOpenVideo={openVideo} /> : null}{activePage === "search" ? <SearchPage queueItems={queueItems} libraryRevision={libraryRevision} onEnqueue={enqueue} onOpenVideo={openVideo} /> : null}{activePage === "queue" ? <QueuePage items={queueItems} onRefresh={refreshContent} /> : null}{activePage === "library" ? <VideoLibraryPage revision={libraryRevision} onOpen={openVideo} onRefresh={refreshOverview} onRequeue={requeueVideo} /> : null}{activePage === "models" ? <ModelsPage onStatusChange={setModelReadiness} /> : null}{activePage === "settings" ? <SettingsPage autoPlayOnTranscriptClick={autoPlayOnTranscriptClick} onAutoPlayOnTranscriptClickChange={updateAutoPlay} /> : null}{activePage === "video-detail" && selectedVideo ? <VideoDetailPage video={selectedVideo} onBack={() => setActivePage("library")} onRefresh={refreshSelectedVideo} autoPlayOnTranscriptClick={autoPlayOnTranscriptClick} /> : null}</AppShell>;
}

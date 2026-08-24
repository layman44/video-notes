import { useCallback, useEffect, useMemo, useState } from "react";
import type { JobActionHandlers } from "./components/JobTable";
import { AppShell } from "./components/AppShell";
import { HomePage } from "./features/home/HomePage";
import { ModelsPage } from "./features/models/ModelsPage";
import { SettingsPage } from "./features/settings/SettingsPage";
import { TaskDetailPage } from "./features/task/TaskDetailPage";
import { TasksPage } from "./features/task/TasksPage";
import { loadAsrSettings, loadPlaybackPreferences, savePlaybackPreferences } from "./lib/preferences";
import { runtime } from "./lib/runtime";
import type { Job, JobPhase, ModelReadiness, PageId, SourcePreview } from "./types";

function safeFilename(title: string) {
  return title.replace(/[<>:"/\\|?*\u0000-\u001F]/g, "_").trim() || "video-notes";
}

export default function App() {
  const [activePage, setActivePage] = useState<PageId>("home");
  const [jobs, setJobs] = useState<Job[]>([]);
  const [selectedJobId, setSelectedJobId] = useState<string | null>(null);
  const [modelReadiness, setModelReadiness] = useState<ModelReadiness | null>(null);
  const [autoPlayOnTranscriptClick, setAutoPlayOnTranscriptClick] = useState(
    () => loadPlaybackPreferences().autoPlayOnTranscriptClick,
  );
  const [noteRevisions, setNoteRevisions] = useState<Record<string, number>>({});

  useEffect(() => {
    let active = true;
    void Promise.all([
      runtime.listJobs(),
      runtime.inspectAsrModel(),
      runtime.inspectMossModel(),
      runtime.inspectSummaryModel(),
      runtime.inspectTranslationModel(),
    ])
      .then(async ([loadedJobs, asr, moss, summary, translation]) => {
        let jobs = loadedJobs;
        try {
          const reconciled = await runtime.reconcileJobs();
          if (!active) return;
          if (reconciled.length > 0) {
            jobs = jobs.map(
              (job): Job => {
                const reconciledItem = reconciled.find((item) => item.id === job.id);
                return reconciledItem ? { ...job, ...reconciledItem } : job;
              },
            );
          }
        } catch {
          // 对账失败不阻塞启动，保持原状态
        }
        if (!active) return;
        setJobs(jobs);
        setModelReadiness({ asr: asr.installed || moss.installed, summary: summary.installed, translation: translation.installed });
      })
      .catch(() => {
        if (active) setModelReadiness({ asr: false, summary: false, translation: false });
      });
    return () => {
      active = false;
    };
  }, []);

  const openJob = (job: Job) => {
    setSelectedJobId(job.id);
    setActivePage("task-detail");
  };

  const applyJobPatch = useCallback((jobId: string, patch: Partial<Job>, save = true) => {
    setJobs((current) => current.map((item) => {
      if (item.id !== jobId) return item;
      const next = { ...item, ...patch };
      if (save) void runtime.saveJob(next);
      return next;
    }));
  }, []);

  const phasePercent = useCallback((completed?: number, total?: number) => {
    if (completed == null || total == null || total <= 0) return 0;
    return Math.min(100, Math.max(0, Math.round((completed / total) * 100)));
  }, []);

  const runTranscription = useCallback((job: Job) => {
    void (async () => {
      try {
        await runtime.transcribeMedia(
          job.id,
          (phaseEvent) => {
            if (phaseEvent.state === "started") {
              applyJobPatch(job.id, {
                status: "processing",
                phase: phaseEvent.phase as JobPhase,
                phaseCompleted: undefined,
                phaseTotal: undefined,
                phaseUnit: undefined,
                progress: 0,
                updatedAt: "刚刚",
                statusMessage: phaseEvent.message,
                errorMessage: undefined,
              });
            } else {
              applyJobPatch(job.id, {
                status: "processing",
                phase: phaseEvent.phase as JobPhase,
                updatedAt: "刚刚",
                statusMessage: phaseEvent.message,
              });
            }
          },
          (phaseProgress) => {
            applyJobPatch(job.id, {
              status: "processing",
              phase: phaseProgress.phase as JobPhase,
              phaseCompleted: phaseProgress.completed,
              phaseTotal: phaseProgress.total,
              phaseUnit: phaseProgress.unit,
              progress: phasePercent(phaseProgress.completed, phaseProgress.total),
              updatedAt: "刚刚",
              statusMessage: phaseProgress.message,
              errorMessage: undefined,
            });
          },
        );
        applyJobPatch(job.id, {
          status: "transcribed",
          progress: 100,
          phase: undefined,
          phaseCompleted: undefined,
          phaseTotal: undefined,
          phaseUnit: undefined,
          updatedAt: "刚刚",
          statusMessage: "转录完成。可先人工检查校正结果，再按需翻译或生成笔记。",
          errorMessage: undefined,
        });
      } catch (reason) {
        const message = reason instanceof Error ? reason.message : String(reason);
        if (message.includes("MODEL_NOT_INSTALLED")) {
          applyJobPatch(job.id, {
            status: "waiting",
            progress: 0,
            phase: undefined,
            phaseCompleted: undefined,
            phaseTotal: undefined,
            phaseUnit: undefined,
            updatedAt: "刚刚",
            statusMessage: message.replace(/^.*MODEL_NOT_INSTALLED:/, ""),
            errorMessage: undefined,
          });
          setModelReadiness((current) => ({ asr: false, summary: current?.summary ?? false, translation: current?.translation ?? false }));
          return;
        }
        applyJobPatch(job.id, {
          status: message.includes("取消") ? "paused" : "failed",
          updatedAt: "刚刚",
          errorMessage: message,
        });
      }
    })();
  }, [applyJobPatch, phasePercent]);

  const runPipeline = useCallback((job: Job, skipMedia = false) => {
    void (async () => {
      try {
        if (!skipMedia) {
          await runtime.prepareMedia(job.id, job.sourceUrl, (mediaProgress) => {
            const phase: JobPhase = mediaProgress.stage === "download" ? "media_download" : "media_normalize";
            applyJobPatch(job.id, {
              status: "processing",
              phase,
              phaseCompleted: mediaProgress.progress,
              phaseTotal: 100,
              phaseUnit: "percent",
              progress: mediaProgress.progress,
              updatedAt: "刚刚",
              statusMessage: mediaProgress.message,
              errorMessage: undefined,
            });
          });
        }
        applyJobPatch(job.id, {
          status: "waiting",
          progress: 0,
          phase: undefined,
          phaseCompleted: undefined,
          phaseTotal: undefined,
          phaseUnit: undefined,
          updatedAt: "刚刚",
          statusMessage: "音频已准备完成，正在启动语音识别模型……",
          errorMessage: undefined,
        });
        runTranscription(job);
      } catch (reason) {
        const message = reason instanceof Error ? reason.message : String(reason);
        applyJobPatch(job.id, {
          status: message.includes("取消") ? "paused" : "failed",
          updatedAt: "刚刚",
          errorMessage: message,
        });
      }
    })();
  }, [applyJobPatch, runTranscription]);

  const startJob = useCallback((preview: SourcePreview) => {
    const job: Job = {
      id: `job-${Date.now()}`,
      title: preview.title,
      platform: preview.platform,
      duration: preview.duration,
      updatedAt: "刚刚",
      status: "processing",
      progress: 0,
      phase: "media_download",
      phaseCompleted: 0,
      phaseTotal: 100,
      phaseUnit: "percent",
      sourceUrl: preview.sourceUrl,
      thumbnailUrl: preview.thumbnailUrl,
      asrBackend: loadAsrSettings().backend,
      asrConfigJson: JSON.stringify(loadAsrSettings().moss),
    };
    setJobs((current) => [job, ...current]);
    setSelectedJobId(job.id);
    setActivePage("task-detail");
    if (runtime.isDesktop()) {
      // Persist the backend snapshot before the native command reads it.
      void runtime.saveJob(job).then(() => runPipeline(job));
    } else {
      void runtime.saveJob(job);
    }
  }, [runPipeline]);

  const retryPipeline = useCallback((job: Job) => {
    const downstreamPhase = [
      "recognition", "pause_alignment", "verification", "boundary_review", "word_alignment", "standardization", "semantic_segmentation", "translation", "summary",
    ].includes(job.phase ?? "");
    const mediaAlreadyPrepared = job.status === "waiting"
      || job.status === "transcribed"
      || job.status === "completed"
      || downstreamPhase;
    const asrSettings = loadAsrSettings();
    const retryingJob: Job = {
      ...job,
      status: "processing",
      progress: 0,
      phase: mediaAlreadyPrepared ? "recognition" : "media_download",
      phaseCompleted: undefined,
      phaseTotal: undefined,
      phaseUnit: undefined,
      updatedAt: "刚刚",
      errorMessage: undefined,
      statusMessage: mediaAlreadyPrepared ? "正在重新启动语音转录……" : "正在重新准备本地视频与音频……",
      asrBackend: asrSettings.backend,
      asrConfigJson: JSON.stringify(asrSettings.moss),
    };
    setJobs((current) => current.map((item) => (item.id === job.id ? retryingJob : item)));
    // A retry may skip media preparation, so wait for the DB snapshot before
    // starting native transcription; otherwise it could pick the old backend.
    void runtime.saveJob(retryingJob).then(() => runPipeline(retryingJob, mediaAlreadyPrepared));
  }, [runPipeline]);

  const translateJob = useCallback((job: Job) => {
    void (async () => {
      const returnStatus = job.status === "completed" ? "completed" : "transcribed";
      try {
        const translationStatus = await runtime.inspectTranslationModel();
        if (!translationStatus.installed) {
          setModelReadiness((current) => ({ asr: current?.asr ?? true, summary: current?.summary ?? false, translation: false }));
          applyJobPatch(job.id, {
            status: returnStatus,
            statusMessage: "请先到“模型”页面下载 MiLMMT 翻译模型，再手动开始翻译。",
          });
          return;
        }
        applyJobPatch(job.id, {
          status: "processing",
          phase: "translation",
          progress: 0,
          phaseCompleted: 0,
          phaseTotal: undefined,
          phaseUnit: "segments",
          updatedAt: "刚刚",
          statusMessage: "正在翻译标准转录……",
          errorMessage: undefined,
        });
        await runtime.translateTranscript(job.id, (translationProgress) => {
          applyJobPatch(job.id, {
            status: "processing",
            phase: "translation",
            phaseCompleted: translationProgress.completed,
            phaseTotal: translationProgress.total,
            phaseUnit: "segments",
            progress: phasePercent(translationProgress.completed, translationProgress.total),
            statusMessage: translationProgress.message,
            updatedAt: "刚刚",
          });
        });
        applyJobPatch(job.id, {
          status: returnStatus,
          progress: 100,
          phase: undefined,
          phaseCompleted: undefined,
          phaseTotal: undefined,
          phaseUnit: undefined,
          statusMessage: returnStatus === "completed"
            ? "翻译完成。现有笔记不会自动重写；如需让笔记使用新翻译，可点击“重新生成笔记”。"
            : "翻译完成。标准转录仍可继续人工修改；修改过的段落会自动使对应翻译失效。",
          updatedAt: "刚刚",
        });
        setNoteRevisions((current) => ({ ...current, [job.id]: (current[job.id] ?? 0) + 1 }));
      } catch (reason) {
        const message = reason instanceof Error ? reason.message : String(reason);
        applyJobPatch(job.id, {
          status: returnStatus,
          phase: undefined,
          phaseCompleted: undefined,
          phaseTotal: undefined,
          phaseUnit: undefined,
          statusMessage: "翻译未完成，标准转录已保留。",
          errorMessage: message,
        });
      }
    })();
  }, [applyJobPatch, phasePercent]);

  const organizeJob = useCallback((job: Job, force = false) => {
    void (async () => {
      try {
        const summaryStatus = await runtime.inspectSummaryModel();
        if (!summaryStatus.installed) {
          setModelReadiness((current) => ({ asr: current?.asr ?? true, summary: false, translation: current?.translation ?? false }));
          applyJobPatch(job.id, {
            status: "transcribed",
            statusMessage: "请先到“模型”页面下载 Qwen3.5 总结模型，再手动生成笔记。",
          });
          return;
        }
        applyJobPatch(job.id, {
          status: "processing",
          phase: "summary",
          progress: 0,
          phaseCompleted: 0,
          phaseTotal: undefined,
          phaseUnit: "parts",
          updatedAt: "刚刚",
          statusMessage: "正在根据当前标准转录生成笔记……",
          errorMessage: undefined,
        });
        await runtime.organizeNotes(job, (summaryProgress) => {
          applyJobPatch(job.id, {
            status: "processing",
            phase: "summary",
            phaseCompleted: summaryProgress.partIndex,
            phaseTotal: summaryProgress.partCount || undefined,
            phaseUnit: "parts",
            progress: summaryProgress.progress,
            updatedAt: "刚刚",
            statusMessage: summaryProgress.message,
          });
        }, force);
        applyJobPatch(job.id, {
          status: "completed",
          progress: 100,
          phase: undefined,
          phaseCompleted: undefined,
          phaseTotal: undefined,
          phaseUnit: undefined,
          updatedAt: "刚刚",
          statusMessage: "笔记已生成",
          errorMessage: undefined,
        });
        setNoteRevisions((current) => ({ ...current, [job.id]: (current[job.id] ?? 0) + 1 }));
      } catch (reason) {
        const message = reason instanceof Error ? reason.message : String(reason);
        applyJobPatch(job.id, {
          status: "transcribed",
          phase: undefined,
          phaseCompleted: undefined,
          phaseTotal: undefined,
          phaseUnit: undefined,
          statusMessage: "笔记生成失败；标准转录已保留，可稍后重试。",
          errorMessage: message,
        });
      }
    })();
  }, [applyJobPatch]);

  const openTaskDirectory = useCallback(async (job: Job) => {
    await runtime.openTaskDirectory(job.id);
  }, []);

  const redownloadJob = useCallback(async (job: Job) => {
    await runtime.resetTaskMedia(job.id);
    const asrSettings = loadAsrSettings();
    const restartingJob: Job = {
      ...job,
      status: "processing",
      progress: 0,
      phase: "media_download",
      phaseCompleted: 0,
      phaseTotal: 100,
      phaseUnit: "percent",
      updatedAt: "刚刚",
      errorMessage: undefined,
      statusMessage: "正在重新下载本地视频……",
      asrBackend: asrSettings.backend,
      asrConfigJson: JSON.stringify(asrSettings.moss),
    };
    setJobs((current) => current.map((item) => item.id === job.id ? restartingJob : item));
    await runtime.saveJob(restartingJob);
    runPipeline(restartingJob);
  }, [runPipeline]);

  const retranscribeJob = useCallback(async (job: Job) => {
    await runtime.resetTaskTranscript(job.id);
    const asrSettings = loadAsrSettings();
    const retranscribingJob: Job = {
      ...job,
      status: "processing",
      progress: 0,
      phase: "recognition",
      phaseCompleted: undefined,
      phaseTotal: undefined,
      phaseUnit: undefined,
      updatedAt: "刚刚",
      errorMessage: undefined,
      statusMessage: "正在重新进行本地语音转写……",
      asrBackend: asrSettings.backend,
      asrConfigJson: JSON.stringify(asrSettings.moss),
    };
    setJobs((current) => current.map((item) => item.id === job.id ? retranscribingJob : item));
    await runtime.saveJob(retranscribingJob);
    runPipeline(retranscribingJob, true);
  }, [runPipeline]);

  const exportJobAudio = useCallback(async (job: Job) => {
    const path = await runtime.exportTaskAudio(job.id, `${safeFilename(job.title)}.m4a`);
    if (path) window.alert("音频已导出完成");
  }, []);

  const deleteJob = useCallback(async (job: Job) => {
    if (!window.confirm(`确定删除“${job.title}”吗？\n\n本地视频、音频切片、转录和笔记都会被删除，此操作无法撤销。`)) return;
    await runtime.deleteTask(job.id);
    setJobs((current) => current.filter((item) => item.id !== job.id));
    if (selectedJobId === job.id) {
      setSelectedJobId(null);
      setActivePage("tasks");
    }
  }, [selectedJobId]);

  const jobActions = useMemo<JobActionHandlers>(() => ({
    onOpenDirectory: openTaskDirectory,
    onRedownload: redownloadJob,
    onRetranscribe: retranscribeJob,
    onExportAudio: exportJobAudio,
    onDelete: deleteJob,
  }), [deleteJob, exportJobAudio, openTaskDirectory, redownloadJob, retranscribeJob]);

  const completeJob = useCallback((jobId: string) => {
    setJobs((current) => current.map((job) => (
      job.id === jobId ? { ...job, status: "completed", progress: 100, updatedAt: "刚刚" } : job
    )));
  }, []);

  const updateAutoPlayOnTranscriptClick = useCallback((enabled: boolean) => {
    setAutoPlayOnTranscriptClick(enabled);
    savePlaybackPreferences({ autoPlayOnTranscriptClick: enabled });
  }, []);

  const selectedJob = jobs.find((job) => job.id === selectedJobId) ?? jobs[0];

  return (
    <AppShell activePage={activePage} modelReadiness={modelReadiness} onNavigate={setActivePage}>
      {activePage === "home" ? <HomePage jobs={jobs} onOpenJob={openJob} onStart={startJob} jobActions={jobActions} /> : null}
      {activePage === "tasks" ? <TasksPage jobs={jobs} onOpenJob={openJob} jobActions={jobActions} /> : null}
      {activePage === "models" ? <ModelsPage onStatusChange={setModelReadiness} /> : null}
      {activePage === "settings" ? (
        <SettingsPage
          autoPlayOnTranscriptClick={autoPlayOnTranscriptClick}
          onAutoPlayOnTranscriptClickChange={updateAutoPlayOnTranscriptClick}
        />
      ) : null}
      {activePage === "task-detail" && selectedJob ? (
        <TaskDetailPage
          job={selectedJob}
          onBack={() => setActivePage("tasks")}
          onNavigateToModels={() => setActivePage("models")}
          onComplete={completeJob}
          onCancelMedia={() => runtime.cancelMedia(selectedJob.id)}
          onRetryMedia={() => retryPipeline(selectedJob)}
          onTranslate={() => translateJob(selectedJob)}
          onOrganize={() => organizeJob(selectedJob)}
          onReorganize={() => organizeJob(selectedJob, true)}
          onTranscriptEdited={() => {
            applyJobPatch(selectedJob.id, {
              status: "transcribed",
              progress: 100,
              phase: undefined,
              phaseCompleted: undefined,
              phaseTotal: undefined,
              phaseUnit: undefined,
              statusMessage: "标准转录已人工修改；对应旧翻译与旧笔记已失效。",
              errorMessage: undefined,
              updatedAt: "刚刚",
            });
            setNoteRevisions((current) => ({ ...current, [selectedJob.id]: (current[selectedJob.id] ?? 0) + 1 }));
          }}
          autoPlayOnTranscriptClick={autoPlayOnTranscriptClick}
          usesRealMediaPipeline={runtime.isDesktop() && selectedJob.id.startsWith("job-")}
          noteRevision={noteRevisions[selectedJob.id] ?? 0}
        />
      ) : null}
    </AppShell>
  );
}

import { Check, CircleAlert, FolderOpen, LoaderCircle, RotateCcw } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { runtime } from "../../lib/runtime";
import { loadAsrSettings, loadDownloadPreferences, saveAsrSettings, saveDownloadPreferences } from "../../lib/preferences";
import { toast } from "../../lib/toast";
import type { AsrBackend, DataDirectorySettings, MediaToolsStatus, VideoDownloadQuality } from "../../types";

function Toggle({ checked, onChange, label }: { checked: boolean; onChange: () => void; label: string }) {
  return (
    <button className={`toggle ${checked ? "is-on" : ""}`} type="button" role="switch" aria-checked={checked} aria-label={label} onClick={onChange}>
      <span />
    </button>
  );
}

interface SettingsPageProps {
  autoPlayOnTranscriptClick: boolean;
  onAutoPlayOnTranscriptClickChange: (enabled: boolean) => void;
}

let memoryCachedMediaTools: MediaToolsStatus | null = null;
let memoryCachedDataDirectory: DataDirectorySettings | null = null;

const toolDescriptions: Record<string, string> = {
  "yt-dlp": "用于公开视频链接解析与音视频资源下载",
  "FFmpeg": "用于音视频格式转码、16kHz 音频提取及时长分析",
};

export function SettingsPage({
  autoPlayOnTranscriptClick,
  onAutoPlayOnTranscriptClickChange,
}: SettingsPageProps) {
  const [lowPriority, setLowPriority] = useState(true);
  const [mediaTools, setMediaTools] = useState<MediaToolsStatus | null>(memoryCachedMediaTools);
  const [dataDirectory, setDataDirectory] = useState<DataDirectorySettings | null>(memoryCachedDataDirectory);
  const [directoryBusy, setDirectoryBusy] = useState(false);
  const [directoryFeedback, setDirectoryFeedback] = useState<{ kind: "success" | "error"; message: string } | null>(null);
  const [showResetConfirm, setShowResetConfirm] = useState(false);
  const [asrSettings, setAsrSettings] = useState(loadAsrSettings);
  const [downloadPrefs, setDownloadPrefs] = useState(loadDownloadPreferences);

  const updateAsrBackend = (backend: AsrBackend) => {
    const next = { ...asrSettings, backend };
    setAsrSettings(next);
    saveAsrSettings(next);
    toast.info(`默认转录引擎已切换为：${backend === "openasr-moss-q4" ? "MOSS q4 高精度模式" : "Fun-ASR-Nano 快速模式"}`);
  };

  const updateMaxConcurrentDownloads = async (count: number) => {
    const next = { ...downloadPrefs, maxConcurrentDownloads: count };
    setDownloadPrefs(next);
    saveDownloadPreferences(next);
    try {
      await runtime.setMaxConcurrentDownloads(count);
      toast.info(`并发下载数已设置为：${count} 个视频`);
    } catch {
      // ignore
    }
  };

  const updateVideoQuality = async (quality: VideoDownloadQuality) => {
    const next = { ...downloadPrefs, videoQuality: quality };
    setDownloadPrefs(next);
    saveDownloadPreferences(next);
    try {
      await runtime.setVideoDownloadQuality(quality);
      const label = quality === "1080p" ? "1080P 高清" : quality === "best" ? "原画最高" : "720P 高速省空间";
      toast.info(`视频下载清晰度已设置为：${label}`);
    } catch {
      // ignore
    }
  };

  useEffect(() => {
    let active = true;
    void Promise.all([runtime.inspectMediaTools(), runtime.inspectDataDirectory()])
      .then(([tools, directory]) => {
        if (!active) return;
        memoryCachedMediaTools = tools;
        memoryCachedDataDirectory = directory;
        setMediaTools(tools);
        setDataDirectory(directory);
      })
      .catch((reason) => {
        if (active) setDirectoryFeedback({ kind: "error", message: reason instanceof Error ? reason.message : String(reason) });
      });
    return () => {
      active = false;
    };
  }, []);

  const chooseDirectory = useCallback(async () => {
    if (directoryBusy) return;
    setDirectoryBusy(true);
    setDirectoryFeedback(null);
    try {
      const selected = await runtime.chooseDataDirectory();
      if (selected) {
        memoryCachedDataDirectory = selected;
        setDataDirectory(selected);
        const msg = "数据目录已更新，已有视频数据已复制到新位置。";
        setDirectoryFeedback({ kind: "success", message: msg });
        toast.success(msg);
      }
    } catch (reason) {
      const msg = reason instanceof Error ? reason.message : String(reason);
      setDirectoryFeedback({ kind: "error", message: msg });
      toast.error(`更换目录失败: ${msg}`);
    } finally {
      setDirectoryBusy(false);
    }
  }, [directoryBusy]);

  const resetDirectory = useCallback(async () => {
    if (directoryBusy || dataDirectory?.isDefault) return;
    setDirectoryBusy(true);
    setDirectoryFeedback(null);
    try {
      const reset = await runtime.resetDataDirectory();
      memoryCachedDataDirectory = reset;
      setDataDirectory(reset);
      const msg = "已恢复默认数据目录，已有视频数据已复制回默认位置。";
      setDirectoryFeedback({ kind: "success", message: msg });
      toast.success(msg);
    } catch (reason) {
      const msg = reason instanceof Error ? reason.message : String(reason);
      setDirectoryFeedback({ kind: "error", message: msg });
      toast.error(`恢复默认目录失败: ${msg}`);
    } finally {
      setDirectoryBusy(false);
    }
  }, [dataDirectory?.isDefault, directoryBusy]);

  return (
    <section className="standard-page page-frame settings-page">
      <header className="page-header">
        <div>
          <h1>设置</h1>
          <p>控制本地资源、文件保留与隐私行为。</p>
        </div>
      </header>

      <div className="settings-body-scroll">
        <section className="settings-group media-tools-group">
          <h2>媒体组件</h2>
          <p className="settings-description">用于解析公开视频并生成适合本地语音识别的音频切片。</p>
          <div className="tool-status-list">
            {mediaTools ? [mediaTools.ytDlp, mediaTools.ffmpeg].map((tool) => (
              <div className="setting-row tool-status-row" key={tool.name}>
                <div>
                  <strong>{tool.name}</strong>
                  <span title={tool.path}>{toolDescriptions[tool.name] ?? (tool.available ? "组件正常运行" : "未检测到该组件，请确保完整安装包解压正常")}</span>
                </div>
                <span className={`tool-health ${tool.available ? "is-ready" : "is-missing"}`}>
                  {tool.available ? <Check size={14} aria-hidden="true" /> : <CircleAlert size={14} aria-hidden="true" />}
                  {tool.available ? "已就绪" : "待配置"}
                </span>
              </div>
            )) : (
              <div className="tool-loading"><LoaderCircle className="spin" size={17} />正在检查本地组件……</div>
            )}
          </div>
        </section>

        <section className="settings-group">
          <h2>播放与转录</h2>
          <div className="setting-row">
            <div>
              <strong>默认识别模型</strong>
              <span>{asrSettings.backend === "openasr-moss-q4" ? "MOSS q4 高精度，速度较慢" : "Fun-ASR-Nano 快速识别"}</span>
            </div>
            <select value={asrSettings.backend} onChange={(event) => updateAsrBackend(event.target.value as AsrBackend)} aria-label="默认识别模型">
              <option value="funasr-nano">FunASR 快速</option>
              <option value="openasr-moss-q4">MOSS q4 高精度</option>
            </select>
          </div>
          <div className="setting-row">
            <div>
              <strong>视频下载清晰度</strong>
              <span>控制下载本地视频的画质上限（720P 下载更快更省空间，1080P 画质更细腻）</span>
            </div>
            <select
              value={downloadPrefs.videoQuality ?? "720p"}
              onChange={(event) => void updateVideoQuality(event.target.value as VideoDownloadQuality)}
              aria-label="视频下载清晰度"
            >
              <option value="720p">720P (推荐，高速省空间)</option>
              <option value="1080p">1080P 高清 (画质更细腻)</option>
              <option value="best">原画最高 (平台最高可用)</option>
            </select>
          </div>
          <div className="setting-row">
            <div>
              <strong>同时下载视频数</strong>
              <span>后台预下载后序视频（建议 2 个，兼顾下载速度与防平台限速）</span>
            </div>
            <select
              value={downloadPrefs.maxConcurrentDownloads}
              onChange={(event) => void updateMaxConcurrentDownloads(Number(event.target.value))}
              aria-label="同时下载视频数"
            >
              <option value={1}>1 个 (保守模式)</option>
              <option value={2}>2 个 (推荐，兼顾速度与防封)</option>
              <option value={3}>3 个 (高速宽带)</option>
            </select>
          </div>
          <div className="setting-row">
            <div>
              <strong>点击转录后自动播放</strong>
              <span>跳转到对应时间后立即开始播放本地视频</span>
            </div>
            <Toggle
              checked={autoPlayOnTranscriptClick}
              onChange={() => onAutoPlayOnTranscriptClickChange(!autoPlayOnTranscriptClick)}
              label="点击转录后自动播放"
            />
          </div>
        </section>

        <section className="settings-group">
          <h2>性能</h2>
          <div className="setting-row">
            <div><strong>处理模式</strong><span>为 16GB 无独显电脑保留两个逻辑核心</span></div>
            <select defaultValue="balanced" aria-label="处理模式">
              <option value="eco">节能</option>
              <option value="balanced">均衡</option>
              <option value="quality">高质量</option>
            </select>
          </div>
          <div className="setting-row">
            <div><strong>后台低优先级运行</strong><span>处理期间减少对其他应用的影响</span></div>
            <Toggle checked={lowPriority} onChange={() => setLowPriority((value) => !value)} label="后台低优先级运行" />
          </div>
        </section>

        <section className="settings-group">
          <h2>文件与隐私</h2>
          <div className="setting-row data-directory-row">
            <div>
              <strong>视频与解析数据目录</strong>
              <span>视频、音频、转录和 Markdown 笔记均保存在此目录中</span>
            </div>
            <div className="data-directory-controls">
              <code title={dataDirectory?.currentPath}>{dataDirectory?.currentPath ?? "正在读取目录……"}</code>
              <div>
                <button className="secondary-button compact-button" type="button" disabled={directoryBusy} onClick={() => void chooseDirectory()}>
                  {directoryBusy ? <LoaderCircle className="spin" size={15} /> : <FolderOpen size={15} />}
                  选择目录
                </button>
                <button
                  className="secondary-button compact-button"
                  type="button"
                  disabled={directoryBusy || !dataDirectory || dataDirectory.isDefault}
                  onClick={() => setShowResetConfirm(true)}
                >
                  <RotateCcw size={15} />
                  恢复默认
                </button>
              </div>
            </div>
          </div>
          {directoryFeedback ? (
            <p className={`directory-feedback is-${directoryFeedback.kind}`} role="status">
              {directoryFeedback.message}
            </p>
          ) : null}
          <p className="data-directory-note">更改目录时会复制现有视频数据，原目录暂不删除；模型文件与视频数据库仍保存在应用内部目录。</p>
        </section>
      </div>

      {showResetConfirm ? (
        <ConfirmDialog
          title="恢复默认数据目录"
          message="确定恢复为系统默认存储目录吗？已有视频数据与解析结果将同步复制回默认路径。"
          confirmText="确定恢复"
          onConfirm={() => {
            setShowResetConfirm(false);
            void resetDirectory();
          }}
          onCancel={() => setShowResetConfirm(false)}
        />
      ) : null}
    </section>
  );
}

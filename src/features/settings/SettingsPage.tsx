import { Check, CircleAlert, FolderOpen, LoaderCircle, RotateCcw } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { runtime } from "../../lib/runtime";
import type { DataDirectorySettings, MediaToolsStatus } from "../../types";

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

export function SettingsPage({
  autoPlayOnTranscriptClick,
  onAutoPlayOnTranscriptClickChange,
}: SettingsPageProps) {
  const [autoClean, setAutoClean] = useState(true);
  const [lowPriority, setLowPriority] = useState(true);
  const [diagnostics, setDiagnostics] = useState(false);
  const [mediaTools, setMediaTools] = useState<MediaToolsStatus | null>(null);
  const [dataDirectory, setDataDirectory] = useState<DataDirectorySettings | null>(null);
  const [directoryBusy, setDirectoryBusy] = useState(false);
  const [directoryFeedback, setDirectoryFeedback] = useState<{ kind: "success" | "error"; message: string } | null>(null);

  useEffect(() => {
    let active = true;
    void Promise.all([runtime.inspectMediaTools(), runtime.inspectDataDirectory()])
      .then(([tools, directory]) => {
        if (!active) return;
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
        setDataDirectory(selected);
        setDirectoryFeedback({ kind: "success", message: "数据目录已更新，已有任务数据已复制到新位置。" });
      }
    } catch (reason) {
      setDirectoryFeedback({ kind: "error", message: reason instanceof Error ? reason.message : String(reason) });
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
      setDataDirectory(reset);
      setDirectoryFeedback({ kind: "success", message: "已恢复默认数据目录，已有任务数据已复制回默认位置。" });
    } catch (reason) {
      setDirectoryFeedback({ kind: "error", message: reason instanceof Error ? reason.message : String(reason) });
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

      <section className="settings-group media-tools-group">
        <h2>媒体组件</h2>
        <p className="settings-description">用于解析公开视频并生成适合本地语音识别的音频切片。</p>
        <div className="tool-status-list">
          {mediaTools ? [mediaTools.ytDlp, mediaTools.ffmpeg, mediaTools.ffprobe].map((tool) => (
            <div className="setting-row tool-status-row" key={tool.name}>
              <div>
                <strong>{tool.name}</strong>
                <span title={tool.path}>{tool.version ?? (tool.available ? "版本信息不可用" : "安装包中未找到该组件")}</span>
              </div>
              <span className={`tool-health ${tool.available ? "is-ready" : "is-missing"}`}>
                {tool.available ? <Check size={14} aria-hidden="true" /> : <CircleAlert size={14} aria-hidden="true" />}
                {tool.available ? "已就绪" : "缺失"}
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
            <span>视频、音频切片、转录和 Markdown 笔记均保存在此目录的 tasks 文件夹中</span>
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
                onClick={() => void resetDirectory()}
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
        <p className="data-directory-note">更改目录时会复制现有任务数据，原目录暂不删除；模型文件与任务数据库仍保存在应用内部目录。</p>
        <div className="setting-row">
          <div><strong>完成后删除临时音频</strong><span>保留转录和 Markdown，不保留原始音频</span></div>
          <Toggle checked={autoClean} onChange={() => setAutoClean((value) => !value)} label="完成后删除临时音频" />
        </div>
        <div className="setting-row">
          <div><strong>发送匿名诊断信息</strong><span>默认关闭；不会包含链接、转录或文档内容</span></div>
          <Toggle checked={diagnostics} onChange={() => setDiagnostics((value) => !value)} label="发送匿名诊断信息" />
        </div>
      </section>
    </section>
  );
}

import {
  Box,
  FileClock,
  Home,
  Settings,
  ShieldCheck,
  type LucideIcon,
} from "lucide-react";
import type { ModelReadiness, PageId } from "../types";

interface AppShellProps {
  activePage: PageId;
  children: React.ReactNode;
  modelReadiness: ModelReadiness | null;
  onNavigate: (page: Exclude<PageId, "task-detail">) => void;
}

const navItems: Array<{
  id: Exclude<PageId, "task-detail">;
  label: string;
  icon: LucideIcon;
}> = [
  { id: "home", label: "首页", icon: Home },
  { id: "tasks", label: "任务", icon: FileClock },
  { id: "models", label: "模型", icon: Box },
  { id: "settings", label: "设置", icon: Settings },
];

export function AppShell({ activePage, children, modelReadiness, onNavigate }: AppShellProps) {
  const normalizedActive = activePage === "task-detail" ? "tasks" : activePage;
  const readyModelCount = modelReadiness
    ? Number(modelReadiness.asr) + Number(modelReadiness.summary) + Number(Boolean(modelReadiness.translation))
    : 0;

  return (
    <div className="app-shell">
      <aside className="sidebar" aria-label="主导航">
        <button className="brand" type="button" onClick={() => onNavigate("home")}>
          <span className="brand-mark" aria-hidden="true">
            <span className="brand-page" />
            <span className="brand-play" />
          </span>
          <span>VideoNotes</span>
        </button>

        <nav className="nav-list">
          {navItems.map(({ id, label, icon: Icon }) => (
            <button
              className={`nav-item ${normalizedActive === id ? "is-active" : ""}`}
              type="button"
              key={id}
              onClick={() => onNavigate(id)}
              aria-current={normalizedActive === id ? "page" : undefined}
            >
              <Icon size={21} strokeWidth={1.8} aria-hidden="true" />
              <span>{label}</span>
            </button>
          ))}
        </nav>

        <div className="sidebar-status">
          <div className="model-ready">
            <span className={`status-dot ${modelReadiness && readyModelCount < 3 ? "is-unavailable" : ""}`} aria-hidden="true" />
            <span>{modelReadiness === null ? "正在检查本地模型" : `本地模型 ${readyModelCount}/3 已就绪`}</span>
          </div>
          <div className="privacy-note">
            <ShieldCheck size={16} strokeWidth={1.8} aria-hidden="true" />
            <span>数据不会上传</span>
          </div>
        </div>
      </aside>

      <main className="main-content">{children}</main>
    </div>
  );
}

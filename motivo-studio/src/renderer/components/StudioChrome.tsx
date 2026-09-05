import type { StudioView } from "../../shared/contracts";
import { formatTime } from "../format";
import type { NavigationView } from "../model";
import { Icon, type IconName } from "./Icon";

interface StudioHeaderProps {
  readonly studio: StudioView | null;
  readonly workspaceBusy: boolean;
  readonly actionBusy: boolean;
  readonly onOpen: () => void;
  readonly onInitialize: () => void;
  readonly onRefresh: () => void;
}

export function StudioHeader({
  studio,
  workspaceBusy,
  actionBusy,
  onOpen,
  onInitialize,
  onRefresh,
}: StudioHeaderProps) {
  const snapshot = studio?.snapshot;
  return (
    <header className="topbar">
      <div className="brand" aria-label="Motivo Studio">
        <span className="brand-mark" aria-hidden="true" />
        <span className="brand-copy">
          <strong>MOTIVO</strong>
          <small>STUDIO 0.3</small>
        </span>
      </div>

      <div className="workspace-crumb">
        <span className={`status-dot ${snapshot?.health.ok ? "ready" : ""}`} aria-hidden="true" />
        <div>
          <strong>{snapshot?.workspace.name ?? "No workspace connected"}</strong>
          <span>
            {snapshot
              ? `${snapshot.health.ok ? "runtime healthy" : "runtime needs attention"} · snapshot ${formatTime(snapshot.generatedAtUnixMs)}`
              : "Open an initialized Tactus folder or initialize a new one"}
          </span>
        </div>
      </div>

      <div className="topbar-actions">
        <button
          type="button"
          className="button ghost"
          disabled={workspaceBusy || actionBusy}
          onClick={onOpen}
        >
          <Icon name="folder" />
          <span>Open initialized workspace</span>
        </button>
        <button
          type="button"
          className="button"
          disabled={workspaceBusy || actionBusy}
          onClick={onInitialize}
        >
          <Icon name="plus" />
          <span>Initialize folder</span>
        </button>
        <button
          type="button"
          className="icon-button"
          aria-label="Refresh workspace"
          title="Refresh workspace"
          disabled={!studio || workspaceBusy || actionBusy}
          onClick={onRefresh}
        >
          <Icon name="refresh" />
        </button>
      </div>
    </header>
  );
}

interface StudioSidebarProps {
  readonly view: NavigationView;
  readonly tasks?: number | undefined;
  readonly scripts?: number | undefined;
  readonly plugins: number;
  readonly runs?: number | undefined;
  readonly sessions?: number | undefined;
  readonly onNavigate: (view: NavigationView) => void;
}

export function StudioSidebar({
  view,
  tasks,
  scripts,
  plugins,
  runs,
  sessions,
  onNavigate,
}: StudioSidebarProps) {
  return (
    <aside className="sidebar">
      <span className="nav-label">Work</span>
      <nav className="navigation" aria-label="Studio views">
        <NavButton
          active={view === "tasks"}
          icon="spark"
          label="Tasks"
          count={tasks}
          onClick={() => onNavigate("tasks")}
        />
        <span className="nav-label advanced-nav-label">Workspace tools</span>
        <NavButton
          active={view === "overview"}
          icon="overview"
          label="Overview"
          onClick={() => onNavigate("overview")}
        />
        <NavButton
          active={view === "workflow"}
          icon="workflow"
          label="Workflow"
          count={scripts}
          onClick={() => onNavigate("workflow")}
        />
        <NavButton
          active={view === "plugins"}
          icon="plugins"
          label="Plugins"
          count={plugins}
          onClick={() => onNavigate("plugins")}
        />
        <NavButton
          active={view === "runs"}
          icon="runs"
          label="Runs"
          count={runs}
          onClick={() => onNavigate("runs")}
        />
        <NavButton
          active={view === "sessions"}
          icon="sessions"
          label="Sessions"
          count={sessions}
          onClick={() => onNavigate("sessions")}
        />
      </nav>
      <div className="sidebar-foot">
        <strong>WORK THAT CARRIES FORWARD</strong>
        Goals, findings, and decisions
        <br />
        stay with your workspace.
      </div>
    </aside>
  );
}

function NavButton({
  active,
  icon,
  label,
  count,
  onClick,
}: {
  readonly active: boolean;
  readonly icon: IconName;
  readonly label: string;
  readonly count?: number | undefined;
  readonly onClick: () => void;
}) {
  return (
    <button
      type="button"
      className={`nav-button ${active ? "active" : ""}`}
      aria-current={active ? "page" : undefined}
      aria-label={label}
      title={label}
      onClick={onClick}
    >
      <Icon name={icon} />
      <span>{label}</span>
      {count !== undefined ? <small className="nav-count">{count}</small> : null}
    </button>
  );
}

export function LoadingView() {
  return (
    <div className="loading-view" role="status">
      <div>
        <span className="loading-mark" aria-hidden="true" />
        Reading local runtime
      </div>
    </div>
  );
}

export function ErrorToast({
  error,
  onDismiss,
}: {
  readonly error: string;
  readonly onDismiss: () => void;
}) {
  return (
    <div className="toast" role="alert">
      <Icon name="warning" />
      <span>{error}</span>
      <button type="button" aria-label="Dismiss error" onClick={onDismiss}>
        ×
      </button>
    </div>
  );
}

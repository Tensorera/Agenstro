import { Icon } from "./Icon";
import { StatusPill } from "./StatusPill";
import { formatRelative } from "../lib/format";
import type { TaskSummary } from "../types/segnoFlow";

export type TaskFilter = "all" | "running" | "failed" | "disabled";

interface TaskSidebarProps {
  tasks: TaskSummary[];
  selectedId: string | null;
  query: string;
  filter: TaskFilter;
  loading: boolean;
  onQueryChange: (value: string) => void;
  onFilterChange: (value: TaskFilter) => void;
  onSelect: (taskId: string) => void;
  onImport: () => void;
}

const filters: Array<{ value: TaskFilter; label: string }> = [
  { value: "all", label: "All" },
  { value: "running", label: "Running" },
  { value: "failed", label: "Failed" },
  { value: "disabled", label: "Paused" },
];

export function TaskSidebar({
  tasks,
  selectedId,
  query,
  filter,
  loading,
  onQueryChange,
  onFilterChange,
  onSelect,
  onImport,
}: TaskSidebarProps) {
  return (
    <aside className="task-sidebar" aria-label="Task navigation">
      <div className="sidebar-heading">
        <div>
          <p className="eyebrow">Workspace</p>
          <h2>Scheduled tasks</h2>
        </div>
        <span className="task-count" aria-label={`${tasks.length} visible tasks`}>
          {tasks.length}
        </span>
      </div>

      <label className="search-field">
        <span className="sr-only">Search tasks</span>
        <Icon name="search" />
        <input
          type="search"
          value={query}
          placeholder="Search tasks"
          onChange={(event) => onQueryChange(event.target.value)}
        />
      </label>

      <div className="filter-strip" aria-label="Filter tasks">
        {filters.map((item) => (
          <button
            type="button"
            key={item.value}
            className={filter === item.value ? "active" : ""}
            aria-pressed={filter === item.value}
            onClick={() => onFilterChange(item.value)}
          >
            {item.label}
          </button>
        ))}
      </div>

      <nav className="task-list" aria-label="Tasks" aria-busy={loading}>
        {loading ? (
          <div className="task-skeletons" aria-label="Loading tasks">
            {[0, 1, 2].map((item) => <span key={item} />)}
          </div>
        ) : tasks.length ? (
          tasks.map((task) => (
            <button
              key={task.id}
              type="button"
              className={`task-row ${selectedId === task.id ? "selected" : ""}`}
              aria-current={selectedId === task.id ? "page" : undefined}
              onClick={() => onSelect(task.id)}
            >
              <span className="task-row-top">
                <strong>{task.name}</strong>
                <StatusPill status={task.status} compact />
              </span>
              <span className="task-row-cron">
                <Icon name="calendar" />
                <code>{task.cron}</code>
                <span aria-hidden="true">·</span>
                {task.enabled ? formatRelative(task.nextRunAt) : "Paused"}
              </span>
            </button>
          ))
        ) : (
          <div className="sidebar-empty">
            <Icon name="search" />
            <strong>No matching tasks</strong>
            <span>Try another search or filter.</span>
          </div>
        )}
      </nav>

      <div className="sidebar-footer">
        <button type="button" className="secondary-button wide" onClick={onImport}>
          <Icon name="import" />
          Import task package
        </button>
        <p>ZIP packages are compiled before registration.</p>
      </div>
    </aside>
  );
}

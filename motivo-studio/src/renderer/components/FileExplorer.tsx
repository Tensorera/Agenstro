import type { FileEntry, FilePage, Workspace } from "../../shared/contracts";

interface DirectoryFrame {
  readonly id: FileEntry["id"];
  readonly name: string;
}

interface FileExplorerProps {
  readonly workspace: Workspace | null;
  readonly page: FilePage | null;
  readonly stack: readonly DirectoryFrame[];
  readonly busy: boolean;
  readonly pageCursor: string | undefined;
  onSelect(entry: FileEntry): void;
  onUp(): void;
  onNext(): void;
  onFirst(): void;
}

export function FileExplorer({
  workspace,
  page,
  stack,
  busy,
  pageCursor,
  onSelect,
  onUp,
  onNext,
  onFirst,
}: FileExplorerProps) {
  return (
    <section className="explorer-panel" aria-label="Paged workspace files">
      <div className="panel-heading">
        <span>FILES / PAGE</span>
        <strong>{workspace?.name ?? "OFFLINE"}</strong>
      </div>
      <div className="breadcrumbs" title={stack.map((part) => part.name).join(" / ")}>
        {stack.map((part) => part.name).join(" / ") || "Open a workspace"}
      </div>
      <div className="file-list">
        {page?.entries.map((entry) => (
          <button
            type="button"
            key={entry.id}
            disabled={busy}
            className="file-row"
            onClick={() => onSelect(entry)}
          >
            <span aria-hidden="true">{entry.kind === "directory" ? "D" : "F"}</span>
            <span>{entry.name}</span>
            <small>{entry.kind === "directory" ? "DIR" : formatBytes(entry.sizeBytes)}</small>
          </button>
        ))}
        {workspace && page?.entries.length === 0 ? (
          <p className="panel-empty">This bounded page is empty.</p>
        ) : null}
        {!workspace ? (
          <p className="panel-empty">Choose a directory through the desktop picker.</p>
        ) : null}
      </div>
      <div className="pager">
        <button type="button" disabled={busy || stack.length <= 1} onClick={onUp}>
          Up
        </button>
        <button type="button" disabled={busy || pageCursor === undefined} onClick={onFirst}>
          First
        </button>
        <button type="button" disabled={busy || !page?.nextCursor} onClick={onNext}>
          Next
        </button>
      </div>
    </section>
  );
}

function formatBytes(bytes: number): string {
  if (bytes < 1_024) return `${String(bytes)} B`;
  if (bytes < 1_048_576) return `${(bytes / 1_024).toFixed(1)} KB`;
  return `${(bytes / 1_048_576).toFixed(1)} MB`;
}

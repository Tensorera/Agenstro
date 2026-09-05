import { useState } from "react";
import type { StudioScript, StudioView } from "../../shared/contracts";
import { Icon } from "./Icon";
import { PanelHeader, ViewHeader } from "./Primitives";

interface WorkflowViewProps {
  readonly studio: StudioView | null;
  readonly goal: string;
  readonly provider: string;
  readonly running: boolean;
  readonly onGoal: (goal: string) => void;
  readonly onProvider: (provider: string) => void;
  readonly onGenerate: () => void;
  readonly onCheck: (scripts: string[]) => void;
  readonly onRun: (scripts: string[]) => void;
}

export function WorkflowView({
  studio,
  goal,
  provider,
  running,
  onGoal,
  onProvider,
  onGenerate,
  onCheck,
  onRun,
}: WorkflowViewProps) {
  const snapshot = studio?.snapshot;
  const scripts = snapshot?.scripts ?? [];
  const entries = scripts.filter((script) => script.runnable);
  const [selection, setSelection] = useState<string[]>([]);
  const selected = scripts.filter((script) => selection.includes(script.relativePath));
  const selectedEntries = selected.filter((script) => script.runnable);
  function toggle(path: string): void {
    setSelection((current) =>
      current.includes(path) ? current.filter((item) => item !== path) : [...current, path],
    );
  }

  return (
    <>
      <ViewHeader
        eyebrow="Typed composition"
        title="Workflow"
        description="Maintain reusable workflows. Select the sources to check or the numbered entries to run."
      />
      <div className="content-width workflow-grid">
        <article className="panel">
          <PanelHeader
            title="Generate scripts"
            subtitle="A real provider call that may edit this workspace"
          />
          <form
            className="generate-form"
            onSubmit={(event) => {
              event.preventDefault();
              if (goal.trim()) onGenerate();
            }}
          >
            <div className="field">
              <label htmlFor="workflow-goal">Workflow goal</label>
              <textarea
                id="workflow-goal"
                value={goal}
                disabled={!snapshot || running}
                placeholder="Describe the reusable workflow or the specific change you need…"
                onChange={(event) => onGoal(event.target.value)}
              />
            </div>
            <div className="field">
              <label htmlFor="workflow-provider">Provider</label>
              <select
                id="workflow-provider"
                value={provider}
                disabled={!snapshot || running || snapshot.registries.providers.length === 0}
                onChange={(event) => onProvider(event.target.value)}
              >
                {snapshot?.registries.providers.map((item) => (
                  <option value={item.name} key={item.name}>
                    {item.name}
                    {item.default ? " · default" : ""}
                    {item.available ? "" : " · unavailable"}
                  </option>
                ))}
              </select>
            </div>
            <button
              type="submit"
              className="button primary"
              disabled={!snapshot || running || !goal.trim() || !provider}
            >
              <Icon name="spark" /> Generate workflow
            </button>
            <p className="form-note">
              Generation updates workflow sources in this workspace. Review the selected files
              before running them.
            </p>
          </form>
        </article>

        <article className="panel">
          <PanelHeader
            title="Script sequence"
            subtitle={`${entries.length} entries · ${scripts.length - entries.length} helpers`}
            action={
              <div className="inline-actions">
                <button
                  type="button"
                  className="button compact"
                  disabled={!snapshot || running || selected.length === 0}
                  onClick={() => onCheck(selected.map((script) => script.relativePath))}
                >
                  <Icon name="check" /> Check
                </button>
                <button
                  type="button"
                  className="button compact primary"
                  disabled={!snapshot || running || selectedEntries.length === 0}
                  onClick={() => onRun(selectedEntries.map((script) => script.relativePath))}
                >
                  <Icon name="play" /> Run
                </button>
              </div>
            }
          />
          <div className="script-selection-toolbar">
            <span>
              {selected.length} sources selected · {selectedEntries.length} runnable
            </span>
            <div className="inline-actions">
              <button
                type="button"
                className="button compact ghost"
                disabled={running || !entries.length}
                onClick={() => setSelection(entries.map((script) => script.relativePath))}
              >
                Select entries
              </button>
              <button
                type="button"
                className="button compact ghost"
                disabled={running || !scripts.length}
                onClick={() => setSelection(scripts.map((script) => script.relativePath))}
              >
                Select all sources
              </button>
              <button
                type="button"
                className="button compact ghost"
                disabled={running || !selected.length}
                onClick={() => setSelection([])}
              >
                Clear
              </button>
            </div>
          </div>
          <div className="script-stack">
            {scripts.length === 0 ? (
              <div className="empty-list">
                {snapshot
                  ? "No Haskell sources discovered yet."
                  : "Open a workspace to inspect its workflow."}
              </div>
            ) : (
              scripts.map((script, index) => (
                <ScriptRow
                  script={script}
                  index={index}
                  key={script.relativePath}
                  checked={selection.includes(script.relativePath)}
                  disabled={running}
                  onToggle={() => toggle(script.relativePath)}
                />
              ))
            )}
          </div>
        </article>
      </div>
    </>
  );
}

function ScriptRow({
  script,
  index,
  checked,
  disabled,
  onToggle,
}: {
  readonly script: StudioScript;
  readonly index: number;
  readonly checked: boolean;
  readonly disabled: boolean;
  readonly onToggle: () => void;
}) {
  const filename = script.relativePath.split("/").at(-1) ?? script.relativePath;
  return (
    <label className="script-row selectable-script">
      <input
        type="checkbox"
        checked={checked}
        disabled={disabled}
        onChange={onToggle}
        aria-label={`Select ${filename}`}
      />
      <span className="script-order">{script.order?.toString().padStart(3, "0") ?? "H"}</span>
      <div className="script-copy">
        <strong>{filename}</strong>
        <span>{script.relativePath}</span>
      </div>
      <span className={`pill ${script.runnable ? "running" : ""}`}>
        {script.runnable ? `step ${index + 1}` : "helper"}
      </span>
    </label>
  );
}

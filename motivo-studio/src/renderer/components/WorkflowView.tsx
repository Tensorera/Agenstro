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
  readonly onCheck: () => void;
  readonly onRun: () => void;
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

  return (
    <>
      <ViewHeader
        eyebrow="Typed composition"
        title="Workflow"
        description="Generate small Haskell steps, type-check the complete source set, then run numbered entries through Tactus."
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
                placeholder="Describe the outcome and let the provider split it into small, ordered DSL scripts…"
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
              Generation is intentionally powerful: the configured coding agent receives the local
              workspace and may write several numbered `.hs` or `.lhs` files.
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
                  disabled={!snapshot || running || scripts.length === 0}
                  onClick={onCheck}
                >
                  <Icon name="check" /> Check
                </button>
                <button
                  type="button"
                  className="button compact primary"
                  disabled={!snapshot || running || entries.length === 0}
                  onClick={onRun}
                >
                  <Icon name="play" /> Run
                </button>
              </div>
            }
          />
          <div className="script-stack">
            {scripts.length === 0 ? (
              <div className="empty-list">
                {snapshot
                  ? "No Haskell sources discovered yet."
                  : "Open a workspace to inspect its workflow."}
              </div>
            ) : (
              scripts.map((script, index) => (
                <ScriptRow script={script} index={index} key={script.relativePath} />
              ))
            )}
          </div>
        </article>
      </div>
    </>
  );
}

function ScriptRow({ script, index }: { readonly script: StudioScript; readonly index: number }) {
  const filename = script.relativePath.split("/").at(-1) ?? script.relativePath;
  return (
    <div className="script-row">
      <span className="script-order">{script.order?.toString().padStart(3, "0") ?? "H"}</span>
      <div className="script-copy">
        <strong>{filename}</strong>
        <span>{script.relativePath}</span>
      </div>
      <span className={`pill ${script.runnable ? "running" : ""}`}>
        {script.runnable ? `step ${index + 1}` : "helper"}
      </span>
    </div>
  );
}

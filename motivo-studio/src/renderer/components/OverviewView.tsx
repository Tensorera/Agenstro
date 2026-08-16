import type { ActionRequest, StudioView } from "../../shared/contracts";
import { dayPeriod } from "../format";
import type { NavigationView } from "../model";
import { Icon } from "./Icon";
import { Metric, PanelHeader, RunTable, ViewHeader } from "./Primitives";

interface OverviewViewProps {
  readonly studio: StudioView | null;
  readonly running: boolean;
  readonly workspaceBusy: boolean;
  readonly onOpen: () => void;
  readonly onInitialize: () => void;
  readonly onNavigate: (view: NavigationView) => void;
  readonly onAction: (request: ActionRequest) => void;
}

export function OverviewView({
  studio,
  running,
  workspaceBusy,
  onOpen,
  onInitialize,
  onNavigate,
  onAction,
}: OverviewViewProps) {
  const snapshot = studio?.snapshot;
  if (!snapshot) {
    return (
      <div className="content-width">
        <section className="empty-workspace" aria-label="No workspace">
          <div>
            <span className="empty-symbol">
              <Icon name="workflow" />
            </span>
            <p className="eyebrow">Tactus control surface</p>
            <h2>Bring a workflow into focus.</h2>
            <p>
              Open an initialized workspace to inspect its typed scripts, plugins, and factual run
              history. Or initialize a folder with Tactus in one step.
            </p>
            <div className="empty-actions">
              <button
                type="button"
                className="button primary"
                disabled={workspaceBusy || running}
                onClick={onOpen}
              >
                <Icon name="folder" /> Open workspace
              </button>
              <button
                type="button"
                className="button"
                disabled={workspaceBusy || running}
                onClick={onInitialize}
              >
                <Icon name="plus" /> Initialize folder
              </button>
            </div>
          </div>
        </section>
      </div>
    );
  }

  const plugins = [
    ...snapshot.registries.providers,
    ...snapshot.registries.effects,
    ...snapshot.registries.plugins,
  ];
  const availablePlugins = plugins.filter((plugin) => plugin.available).length;
  const runnableScripts = snapshot.scripts.filter((script) => script.runnable).length;
  const successfulRuns = snapshot.runs.filter((run) => run.outcome?.kind === "succeeded").length;

  return (
    <>
      <ViewHeader
        eyebrow="Workspace pulse"
        title={`Good ${dayPeriod()}, ${snapshot.workspace.name}`}
        description="A concise view of the typed workflow, runtime health, plugin surface, and recent execution evidence."
      />
      <div className="content-width">
        <section className="metric-grid" aria-label="Workspace metrics">
          <Metric
            label="Workflow entries"
            value={runnableScripts}
            note={`${snapshot.scripts.length} total sources`}
          />
          <Metric
            label="Plugins online"
            value={`${availablePlugins}/${plugins.length}`}
            note={`${snapshot.registries.providers.length} providers`}
          />
          <Metric
            label="Recent runs"
            value={snapshot.runs.length}
            note={`${successfulRuns} succeeded`}
          />
          <Metric
            label="Runtime health"
            value={snapshot.health.ok ? "Ready" : "Check"}
            note={`${snapshot.health.checks.length} checks`}
          />
        </section>

        <section className="overview-grid">
          <div>
            <article className="panel">
              <PanelHeader
                title="Quick actions"
                subtitle="One supervised Tactus action at a time"
              />
              <div className="panel-body quick-actions">
                <button
                  type="button"
                  className="quick-card"
                  disabled={running}
                  onClick={() => onNavigate("workflow")}
                >
                  <Icon name="spark" />
                  <strong>Generate workflow</strong>
                  <small>Draft numbered Haskell scripts with a provider.</small>
                </button>
                <button
                  type="button"
                  className="quick-card"
                  disabled={running || snapshot.scripts.length === 0}
                  onClick={() => onAction({ kind: "check" })}
                >
                  <Icon name="check" />
                  <strong>Type-check all</strong>
                  <small>Compile every discovered source without running it.</small>
                </button>
                <button
                  type="button"
                  className="quick-card"
                  disabled={running || runnableScripts === 0}
                  onClick={() => onAction({ kind: "run" })}
                >
                  <Icon name="play" />
                  <strong>Run entries</strong>
                  <small>Execute the numbered workflow in deterministic order.</small>
                </button>
              </div>
            </article>

            <article className="table-shell section-gap">
              <PanelHeader
                title="Recent runs"
                subtitle="Local, append-only trace summaries"
                action={
                  <button
                    type="button"
                    className="button compact ghost"
                    onClick={() => onNavigate("runs")}
                  >
                    View all <Icon name="arrow" />
                  </button>
                }
              />
              <RunTable runs={snapshot.runs.slice(0, 6)} onSelect={() => onNavigate("runs")} />
            </article>
          </div>

          <article className="panel">
            <PanelHeader
              title="Runtime checks"
              subtitle={snapshot.health.ok ? "All required tools resolved" : "Action required"}
              action={
                <span className={`pill ${snapshot.health.ok ? "success" : "warning"}`}>
                  {snapshot.health.ok ? "healthy" : "degraded"}
                </span>
              }
            />
            <div className="panel-body">
              <ul className="health-list">
                {snapshot.health.checks.map((check) => (
                  <li className="health-row" key={check.name} title={check.detail}>
                    <span
                      className={`status-dot ${check.ok ? "ready" : "failed"}`}
                      aria-hidden="true"
                    />
                    <strong>{check.name}</strong>
                    <span>{check.ok ? "ready" : "missing"}</span>
                  </li>
                ))}
              </ul>
            </div>
          </article>
        </section>
      </div>
    </>
  );
}

import type { StudioPlugin, StudioView } from "../../shared/contracts";
import { Icon } from "./Icon";
import { ViewHeader } from "./Primitives";

interface PluginsViewProps {
  readonly studio: StudioView | null;
  readonly running: boolean;
  readonly onSmoke: (plugin: StudioPlugin, live: boolean) => void;
}

export function PluginsView({ studio, running, onSmoke }: PluginsViewProps) {
  const registries = studio?.snapshot.registries;
  const groups: readonly [string, readonly StudioPlugin[]][] = [
    ["Providers", registries?.providers ?? []],
    ["Effects", registries?.effects ?? []],
    ["Generic plugins", registries?.plugins ?? []],
  ];

  return (
    <>
      <ViewHeader
        eyebrow="Open extension surface"
        title="Plugins"
        description="Inspect the three typed registries and probe each adapter. Offline smoke checks connectivity; live smoke may contact an external provider."
      />
      <div className="content-width">
        {groups.map(([label, plugins]) => (
          <section className="plugin-section" key={label} aria-label={label}>
            <div className="section-title">
              <h2>{label}</h2>
              <span>{plugins.length.toString().padStart(2, "0")}</span>
            </div>
            {plugins.length === 0 ? (
              <div className="panel empty-list">No entries configured in this registry.</div>
            ) : (
              <div className="plugin-grid">
                {plugins.map((plugin) => (
                  <PluginCard
                    plugin={plugin}
                    busy={running}
                    onSmoke={(live) => onSmoke(plugin, live)}
                    key={`${plugin.namespace}:${plugin.name}`}
                  />
                ))}
              </div>
            )}
          </section>
        ))}
      </div>
    </>
  );
}

function PluginCard({
  plugin,
  busy,
  onSmoke,
}: {
  readonly plugin: StudioPlugin;
  readonly busy: boolean;
  readonly onSmoke: (live: boolean) => void;
}) {
  return (
    <article className="plugin-card">
      <div className="plugin-card-head">
        <span className="plugin-glyph" aria-hidden="true">
          {plugin.namespace.slice(0, 2)}
        </span>
        <div>
          <h3>{plugin.name}</h3>
          <small>{plugin.namespace}</small>
        </div>
        <span className={`pill ${plugin.available ? "success" : "failed"}`}>
          {plugin.available ? "available" : "unavailable"}
        </span>
      </div>
      <div className="plugin-meta">
        <div>
          <span>Model</span>
          <strong title={plugin.model}>{plugin.model ?? "provider default"}</strong>
        </div>
        <div>
          <span>{plugin.namespace === "effect" ? "Observer" : "Effort"}</span>
          <strong>
            {plugin.namespace === "effect"
              ? plugin.observesInvocations
                ? "enabled"
                : "disabled"
              : (plugin.effort ?? "provider default")}
          </strong>
        </div>
      </div>
      <div className="plugin-actions">
        <button
          type="button"
          className="button compact"
          disabled={busy || !plugin.available}
          onClick={() => onSmoke(false)}
        >
          <Icon name="pulse" /> Offline smoke
        </button>
        <button
          type="button"
          className="button compact danger"
          title="May make a real provider request"
          disabled={busy || !plugin.available}
          onClick={() => onSmoke(true)}
        >
          <Icon name="bolt" /> Live smoke
        </button>
      </div>
    </article>
  );
}

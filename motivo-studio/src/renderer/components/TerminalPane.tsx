import { useEffect, useRef, useState } from "react";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import type { StreamHandle, TerminalId, TerminalProfile, Workspace } from "../../shared/contracts";
import { TerminalWriteQueue } from "../features/terminal-write-queue";

interface TerminalPaneProps {
  readonly workspace: Workspace | null;
}

export function TerminalPane({ workspace }: TerminalPaneProps) {
  const host = useRef<HTMLDivElement>(null);
  const terminal = useRef<Terminal | null>(null);
  const fit = useRef<FitAddon | null>(null);
  const sessionId = useRef<TerminalId | null>(null);
  const writeQueue = useRef(new TerminalWriteQueue());
  const [profiles, setProfiles] = useState<readonly TerminalProfile[]>([]);
  const [profileId, setProfileId] = useState<"powershell" | "bash">("powershell");
  const [status, setStatus] = useState("No workspace terminal");
  const workspaceId = workspace?.id;

  useEffect(() => {
    if (!host.current) return;
    const instance = new Terminal({
      allowProposedApi: false,
      convertEol: false,
      cursorBlink: true,
      cursorStyle: "bar",
      fontFamily: '"Cascadia Mono", "SFMono-Regular", Consolas, monospace',
      fontSize: 12,
      lineHeight: 1.2,
      scrollback: 5_000,
      theme: {
        background: "#0c100e",
        foreground: "#d8e0da",
        cursor: "#ef9a51",
        selectionBackground: "#335947",
        black: "#101512",
        red: "#e4756b",
        green: "#93c99b",
        yellow: "#d5b16b",
        blue: "#79a9d1",
        magenta: "#c395c9",
        cyan: "#78b9b1",
        white: "#e4ebe5",
      },
    });
    const fitAddon = new FitAddon();
    instance.loadAddon(fitAddon);
    instance.open(host.current);
    fitAddon.fit();
    terminal.current = instance;
    fit.current = fitAddon;
    const dataListener = instance.onData((data) => {
      const current = sessionId.current;
      if (!current) return;
      const accepted = writeQueue.current.enqueue(
        current,
        data,
        window.motivo.terminals.write,
        (caught) => setStatus(caught instanceof Error ? caught.message : "Terminal input failed"),
      );
      if (!accepted) setStatus("Terminal input backpressure limit reached");
    });
    const observer = new ResizeObserver(() => {
      fitAddon.fit();
      const current = sessionId.current;
      if (current) {
        void window.motivo.terminals
          .resize({ terminalId: current, cols: instance.cols, rows: instance.rows })
          .catch(() => undefined);
      }
    });
    observer.observe(host.current);
    return () => {
      observer.disconnect();
      dataListener.dispose();
      instance.dispose();
      terminal.current = null;
      fit.current = null;
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    void window.motivo.terminals
      .profiles()
      .then((available) => {
        if (cancelled) return;
        setProfiles(available);
        const preferred = available.find((profile) => profile.available);
        if (preferred) setProfileId(preferred.id);
      })
      .catch((caught: unknown) => {
        if (!cancelled)
          setStatus(caught instanceof Error ? caught.message : "PTY broker unavailable");
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (
      !workspaceId ||
      !terminal.current ||
      !profiles.some((profile) => profile.id === profileId && profile.available)
    ) {
      return;
    }
    let disposed = false;
    let stream: StreamHandle | undefined;
    let createdId: TerminalId | undefined;
    const queue = writeQueue.current;
    queue.reset();
    terminal.current.reset();
    fit.current?.fit();
    setStatus("Starting isolated PTY broker session...");
    void window.motivo.terminals
      .create({
        workspaceId,
        profileId,
        cols: terminal.current.cols,
        rows: terminal.current.rows,
      })
      .then(async (session) => {
        createdId = session.id;
        if (disposed) {
          await window.motivo.terminals.close({ terminalId: session.id });
          return;
        }
        sessionId.current = session.id;
        setStatus(
          `${profiles.find((profile) => profile.id === profileId)?.label ?? profileId} ready`,
        );
        const subscription = await window.motivo.terminals.subscribe(
          { terminalId: session.id },
          (message) => {
            if (disposed || message.terminalId !== session.id) return;
            if (message.kind === "output") {
              terminal.current?.write(message.data, () => {
                void window.motivo.terminals
                  .ack({ terminalId: session.id, highestSequence: message.sequence })
                  .catch(() => undefined);
              });
            } else {
              setStatus(
                message.reason === "output-backpressure"
                  ? "Terminal stopped: output backpressure"
                  : `Terminal exited${message.exitCode === null ? "" : ` (${String(message.exitCode)})`}`,
              );
              sessionId.current = null;
            }
          },
        );
        if (disposed) {
          await subscription.unsubscribe();
          return;
        }
        stream = subscription;
      })
      .catch((caught: unknown) => {
        if (!disposed) {
          sessionId.current = null;
          if (createdId) {
            void window.motivo.terminals.close({ terminalId: createdId }).catch(() => undefined);
          }
          setStatus(caught instanceof Error ? caught.message : "Terminal unavailable");
        }
      });
    return () => {
      disposed = true;
      queue.reset();
      sessionId.current = null;
      if (stream) void stream.unsubscribe().catch(() => undefined);
      if (createdId)
        void window.motivo.terminals.close({ terminalId: createdId }).catch(() => undefined);
    };
  }, [workspaceId, profileId, profiles]);

  return (
    <section className="terminal-panel" aria-label="Terminal">
      <div className="pane-title">
        <span>TERMINAL / PTY BROKER</span>
        <label>
          <span className="sr-only">Terminal profile</span>
          <select
            aria-label="Terminal profile"
            value={profileId}
            disabled={!workspace}
            onChange={(event) =>
              setProfileId(event.target.value === "bash" ? "bash" : "powershell")
            }
          >
            {profiles.map((profile) => (
              <option key={profile.id} value={profile.id} disabled={!profile.available}>
                {profile.label}
                {profile.available ? "" : " (unavailable)"}
              </option>
            ))}
          </select>
        </label>
        <small>{status}</small>
      </div>
      <div ref={host} className="xterm-host" />
    </section>
  );
}

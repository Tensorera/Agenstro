import { useEffect, useRef } from "react";
import * as monaco from "monaco-editor/esm/vs/editor/editor.api";
import EditorWorker from "monaco-editor/esm/vs/editor/editor.worker?worker";
import CssWorker from "monaco-editor/esm/vs/language/css/css.worker?worker";
import HtmlWorker from "monaco-editor/esm/vs/language/html/html.worker?worker";
import JsonWorker from "monaco-editor/esm/vs/language/json/json.worker?worker";
import TypeScriptWorker from "monaco-editor/esm/vs/language/typescript/ts.worker?worker";
import type { EntryId, WorkspaceId } from "../../shared/contracts";

window.MonacoEnvironment = {
  getWorker(_moduleId: string, label: string): Worker {
    if (label === "json") return new JsonWorker();
    if (label === "css" || label === "scss" || label === "less") return new CssWorker();
    if (label === "html" || label === "handlebars" || label === "razor") return new HtmlWorker();
    if (label === "typescript" || label === "javascript") return new TypeScriptWorker();
    return new EditorWorker();
  },
};

monaco.editor.defineTheme("motivo-night", {
  base: "vs-dark",
  inherit: true,
  rules: [
    { token: "comment", foreground: "69756D" },
    { token: "keyword", foreground: "E89A55" },
    { token: "string", foreground: "A7D1AA" },
    { token: "number", foreground: "D6B7E6" },
  ],
  colors: {
    "editor.background": "#111512",
    "editor.foreground": "#DDE4DE",
    "editorLineNumber.foreground": "#4F5C54",
    "editorLineNumber.activeForeground": "#B7C1B9",
    "editor.selectionBackground": "#33594788",
    "editorCursor.foreground": "#F09A50",
    "editorIndentGuide.background1": "#27312B",
  },
});

interface EditorPaneProps {
  readonly workspaceId: WorkspaceId;
  readonly entryId: EntryId;
  readonly path: string;
  readonly revision: string;
  readonly language: string;
  readonly value: string;
  readonly readOnly: boolean;
  onChange(value: string): void;
}

export function EditorPane({
  workspaceId,
  entryId,
  path,
  revision,
  language,
  value,
  readOnly,
  onChange,
}: EditorPaneProps) {
  const host = useRef<HTMLDivElement>(null);
  const editor = useRef<monaco.editor.IStandaloneCodeEditor | null>(null);
  const model = useRef<monaco.editor.ITextModel | null>(null);
  const modelListener = useRef<monaco.IDisposable | null>(null);
  const changeListener = useRef(onChange);
  const currentValue = useRef(value);
  const suppressChange = useRef(false);

  useEffect(() => {
    changeListener.current = onChange;
  }, [onChange]);

  useEffect(() => {
    currentValue.current = value;
  }, [value]);

  useEffect(() => {
    if (!host.current) return;
    editor.current = monaco.editor.create(host.current, {
      automaticLayout: false,
      fontFamily: '"Cascadia Mono", "SFMono-Regular", Consolas, monospace',
      fontSize: 13,
      lineHeight: 21,
      minimap: { enabled: false },
      padding: { top: 16, bottom: 18 },
      renderWhitespace: "selection",
      scrollBeyondLastLine: false,
      smoothScrolling: true,
      tabSize: 4,
      theme: "motivo-night",
    });
    const observer = new ResizeObserver(() => editor.current?.layout());
    observer.observe(host.current);
    return () => {
      observer.disconnect();
      editor.current?.dispose();
      editor.current = null;
    };
  }, []);

  useEffect(() => {
    modelListener.current?.dispose();
    editor.current?.setModel(null);
    model.current?.dispose();
    const uri = monaco.Uri.parse(
      `motivo://workspace/${encodeURIComponent(workspaceId)}/${encodeURIComponent(path)}?entry=${encodeURIComponent(entryId)}&revision=${encodeURIComponent(revision)}`,
    );
    const nextModel = monaco.editor.createModel(currentValue.current, language || "plaintext", uri);
    model.current = nextModel;
    editor.current?.setModel(nextModel);
    modelListener.current = nextModel.onDidChangeContent(() => {
      if (!suppressChange.current) changeListener.current(nextModel.getValue());
    });
    editor.current?.layout();
    return () => {
      modelListener.current?.dispose();
      editor.current?.setModel(null);
      nextModel.dispose();
      if (model.current === nextModel) model.current = null;
    };
  }, [workspaceId, entryId, path, revision, language]);

  useEffect(() => {
    if (model.current && model.current.getValue() !== value) {
      suppressChange.current = true;
      model.current.setValue(value);
      suppressChange.current = false;
    }
  }, [value]);

  useEffect(() => {
    editor.current?.updateOptions({ readOnly });
  }, [readOnly]);

  return <div ref={host} className="monaco-host" aria-label={`Editor for ${path}`} />;
}

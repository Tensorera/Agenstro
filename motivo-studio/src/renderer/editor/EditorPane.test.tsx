import { render, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { entryIdSchema, workspaceIdSchema } from "../../shared/contracts";
import { EditorPane } from "./EditorPane";

const monacoMock = vi.hoisted(() => ({
  models: [] as Array<{
    dispose: ReturnType<typeof vi.fn>;
    getValue: () => string;
    setValue: (value: string) => void;
    onDidChangeContent: () => { dispose: ReturnType<typeof vi.fn> };
  }>,
  editorDispose: vi.fn(),
}));

vi.mock("monaco-editor/esm/vs/editor/editor.api", () => ({
  Uri: { parse: (value: string) => value },
  editor: {
    defineTheme: vi.fn(),
    create: vi.fn(() => ({
      setModel: vi.fn(),
      layout: vi.fn(),
      updateOptions: vi.fn(),
      dispose: monacoMock.editorDispose,
    })),
    createModel: vi.fn((initial: string) => {
      let value = initial;
      const model = {
        dispose: vi.fn(),
        getValue: () => value,
        setValue: (next: string) => {
          value = next;
        },
        onDidChangeContent: () => ({ dispose: vi.fn() }),
      };
      monacoMock.models.push(model);
      return model;
    }),
  },
}));

vi.mock("monaco-editor/esm/vs/editor/editor.worker?worker", () => ({
  default: class TestEditorWorker {
    postMessage(): void {}
  },
}));
vi.mock("monaco-editor/esm/vs/language/css/css.worker?worker", () => ({
  default: class TestCssWorker {
    postMessage(): void {}
  },
}));
vi.mock("monaco-editor/esm/vs/language/html/html.worker?worker", () => ({
  default: class TestHtmlWorker {
    postMessage(): void {}
  },
}));
vi.mock("monaco-editor/esm/vs/language/json/json.worker?worker", () => ({
  default: class TestJsonWorker {
    postMessage(): void {}
  },
}));
vi.mock("monaco-editor/esm/vs/language/typescript/ts.worker?worker", () => ({
  default: class TestTypeScriptWorker {
    postMessage(): void {}
  },
}));

describe("Monaco model ownership", () => {
  beforeEach(() => {
    monacoMock.models.length = 0;
    monacoMock.editorDispose.mockClear();
  });

  it("disposes the old revision model and the active model on unmount", async () => {
    const common = {
      workspaceId: workspaceIdSchema.parse("workspace-1"),
      entryId: entryIdSchema.parse("entry-1"),
      path: "main.py",
      language: "python",
      value: "print(1)",
      readOnly: false,
      onChange: vi.fn(),
    };
    const view = render(<EditorPane {...common} revision="revision-1" />);
    await waitFor(() => expect(monacoMock.models).toHaveLength(1));
    const first = monacoMock.models[0];

    view.rerender(<EditorPane {...common} revision="revision-2" value="print(2)" />);
    await waitFor(() => expect(monacoMock.models).toHaveLength(2));
    expect(first?.dispose).toHaveBeenCalledOnce();
    const second = monacoMock.models[1];

    view.unmount();
    expect(second?.dispose).toHaveBeenCalledOnce();
    expect(monacoMock.editorDispose).toHaveBeenCalledOnce();
  });
});

import type { BrowserWindow, Input } from "electron";
import { describe, expect, it, vi } from "vitest";
import { DEFAULT_ZOOM_FACTOR, installWindowZoom } from "../../src/main/windows/zoom";

describe("Electron window zoom", () => {
  it("starts larger and supports bounded wheel and keyboard zoom", () => {
    const handlers = new Map<string, (...values: never[]) => void>();
    let factor = 1;
    const setZoomFactor = vi.fn((value: number) => {
      factor = value;
    });
    const fake = {
      webContents: {
        getZoomFactor: () => factor,
        setZoomFactor,
        once: vi.fn((name: string, handler: (...values: never[]) => void) =>
          handlers.set(name, handler),
        ),
        on: vi.fn((name: string, handler: (...values: never[]) => void) =>
          handlers.set(name, handler),
        ),
      },
    } as unknown as BrowserWindow;

    installWindowZoom(fake);
    handlers.get("did-finish-load")?.();
    expect(factor).toBe(DEFAULT_ZOOM_FACTOR);

    const wheel = { preventDefault: vi.fn() };
    handlers.get("zoom-changed")?.(wheel as never, "in" as never);
    expect(wheel.preventDefault).toHaveBeenCalledOnce();
    expect(factor).toBe(1.3);

    const keyboard = { preventDefault: vi.fn() };
    handlers.get("before-input-event")?.(
      keyboard as never,
      { type: "keyDown", control: true, meta: false, key: "-", code: "Minus" } as Input as never,
    );
    expect(keyboard.preventDefault).toHaveBeenCalledOnce();
    expect(factor).toBe(DEFAULT_ZOOM_FACTOR);

    for (let index = 0; index < 20; index += 1) {
      handlers.get("zoom-changed")?.(wheel as never, "out" as never);
    }
    expect(factor).toBe(0.8);
  });
});

import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach, vi } from "vitest";

class TestResizeObserver implements ResizeObserver {
  readonly callback: ResizeObserverCallback;

  constructor(callback: ResizeObserverCallback) {
    this.callback = callback;
  }

  disconnect(): void {}
  observe(): void {}
  unobserve(): void {}
}

Object.defineProperty(window, "ResizeObserver", {
  configurable: true,
  value: TestResizeObserver,
});
Object.defineProperty(globalThis, "ResizeObserver", {
  configurable: true,
  value: TestResizeObserver,
});
Object.defineProperty(window, "requestAnimationFrame", {
  configurable: true,
  value: (callback: FrameRequestCallback) => window.setTimeout(() => callback(0), 0),
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

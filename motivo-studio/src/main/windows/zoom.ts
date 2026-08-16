import type { BrowserWindow, Input } from "electron";

export const DEFAULT_ZOOM_FACTOR = 1.2;
const MINIMUM_ZOOM_FACTOR = 0.8;
const MAXIMUM_ZOOM_FACTOR = 2;
const ZOOM_STEP = 0.1;

/** Install bounded, application-level zoom without exposing Electron to the renderer. */
export function installWindowZoom(window: BrowserWindow): void {
  const setZoom = (value: number): void => {
    const bounded = Math.min(MAXIMUM_ZOOM_FACTOR, Math.max(MINIMUM_ZOOM_FACTOR, value));
    window.webContents.setZoomFactor(Math.round(bounded * 10) / 10);
  };
  const adjustZoom = (delta: number): void => {
    setZoom(window.webContents.getZoomFactor() + delta);
  };

  window.webContents.once("did-finish-load", () => setZoom(DEFAULT_ZOOM_FACTOR));
  window.webContents.on("zoom-changed", (event, direction) => {
    event.preventDefault();
    adjustZoom(direction === "in" ? ZOOM_STEP : -ZOOM_STEP);
  });
  window.webContents.on("before-input-event", (event, input) => {
    if (input.type !== "keyDown" || (!input.control && !input.meta)) return;
    const action = zoomAction(input);
    if (!action) return;
    event.preventDefault();
    if (action === "reset") setZoom(DEFAULT_ZOOM_FACTOR);
    else adjustZoom(action === "in" ? ZOOM_STEP : -ZOOM_STEP);
  });
}

function zoomAction(input: Input): "in" | "out" | "reset" | null {
  if (input.code === "NumpadAdd" || input.key === "+" || input.key === "=") return "in";
  if (input.code === "NumpadSubtract" || input.key === "-" || input.key === "_") return "out";
  if (input.code === "Numpad0" || input.key === "0") return "reset";
  return null;
}

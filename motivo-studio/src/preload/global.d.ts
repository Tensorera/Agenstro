import type { MotivoBridge } from "../shared/contracts";

declare global {
  interface Window {
    readonly motivo: MotivoBridge;
  }
}

export {};

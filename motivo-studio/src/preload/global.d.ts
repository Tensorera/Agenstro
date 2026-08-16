import type { MotivoBridge } from "../shared/contracts";

declare global {
  interface Window {
    readonly motivo: MotivoBridge;
    MonacoEnvironment?: {
      getWorker(moduleId: string, label: string): Worker;
    };
  }
}

export {};

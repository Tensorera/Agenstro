import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach, beforeEach, vi } from "vitest";
import { resetMockSegnoFlow } from "../api/segnoFlow";

beforeEach(() => {
  resetMockSegnoFlow();
  delete window.pywebview;
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

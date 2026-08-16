import { z } from "zod";

export const studioSurfaceSchema = z.enum(["files", "scheduler"]);
export type StudioSurface = z.infer<typeof studioSurfaceSchema>;

export const DEFAULT_STUDIO_SURFACE: StudioSurface = "files";

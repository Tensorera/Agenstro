import type { ReactNode } from "react";

export type IconName =
  | "arrow"
  | "bolt"
  | "check"
  | "close"
  | "down"
  | "folder"
  | "overview"
  | "play"
  | "plugins"
  | "plus"
  | "pulse"
  | "refresh"
  | "runs"
  | "spark"
  | "stop"
  | "warning"
  | "workflow";

export function Icon({ name }: { readonly name: IconName }) {
  const paths: Record<IconName, ReactNode> = {
    overview: (
      <>
        <rect x="3" y="3" width="7" height="7" />
        <rect x="14" y="3" width="7" height="7" />
        <rect x="3" y="14" width="7" height="7" />
        <rect x="14" y="14" width="7" height="7" />
      </>
    ),
    workflow: (
      <>
        <circle cx="6" cy="5" r="2.5" />
        <circle cx="18" cy="12" r="2.5" />
        <circle cx="6" cy="19" r="2.5" />
        <path d="M8.5 5h3a3 3 0 0 1 3 3v1.5M8.5 19h3a3 3 0 0 0 3-3v-1.5" />
      </>
    ),
    plugins: (
      <>
        <path d="M8 3v4M16 3v4M5 7h14v4a7 7 0 0 1-14 0V7Z" />
        <path d="M12 18v3" />
      </>
    ),
    runs: (
      <>
        <path d="M4 5h10M4 12h16M4 19h10" />
        <path d="m16 3 4 2-4 2M16 17l4 2-4 2" />
      </>
    ),
    folder: <path d="M3 6.5h7l2-2h9v14H3z" />,
    plus: <path d="M12 5v14M5 12h14" />,
    refresh: (
      <>
        <path d="M20 7v5h-5" />
        <path d="M19 12a7 7 0 1 0-2 5" />
      </>
    ),
    spark: (
      <>
        <path d="m12 2 1.5 6.5L20 10l-6.5 1.5L12 18l-1.5-6.5L4 10l6.5-1.5z" />
        <path d="m19 17 .6 2.4L22 20l-2.4.6L19 23l-.6-2.4L16 20l2.4-.6z" />
      </>
    ),
    check: <path d="m5 12 4 4L19 6" />,
    play: <path d="m8 5 11 7-11 7z" />,
    pulse: <path d="M3 12h4l2-6 4 12 2-6h6" />,
    bolt: <path d="m13 2-8 12h7l-1 8 8-12h-7z" />,
    stop: <rect x="6" y="6" width="12" height="12" />,
    close: <path d="M6 6l12 12M18 6 6 18" />,
    arrow: <path d="M5 12h14M14 7l5 5-5 5" />,
    down: <path d="M12 4v15M6 13l6 6 6-6" />,
    warning: (
      <>
        <path d="M12 3 2.5 20h19z" />
        <path d="M12 9v4M12 17h.01" />
      </>
    ),
  };
  return (
    <svg className="icon" viewBox="0 0 24 24" aria-hidden="true">
      {paths[name]}
    </svg>
  );
}

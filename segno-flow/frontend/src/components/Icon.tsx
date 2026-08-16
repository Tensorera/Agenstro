import type { SVGProps } from "react";

export type IconName =
  | "archive"
  | "calendar"
  | "check"
  | "chevron"
  | "clock"
  | "close"
  | "download"
  | "folder"
  | "history"
  | "import"
  | "layers"
  | "pause"
  | "play"
  | "search"
  | "spark"
  | "tray"
  | "warning";

const paths: Record<IconName, React.ReactNode> = {
  archive: <><rect x="3" y="4" width="18" height="5" rx="1"/><path d="M5 9v10a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V9M9 13h6"/></>,
  calendar: <><rect x="3" y="5" width="18" height="16" rx="2"/><path d="M16 3v4M8 3v4M3 10h18"/></>,
  check: <path d="m5 12 4 4L19 6"/>,
  chevron: <path d="m9 18 6-6-6-6"/>,
  clock: <><circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/></>,
  close: <path d="m7 7 10 10M17 7 7 17"/>,
  download: <><path d="M12 3v12m0 0 5-5m-5 5-5-5"/><path d="M5 19h14"/></>,
  folder: <path d="M3 7a2 2 0 0 1 2-2h5l2 2h7a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2Z"/>,
  history: <><path d="M3 12a9 9 0 1 0 3-6.7L3 8"/><path d="M3 3v5h5M12 7v5l4 2"/></>,
  import: <><path d="M12 16V4m0 0L7 9m5-5 5 5"/><path d="M5 20h14"/></>,
  layers: <><path d="m12 3 9 5-9 5-9-5Z"/><path d="m3 12 9 5 9-5M3 16l9 5 9-5"/></>,
  pause: <><path d="M9 5v14M15 5v14"/></>,
  play: <path d="m8 5 11 7-11 7Z"/>,
  search: <><circle cx="11" cy="11" r="7"/><path d="m20 20-4-4"/></>,
  spark: <><path d="m12 3 1.6 4.4L18 9l-4.4 1.6L12 15l-1.6-4.4L6 9l4.4-1.6Z"/><path d="m18 15 .8 2.2L21 18l-2.2.8L18 21l-.8-2.2L15 18l2.2-.8Z"/></>,
  tray: <><path d="M4 4h16v12H4z"/><path d="M8 20h8M12 16v4M8 10l4 3 4-3"/></>,
  warning: <><path d="M10.3 4.1 2.6 18a2 2 0 0 0 1.8 3h15.2a2 2 0 0 0 1.8-3L13.7 4.1a2 2 0 0 0-3.4 0Z"/><path d="M12 9v4M12 17h.01"/></>,
};

export function Icon({ name, ...props }: { name: IconName } & SVGProps<SVGSVGElement>) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
      {...props}
    >
      {paths[name]}
    </svg>
  );
}

import { useEffect, useId, useRef, useState } from "react";
import { Icon } from "./Icon";

interface ImportDialogProps {
  open: boolean;
  busy: boolean;
  onClose: () => void;
  onImport: (file: File) => void;
}

export function ImportDialog({ open, busy, onClose, onImport }: ImportDialogProps) {
  const [file, setFile] = useState<File | null>(null);
  const [dragging, setDragging] = useState(false);
  const inputId = useId();
  const dialogRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) {
      setFile(null);
      setDragging(false);
      return;
    }
    const previous = document.activeElement as HTMLElement | null;
    window.setTimeout(() => dialogRef.current?.focus(), 0);
    const keydown = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !busy) onClose();
    };
    window.addEventListener("keydown", keydown);
    return () => {
      window.removeEventListener("keydown", keydown);
      previous?.focus();
    };
  }, [busy, onClose, open]);

  if (!open) return null;

  const acceptFile = (candidate: File | undefined) => {
    if (candidate) setFile(candidate);
  };

  return (
    <div className="modal-backdrop" role="presentation" onMouseDown={(event) => {
      if (event.target === event.currentTarget && !busy) onClose();
    }}>
      <div
        ref={dialogRef}
        className="import-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="import-title"
        tabIndex={-1}
      >
        <div className="dialog-header">
          <span className="dialog-icon"><Icon name="import" /></span>
          <div>
            <p className="eyebrow">Register workflow</p>
            <h2 id="import-title">Import task package</h2>
          </div>
          <button type="button" className="icon-button" aria-label="Close import dialog" disabled={busy} onClick={onClose}>
            <Icon name="close" />
          </button>
        </div>
        <p className="dialog-intro">
          Choose a ZIP containing <code>segno-flow.json</code> and its pre-process, main, helper, and post-process scripts.
        </p>
        <label
          className={`drop-zone ${dragging ? "dragging" : ""} ${file ? "has-file" : ""}`}
          htmlFor={inputId}
          onDragEnter={(event) => { event.preventDefault(); setDragging(true); }}
          onDragOver={(event) => event.preventDefault()}
          onDragLeave={() => setDragging(false)}
          onDrop={(event) => {
            event.preventDefault();
            setDragging(false);
            acceptFile(event.dataTransfer.files[0]);
          }}
        >
          <input
            id={inputId}
            type="file"
            accept=".zip,application/zip,application/x-zip-compressed"
            onChange={(event) => acceptFile(event.target.files?.[0])}
          />
          <span className="drop-icon"><Icon name={file ? "check" : "archive"} /></span>
          {file ? (
            <>
              <strong>{file.name}</strong>
              <span>{(file.size / 1_048_576).toFixed(2)} MB · Ready to validate</span>
            </>
          ) : (
            <>
              <strong>Drop a task package here</strong>
              <span>or browse for a .zip file · maximum 64 MB</span>
            </>
          )}
        </label>
        <div className="compile-note">
          <Icon name="spark" />
          <span><strong>Safe import</strong> — schema, paths, cron expression, and Python syntax are checked before anything is registered.</span>
        </div>
        <div className="dialog-actions">
          <button type="button" className="secondary-button" disabled={busy} onClick={onClose}>Cancel</button>
          <button type="button" className="primary-button" disabled={!file || busy} onClick={() => file && onImport(file)}>
            <Icon name={busy ? "clock" : "import"} />
            {busy ? "Compiling…" : "Validate & import"}
          </button>
        </div>
      </div>
    </div>
  );
}

import { X } from "lucide-react";
import React, { useEffect, useRef } from "react";

export interface ConfirmDialogProps {
  title: string;
  message?: React.ReactNode;
  highlightText?: string;
  confirmText?: string;
  cancelText?: string;
  isDanger?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
  children?: React.ReactNode;
}

export function ConfirmDialog({
  title,
  message,
  highlightText,
  confirmText = "确定",
  cancelText = "取消",
  isDanger = false,
  onConfirm,
  onCancel,
  children,
}: ConfirmDialogProps) {
  const dialogRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        onCancel();
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [onCancel]);

  return (
    <div
      className="confirm-dialog-backdrop"
      role="presentation"
      onClick={(e) => {
        if (e.target === e.currentTarget) onCancel();
      }}
    >
      <div
        ref={dialogRef}
        className="confirm-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="confirm-dialog-title"
      >
        <button
          type="button"
          className="confirm-dialog-close"
          onClick={onCancel}
          aria-label="取消"
        >
          <X size={17} />
        </button>

        <h2 id="confirm-dialog-title">{title}</h2>

        {message || highlightText ? (
          <p className="confirm-dialog-desc">
            {message}
            {highlightText ? (
              <strong className="confirm-dialog-highlight" title={highlightText}>
                {highlightText}
              </strong>
            ) : null}
          </p>
        ) : null}

        {children ? (
          <div className="confirm-dialog-body">{children}</div>
        ) : (
          <div className="confirm-dialog-actions">
            <button
              type="button"
              className="secondary-button"
              onClick={onCancel}
            >
              {cancelText}
            </button>
            <button
              type="button"
              className={`primary-button ${isDanger ? "danger-button" : ""}`}
              onClick={onConfirm}
              autoFocus
            >
              {confirmText}
            </button>
          </div>
        )}
      </div>
    </div>
  );
}

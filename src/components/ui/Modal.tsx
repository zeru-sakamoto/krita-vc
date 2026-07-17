import { useEffect } from "react";

interface ModalProps {
  title: string;
  onClose: () => void;
  children: React.ReactNode;
  /** Action row pinned to the bottom (e.g. Cancel / Confirm buttons). */
  footer?: React.ReactNode;
  /** Panel width class. Defaults to the compact dialog width most modals want. */
  maxWidthClassName?: string;
}

/**
 * Minimal themed modal: backdrop + centered surface-2 panel. Closes on Esc or
 * backdrop click. Mirrors Menu's outside-click/Esc pattern — no new dependency.
 */
export function Modal({
  title,
  onClose,
  children,
  footer,
  maxWidthClassName = "max-w-md",
}: ModalProps) {
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  return (
    <div
      className="fixed inset-0 z-(--z-modal) grid place-items-center bg-black/50 p-4"
      onPointerDown={onClose}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label={title}
        onPointerDown={(e) => e.stopPropagation()}
        className={`flex w-full ${maxWidthClassName} max-h-[calc(100vh-2rem)] flex-col overflow-hidden rounded-panel border border-border bg-surface-2 shadow-(--shadow-float)`}
      >
        <h2 className="shrink-0 border-b border-border px-4 py-3 text-[14px] font-medium text-text">
          {title}
        </h2>
        <div className="min-h-0 flex-1 overflow-auto px-4 py-4">{children}</div>
        {footer && (
          <div className="flex shrink-0 justify-end gap-2 border-t border-border px-4 py-3">
            {footer}
          </div>
        )}
      </div>
    </div>
  );
}

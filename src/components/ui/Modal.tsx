import { useEffect } from "react";

interface ModalProps {
  title: string;
  onClose: () => void;
  children: React.ReactNode;
  /** Action row pinned to the bottom (e.g. Cancel / Confirm buttons). */
  footer?: React.ReactNode;
}

/**
 * Minimal themed modal: backdrop + centered surface-2 panel. Closes on Esc or
 * backdrop click. Mirrors Menu's outside-click/Esc pattern — no new dependency.
 */
export function Modal({ title, onClose, children, footer }: ModalProps) {
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
        className="w-full max-w-md overflow-hidden rounded-panel border border-border bg-surface-2 shadow-(--shadow-float)"
      >
        <h2 className="border-b border-border px-4 py-3 text-[14px] font-medium text-text">
          {title}
        </h2>
        <div className="px-4 py-4">{children}</div>
        {footer && (
          <div className="flex justify-end gap-2 border-t border-border px-4 py-3">{footer}</div>
        )}
      </div>
    </div>
  );
}

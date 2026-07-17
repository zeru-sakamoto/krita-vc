import { createContext, useCallback, useContext, useRef, useState } from "react";
import { CheckCircle, WarningCircle, X } from "@phosphor-icons/react";

type ToastVariant = "success" | "error";
interface ToastState {
  id: number;
  message: string;
  variant: ToastVariant;
}

const DURATION_MS = 5000;

const ToastContext = createContext<{
  show: (message: string, variant?: ToastVariant) => void;
} | null>(null);

/**
 * Single-slot global toast: a low-frequency manual action (e.g. backup) needs a brief
 * confirmation regardless of what's open, not a notification feed — a new show() simply
 * replaces whatever's showing.
 */
export function ToastProvider({ children }: { children: React.ReactNode }) {
  const [toast, setToast] = useState<ToastState | null>(null);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const show = useCallback((message: string, variant: ToastVariant = "success") => {
    if (timerRef.current) clearTimeout(timerRef.current);
    const id = Date.now();
    setToast({ id, message, variant });
    timerRef.current = setTimeout(() => setToast((t) => (t?.id === id ? null : t)), DURATION_MS);
  }, []);

  return (
    <ToastContext.Provider value={{ show }}>
      {children}
      {toast && (
        <div
          role="status"
          className="fixed bottom-4 right-4 z-(--z-toast) flex max-w-sm items-start gap-2 rounded-panel border border-border bg-surface-2 px-3 py-2.5 text-[13px] text-text shadow-(--shadow-float)"
        >
          {toast.variant === "error" ? (
            <WarningCircle size={16} className="mt-0.5 shrink-0 text-danger" />
          ) : (
            <CheckCircle size={16} className="mt-0.5 shrink-0 text-success-fg" />
          )}
          <span className="min-w-0 flex-1 break-words">{toast.message}</span>
          <button
            type="button"
            aria-label="Dismiss"
            onClick={() => setToast(null)}
            className="shrink-0 text-text-muted hover:text-text"
          >
            <X size={14} />
          </button>
        </div>
      )}
    </ToastContext.Provider>
  );
}

export function useToast() {
  const ctx = useContext(ToastContext);
  if (!ctx) throw new Error("useToast must be used within ToastProvider");
  return ctx;
}

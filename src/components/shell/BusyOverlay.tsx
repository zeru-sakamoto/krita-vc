import { CircleNotch } from "@phosphor-icons/react";
import { useRepository } from "../../lib/repository";

/**
 * Full-screen, non-dismissible block shown while a write op (commit, branch
 * switch/merge/create/delete, rollback, undo, cleanup) is in flight — stops a
 * stray click from racing a file rewrite. Driven by `busyMessage` in
 * `RepositoryContext`; renders nothing when idle.
 */
export function BusyOverlay() {
  const { busyMessage } = useRepository();
  if (!busyMessage) return null;

  return (
    <div
      role="alert"
      aria-live="assertive"
      className="fixed inset-0 z-(--z-blocking) grid place-items-center bg-black/60"
    >
      <div className="flex flex-col items-center gap-3 text-text">
        <CircleNotch size={28} className="animate-spin text-accent" />
        <p className="text-[13px]">{busyMessage}</p>
      </div>
    </div>
  );
}

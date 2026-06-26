import type { Commit } from "../../types";
import { relativeTime } from "../../lib/format";
import { versionLabel } from "../../lib/friendly";
import { useArtistMode } from "../../lib/artistMode";

interface CommitCardProps {
  commit: Commit;
  /** Version number (newest = highest) — shown instead of the hash in Artist Mode. */
  version: number;
  selected: boolean;
  onSelect: (id: string) => void;
}

/**
 * Commit card for the history timeline.
 * (DESIGN.md → VCS Component Patterns → Commit Card)
 */
export function CommitCard({ commit, version, selected, onSelect }: CommitCardProps) {
  const { artistMode } = useArtistMode();
  return (
    <button
      type="button"
      onClick={() => onSelect(commit.id)}
      aria-pressed={selected}
      className={[
        "block w-full py-2.5 pl-1 pr-3 text-left",
        "transition-colors duration-100 ease-out",
        selected ? "bg-accent/12" : "hover:bg-white/5",
      ].join(" ")}
    >
      <div className="flex items-baseline justify-between gap-2">
        <span
          className={["text-[12px] text-text-muted", artistMode ? "font-medium" : "font-mono"].join(
            " "
          )}
        >
          {artistMode ? versionLabel(version) : commit.hash}
        </span>
        <span className="shrink-0 text-[11px] text-text-muted">
          {relativeTime(commit.timestamp)}
        </span>
      </div>
      <p className="mt-1 line-clamp-2 text-[13px] leading-snug text-text">{commit.message}</p>
      <div className="mt-1.5 flex items-center gap-1.5 text-[11px] text-text-muted">
        <span className="truncate">{commit.author}</span>
        <span aria-hidden>·</span>
        <span>
          {commit.changes.length} file{commit.changes.length === 1 ? "" : "s"}
        </span>
      </div>
    </button>
  );
}

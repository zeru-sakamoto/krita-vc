import {
  ArrowsLeftRight,
  PencilSimple,
  Plus,
  Sparkle,
  Trash,
  Warning,
  type Icon,
} from "@phosphor-icons/react";
import type { FileStatus } from "../../types";
import { useArtistMode } from "../../lib/artistMode";

const STATUS: Record<FileStatus, { color: string; label: string; icon: Icon }> = {
  M: { color: "text-warning-fg", label: "Modified", icon: PencilSimple },
  A: { color: "text-success-fg", label: "Added", icon: Plus },
  D: { color: "text-danger", label: "Deleted", icon: Trash },
  U: { color: "text-text-muted", label: "Untracked", icon: Sparkle },
  R: { color: "text-info-fg", label: "Renamed", icon: ArrowsLeftRight },
  C: { color: "text-accent", label: "Conflicted", icon: Warning },
};

/**
 * Change indicator. In Artist Mode it shows an icon + word; otherwise the
 * single-letter code (M/A/D/...). The letter/word carries the meaning, so color
 * is never the sole signal (a11y rule, DESIGN.md).
 * (DESIGN.md → VCS Component Patterns → File Status Chip)
 */
export function FileStatusChip({ status }: { status: FileStatus }) {
  const { artistMode } = useArtistMode();
  const { color, label, icon: Icon } = STATUS[status];

  if (artistMode) {
    return (
      <span
        title={label}
        className={[
          "inline-flex items-center gap-1 text-[11px] font-medium leading-none",
          color,
        ].join(" ")}
      >
        <Icon size={12} weight="bold" />
        {label}
      </span>
    );
  }

  return (
    <span
      title={label}
      className={["font-mono text-[11px] font-medium leading-none", color].join(" ")}
    >
      {status}
    </span>
  );
}

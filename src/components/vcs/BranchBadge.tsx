import { GitBranch } from "@phosphor-icons/react";
import type { Branch } from "../../types";

const KIND_STYLES: Record<Branch["kind"], string> = {
  current: "text-accent",
  local: "text-text",
};

/**
 * Branch badge — pill, mono 11px, colored by kind.
 * (DESIGN.md → VCS Component Patterns → Branch Badge)
 */
export function BranchBadge({ branch }: { branch: Branch }) {
  return (
    <span
      className={[
        "inline-flex max-w-full min-w-0 items-center gap-1 rounded-panel bg-surface-3 px-1.5 py-0.5",
        "font-mono text-[11px] leading-none",
        KIND_STYLES[branch.kind],
      ].join(" ")}
      title={branch.name}
    >
      <GitBranch size={11} weight="regular" className="shrink-0" />
      <span className="truncate">{branch.name}</span>
    </span>
  );
}

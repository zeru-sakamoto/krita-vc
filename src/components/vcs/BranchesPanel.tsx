import { Check, GitMerge, Plus, Trash } from "@phosphor-icons/react";
import type { Branch } from "../../types";
import { BranchBadge } from "./BranchBadge";
import { IconButton } from "../ui/IconButton";
import { useBranchActions } from "./useBranchActions";
import { useArtistMode } from "../../lib/artistMode";

/**
 * Local branch list with real actions: click to switch, hover a row to merge it into the
 * current branch or delete its label. This is a local-only VCS — there are no remotes.
 *
 * The actions themselves — and every dialog they raise — come from `useBranchActions`, shared
 * with the Version Map's action bar and the History sidebar's branch switcher.
 */
export function BranchesPanel({
  branches,
  onShowChanges,
}: {
  branches: Branch[];
  onShowChanges?: () => void;
}) {
  const { artistMode } = useArtistMode();
  const current = branches.find((b) => b.kind === "current")?.name ?? "main";
  const actions = useBranchActions({ currentBranch: current, onShowChanges });
  const { saving } = actions;

  return (
    <div className="flex flex-col">
      <div>
        <h3 className="flex h-8 shrink-0 items-center px-3 text-[11px] font-medium uppercase tracking-wide text-text-muted">
          Local
        </h3>
        <ul className="flex flex-col">
          {branches.map((b) => {
            const active = b.kind === "current";
            return (
              <li key={b.name} className="group relative">
                <button
                  type="button"
                  onClick={() => actions.switchTo(b.name)}
                  disabled={saving}
                  title={
                    active
                      ? artistMode
                        ? "You're working here"
                        : "Current branch"
                      : `Switch to ${b.name}`
                  }
                  className={[
                    "flex w-full items-center gap-2 px-3 py-1.5 text-left transition-colors",
                    active ? "row-selected" : "hover:bg-state-hover",
                  ].join(" ")}
                >
                  <BranchBadge branch={b} />
                  {active && <Check size={13} className="ml-auto text-accent" />}
                </button>
                {!active && (
                  <span className="absolute right-2 top-1/2 flex -translate-y-1/2 items-center gap-0.5 opacity-0 transition-opacity focus-within:opacity-100 group-hover:opacity-100">
                    <IconButton
                      icon={GitMerge}
                      label={
                        artistMode ? `Bring ${b.name} into ${current}` : `Merge into ${current}`
                      }
                      size={14}
                      disabled={saving || !b.tip}
                      onClick={() => actions.askMerge(b.name)}
                    />
                    {b.name !== "main" && (
                      <IconButton
                        icon={Trash}
                        label={artistMode ? "Remove this version line" : "Delete branch"}
                        size={14}
                        disabled={saving}
                        onClick={() => actions.askDelete(b.name)}
                      />
                    )}
                  </span>
                )}
              </li>
            );
          })}
        </ul>
      </div>

      <button
        type="button"
        onClick={() => actions.askCreate()}
        disabled={saving}
        data-tour-id="branches-new"
        className="mx-3 mt-2 flex items-center gap-1.5 rounded-button px-1 py-1 text-[12px] text-text-muted transition-colors hover:bg-state-hover hover:text-text"
      >
        <Plus size={13} />
        {artistMode ? "New version line" : "New branch"}
      </button>

      {actions.error && <p className="px-3 pt-2 text-[12px] text-danger">{actions.error}</p>}

      {actions.dialogs}
    </div>
  );
}

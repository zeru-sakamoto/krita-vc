import { X } from "@phosphor-icons/react";
import { IconButton } from "../ui/IconButton";
import { FileStatusChip } from "../vcs/FileStatusChip";
import type { Commit } from "../../types";
import { fullTimestamp } from "../../lib/format";
import { assetName, versionLabel } from "../../lib/friendly";
import { useArtistMode } from "../../lib/artistMode";

interface InspectorProps {
  commit: Commit | null;
  /** Version number for the selected commit (used in Artist Mode). */
  version: number;
  onClose: () => void;
}

function MetaRow({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="grid grid-cols-[64px_1fr] items-baseline gap-2 px-3 py-1.5">
      <span className="text-[11px] font-medium uppercase text-text-muted">{label}</span>
      <span className="selectable text-[13px] text-text">{children}</span>
    </div>
  );
}

/**
 * 280px toggleable inspector showing the selected commit's metadata.
 * (DESIGN.md → Layout & App Shell → Inspector panel)
 */
export function Inspector({ commit, version, onClose }: InspectorProps) {
  const { artistMode } = useArtistMode();
  return (
    <div className="flex h-full w-70 shrink-0 flex-col border-l border-border">
      {/* Single header row — py-1.5, aligns with the "Modified Hero" file header */}
      <div className="flex shrink-0 items-center border-b border-border bg-surface-2 px-3 py-2 h-8">
        <span className="flex-1 text-[11px] font-medium uppercase tracking-wide text-text-muted">
          {artistMode ? "Version" : "Commit"}
        </span>
        <IconButton icon={X} label="Close inspector" size={16} onClick={onClose} />
      </div>
      {/* Scrollable content */}
      <div className="min-h-0 flex-1 overflow-auto bg-surface">
        {!commit ? (
          <div className="grid h-full place-items-center px-6 text-center text-[12px] text-text-muted">
            Select a commit to inspect its details.
          </div>
        ) : (
          <div className="flex flex-col">
            <div className="border-b border-border py-1">
              {artistMode ? (
                <MetaRow label="Version">{versionLabel(version)}</MetaRow>
              ) : (
                <MetaRow label="Hash">
                  <span className="font-mono text-[12px]">{commit.hash}</span>
                </MetaRow>
              )}
              <MetaRow label="Author">{commit.author}</MetaRow>
              <MetaRow label="Date">
                <span className="text-[12px] text-text-muted">
                  {fullTimestamp(commit.timestamp)}
                </span>
              </MetaRow>
            </div>

            <div className="border-b border-border px-3 py-2.5">
              <p className="selectable text-[13px] leading-relaxed text-text">{commit.message}</p>
            </div>

            <div className="px-3 py-2">
              <h3 className="mb-1.5 text-[11px] font-medium uppercase text-text-muted">
                Changed files ({commit.changes.length})
              </h3>
              <ul className="flex flex-col">
                {commit.changes.map((c) => (
                  <li
                    key={c.path}
                    className="flex items-center gap-2 rounded-button px-1 py-1 hover:bg-white/5"
                  >
                    <FileStatusChip status={c.status} />
                    <span
                      className={[
                        "selectable truncate text-[12px] text-text",
                        artistMode ? "" : "font-mono",
                      ].join(" ")}
                    >
                      {artistMode ? assetName(c.path) : c.path}
                    </span>
                  </li>
                ))}
              </ul>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

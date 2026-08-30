import { useState } from "react";
import {
  ClockCounterClockwise,
  GitBranch,
  Stack,
  GearSix,
  Gauge,
  FileZip,
  MapTrifold,
} from "@phosphor-icons/react";
import { IconButton } from "../ui/IconButton";
import { ICON } from "../../lib/iconSize";
import { SettingsModal } from "./SettingsModal";
import { BackupModal } from "./BackupModal";
import { useRepository } from "../../lib/repository";
import { useLegacyHistory } from "../../lib/legacyHistory";

export type ActivityView = "changes" | "map" | "history" | "branches" | "performance";

interface ActivityBarProps {
  active: ActivityView;
  onChange: (view: ActivityView) => void;
}

const ITEMS: { view: ActivityView; icon: typeof Stack; label: string; legacy?: boolean }[] = [
  { view: "changes", icon: Stack, label: "Changes" },
  { view: "map", icon: MapTrifold, label: "Version map" },
  // Superseded by the Version Map — shown only when "Legacy version history" is on.
  { view: "history", icon: ClockCounterClockwise, label: "History", legacy: true },
  { view: "branches", icon: GitBranch, label: "Branches", legacy: true },
  { view: "performance", icon: Gauge, label: "Performance" },
];

/**
 * 48px fixed icon-only vertical strip, leftmost zone.
 * (DESIGN.md → Layout & App Shell → Activity bar)
 */
export function ActivityBar({ active, onChange }: ActivityBarProps) {
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [backupOpen, setBackupOpen] = useState(false);
  const { repositories, busyMessage } = useRepository();
  const { legacy } = useLegacyHistory();

  return (
    // py-2 matches the well's padding, so the first chip's top edge and the last
    // chip's bottom edge land exactly on the card edges across the gutter.
    <nav className="flex w-12 shrink-0 flex-col items-center bg-surface py-2">
      <div className="flex flex-col items-center gap-2">
        {ITEMS.filter((i) => legacy || !i.legacy).map(({ view, icon, label }) => (
          <IconButton
            key={view}
            icon={icon}
            label={label}
            size={ICON.toolbar}
            active={active === view}
            onClick={() => onChange(view)}
            tourId={view}
          />
        ))}
      </div>
      <div className="mt-auto flex flex-col items-center gap-2">
        <IconButton
          icon={FileZip}
          label="Back up artworks…"
          size={ICON.toolbar}
          disabled={repositories.length === 0 || !!busyMessage}
          onClick={() => setBackupOpen(true)}
          tourId="backup"
        />
        <IconButton
          icon={GearSix}
          label="Settings"
          size={ICON.toolbar}
          onClick={() => setSettingsOpen(true)}
          tourId="settings"
        />
      </div>
      {backupOpen && <BackupModal onClose={() => setBackupOpen(false)} />}
      {settingsOpen && <SettingsModal onClose={() => setSettingsOpen(false)} />}
    </nav>
  );
}

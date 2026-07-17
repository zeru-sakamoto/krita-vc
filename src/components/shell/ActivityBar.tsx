import { useState } from "react";
import {
  ClockCounterClockwise,
  GitBranch,
  Stack,
  GearSix,
  Gauge,
  FileZip,
} from "@phosphor-icons/react";
import { IconButton } from "../ui/IconButton";
import { SettingsModal } from "./SettingsModal";
import { useRepository } from "../../lib/repository";
import { useToast } from "../../lib/toast";

export type ActivityView = "changes" | "history" | "branches" | "performance";

interface ActivityBarProps {
  active: ActivityView;
  onChange: (view: ActivityView) => void;
}

const ITEMS: { view: ActivityView; icon: typeof Stack; label: string }[] = [
  { view: "changes", icon: Stack, label: "Changes" },
  { view: "history", icon: ClockCounterClockwise, label: "History" },
  { view: "branches", icon: GitBranch, label: "Branches" },
  { view: "performance", icon: Gauge, label: "Performance" },
];

/**
 * 48px fixed icon-only vertical strip, leftmost zone.
 * (DESIGN.md → Layout & App Shell → Activity bar)
 */
export function ActivityBar({ active, onChange }: ActivityBarProps) {
  const [settingsOpen, setSettingsOpen] = useState(false);
  const { current, backupRepository, busyMessage } = useRepository();
  const { show } = useToast();

  const doBackup = async () => {
    if (!current) return;
    try {
      const dest = await backupRepository(current.id);
      if (dest) show(`Backed up to ${dest}`);
    } catch (e) {
      show(String(e), "error");
    }
  };

  return (
    <nav className="flex w-12 shrink-0 flex-col items-center border-r border-border bg-surface py-1.5">
      <div className="flex flex-col items-center gap-0.5">
        {ITEMS.map(({ view, icon, label }) => (
          <IconButton
            key={view}
            icon={icon}
            label={label}
            size={24}
            active={active === view}
            onClick={() => onChange(view)}
          />
        ))}
      </div>
      <div className="mt-auto flex flex-col items-center gap-0.5">
        <IconButton
          icon={FileZip}
          label="Back up this repository…"
          size={24}
          disabled={!current || !!busyMessage}
          onClick={doBackup}
        />
        <IconButton
          icon={GearSix}
          label="Settings"
          size={24}
          onClick={() => setSettingsOpen(true)}
        />
      </div>
      {settingsOpen && <SettingsModal onClose={() => setSettingsOpen(false)} />}
    </nav>
  );
}

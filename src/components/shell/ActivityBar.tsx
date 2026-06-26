import { ClockCounterClockwise, GitBranch, Stack, GearSix } from "@phosphor-icons/react";
import { IconButton } from "../ui/IconButton";

export type ActivityView = "changes" | "history" | "branches";

interface ActivityBarProps {
  active: ActivityView;
  onChange: (view: ActivityView) => void;
}

const ITEMS: { view: ActivityView; icon: typeof Stack; label: string }[] = [
  { view: "changes", icon: Stack, label: "Changes" },
  { view: "history", icon: ClockCounterClockwise, label: "History" },
  { view: "branches", icon: GitBranch, label: "Branches" },
];

/**
 * 48px fixed icon-only vertical strip, leftmost zone.
 * (DESIGN.md → Layout & App Shell → Activity bar)
 */
export function ActivityBar({ active, onChange }: ActivityBarProps) {
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
        <IconButton icon={GearSix} label="Settings (mock)" size={24} />
      </div>
    </nav>
  );
}

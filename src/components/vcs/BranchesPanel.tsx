import { useState } from "react";
import { Check } from "@phosphor-icons/react";
import type { Branch } from "../../types";
import { BranchBadge } from "./BranchBadge";

function Group({
  title,
  branches,
  checkedOut,
  onCheckout,
}: {
  title: string;
  branches: Branch[];
  checkedOut: string;
  onCheckout: (name: string) => void;
}) {
  if (branches.length === 0) return null;
  return (
    <div>
      <h3 className="flex h-8 shrink-0 items-center px-3 text-[11px] font-medium uppercase tracking-wide text-text-muted">
        {title}
      </h3>
      <ul className="flex flex-col">
        {branches.map((b) => {
          const active = b.name === checkedOut;
          return (
            <li key={b.name}>
              <button
                type="button"
                onClick={() => onCheckout(b.name)}
                title={active ? "Current branch" : `Switch to ${b.name} (mock)`}
                className={[
                  "flex w-full items-center gap-2 border-l-2 px-3 py-1.5 text-left transition-colors",
                  active ? "border-accent bg-accent/12" : "border-transparent hover:bg-white/5",
                ].join(" ")}
              >
                <BranchBadge branch={b} />
                {active && <Check size={13} className="ml-auto text-accent" />}
              </button>
            </li>
          );
        })}
      </ul>
    </div>
  );
}

/**
 * Local branch list. Checkout is mock-only (sets a local "current" highlight) —
 * no backend branch switch. This is a local-only VCS — there are no remotes.
 */
export function BranchesPanel({ branches }: { branches: Branch[] }) {
  const initial = branches.find((b) => b.kind === "current")?.name ?? branches[0]?.name ?? "";
  const [checkedOut, setCheckedOut] = useState(initial);

  return (
    <div className="flex flex-col">
      <Group title="Local" branches={branches} checkedOut={checkedOut} onCheckout={setCheckedOut} />
    </div>
  );
}

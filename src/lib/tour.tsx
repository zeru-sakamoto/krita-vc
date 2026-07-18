import { createContext, useCallback, useContext, useState } from "react";
import type { ActivityView } from "../components/shell/ActivityBar";

/**
 * First-launch product tour: a linear, one-time spotlight walkthrough of the
 * main shell, fired once via `beginIfFirstTime` and never again automatically.
 * Same localStorage-flag pattern as `artistMode.tsx`/`windowChrome.tsx`.
 */

const STORAGE_KEY = "krita-vc:tour-completed";

export interface TourStep {
  tourId: string;
  title: string;
  body: string;
  /** If set, the tour switches the Sidebar to this view while the step is active. */
  view?: ActivityView;
}

export const TOUR_STEPS: TourStep[] = [
  {
    tourId: "repo-switcher",
    title: "Your repositories",
    body: "Switch between local repositories here, or create/open another one.",
  },
  {
    tourId: "changes",
    title: "Changes",
    body: "See what's changed in your working files since the last saved version.",
    view: "changes",
  },
  {
    tourId: "refresh",
    title: "Rescan for changes",
    body: "If a change you made doesn't show up, click here to check the folder again.",
    view: "changes",
  },
  {
    tourId: "changes-branch",
    title: "Your current branch",
    body: "Every version you commit here is saved onto this branch.",
    view: "changes",
  },
  {
    tourId: "changes-staged",
    title: "Staged files",
    body: "Files you move here are what get included when you commit a new version.",
    view: "changes",
  },
  {
    tourId: "changes-unstaged",
    title: "Changes",
    body: "Everything you've edited but haven't staged yet — hover a file to stage it or discard just that one.",
    view: "changes",
  },
  {
    tourId: "commit-message",
    title: "Describe this version",
    body: "Add a short note about what changed before committing.",
    view: "changes",
  },
  {
    tourId: "commit-button",
    title: "Commit version",
    body: "Saves a new version from your staged files — or everything, if nothing's staged.",
    view: "changes",
  },
  {
    tourId: "panel-options",
    title: "Panel options",
    body: "Undo your last save, discard current changes, or set work aside — all from this menu.",
    view: "changes",
  },
  {
    tourId: "panel-option-undo",
    title: "Undo",
    body: "Removes the most recent version — your changes come back as unsaved work, ready to re-save.",
    view: "changes",
  },
  {
    tourId: "panel-option-discard-all",
    title: "Discard current changes",
    body: "Reverts every changed file to its last saved version. Staged files aren't touched.",
    view: "changes",
  },
  {
    tourId: "panel-option-stash-staged",
    title: "Set aside staged files",
    body: "Parks just your staged files off to the side without committing them.",
    view: "changes",
  },
  {
    tourId: "panel-option-stash-all",
    title: "Set aside everything",
    body: "Parks all your current changes off to the side, clearing your working files for now.",
    view: "changes",
  },
  {
    tourId: "panel-option-stash-pop-latest",
    title: "Bring back latest",
    body: "Brings back the most recent work you set aside.",
    view: "changes",
  },
  {
    tourId: "panel-option-stash-pick",
    title: "Bring back…",
    body: "Pick which set-aside work to bring back, if you've set aside more than once.",
    view: "changes",
  },
  {
    tourId: "history",
    title: "History",
    body: "Browse every saved version as a graph, and open any one to see its diff.",
    view: "history",
  },
  {
    tourId: "history-branch",
    title: "Switch branch",
    body: "Pick a different branch to see its own line of versions.",
    view: "history",
  },
  {
    tourId: "history-versions",
    title: "Versions",
    body: "Each dot is a saved version — click one to see what changed.",
    view: "history",
  },
  {
    tourId: "panel-options",
    title: "Panel options",
    body: "Undo your last save from here.",
    view: "history",
  },
  {
    tourId: "panel-option-undo",
    title: "Undo",
    body: "Works the same as in Changes — removes the most recent version, ready to re-save.",
    view: "history",
  },
  {
    tourId: "branches",
    title: "Branches",
    body: "Create, switch, and merge branches to work on variations side by side.",
    view: "branches",
  },
  {
    tourId: "branches-new",
    title: "Start a new branch",
    body: "Click here to branch off and try something without touching your main work — hover any branch below to merge or delete it.",
    view: "branches",
  },
  {
    tourId: "performance",
    title: "Performance",
    body: "See how fast saves and switches run, and how much space your version history is actually using.",
    view: "performance",
  },
  {
    tourId: "performance-stats",
    title: "Storage & speed",
    body: "This shows how much space you're saving versus a full copy of every version, plus average time for each operation.",
    view: "performance",
  },
  {
    tourId: "inspector",
    title: "Inspector",
    body: "Show or hide details about the selected version — message, author, and files changed.",
  },
  {
    tourId: "settings",
    title: "Settings",
    body: "Appearance, the set-aside shelf, and storage preferences all live here.",
  },
  {
    tourId: "backup",
    title: "Back up",
    body: "One click zips the whole repository as a safety copy.",
  },
];

function hasCompleted(): boolean {
  if (typeof localStorage === "undefined") return true;
  return localStorage.getItem(STORAGE_KEY) === "true";
}

function markCompleted() {
  try {
    localStorage.setItem(STORAGE_KEY, "true");
  } catch {
    // ignore (e.g. private mode) — tour just won't stay dismissed next session
  }
}

interface TourValue {
  active: boolean;
  step: TourStep;
  stepIndex: number;
  totalSteps: number;
  next: () => void;
  back: () => void;
  skip: () => void;
  restart: () => void;
  beginIfFirstTime: () => void;
}

const TourContext = createContext<TourValue | null>(null);

export function TourProvider({ children }: { children: React.ReactNode }) {
  const [stepIndex, setStepIndex] = useState<number | null>(null);
  const active = stepIndex !== null;

  const finish = useCallback(() => {
    markCompleted();
    setStepIndex(null);
  }, []);

  const next = useCallback(() => {
    setStepIndex((i) => {
      if (i === null) return i;
      if (i + 1 >= TOUR_STEPS.length) {
        markCompleted();
        return null;
      }
      return i + 1;
    });
  }, []);

  const back = useCallback(() => {
    setStepIndex((i) => (i !== null && i > 0 ? i - 1 : i));
  }, []);

  const restart = useCallback(() => setStepIndex(0), []);

  const beginIfFirstTime = useCallback(() => {
    if (!hasCompleted()) setStepIndex(0);
  }, []);

  const value: TourValue = {
    active,
    step: TOUR_STEPS[stepIndex ?? 0],
    stepIndex: stepIndex ?? 0,
    totalSteps: TOUR_STEPS.length,
    next,
    back,
    skip: finish,
    restart,
    beginIfFirstTime,
  };

  return <TourContext.Provider value={value}>{children}</TourContext.Provider>;
}

export function useTour(): TourValue {
  const ctx = useContext(TourContext);
  if (!ctx) throw new Error("useTour must be used within a TourProvider");
  return ctx;
}

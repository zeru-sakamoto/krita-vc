import { createContext, useCallback, useContext, useEffect, useMemo, useState } from "react";
import type { ActivityView } from "../components/shell/ActivityBar";

/**
 * First-launch product tour: a linear, one-time spotlight walkthrough of the
 * main shell, fired once via `beginIfFirstTime` and never again automatically.
 * Same localStorage-flag pattern as `artistMode.tsx`/`windowChrome.tsx`.
 *
 * Steps are **conditionally visible**: the Version Map is the default view and the legacy
 * History/Branches tabs are off by default, so a flat unconditional list spotlit targets that
 * aren't in the DOM — and a missing target makes `TourOverlay` render nothing at all (no card, no
 * Next, no Skip). A step's optional `when` predicate is evaluated *live* against conditions
 * reported by `RepoShell`, not snapshotted when the tour starts: `beginIfFirstTime` fires from a
 * mount effect while `useCommits` is still in flight, so a snapshot would drop every map step on
 * a repo that does have history.
 */

const STORAGE_KEY = "krita-vc:tour-completed";

/** What the shell knows that decides whether a step has anything to point at. */
export interface TourConditions {
  /** Settings → Appearance → "Legacy version history" — the History/Branches tabs exist. */
  legacy: boolean;
  /** At least one saved version, so the map draws nodes rather than its empty state. */
  hasVersions: boolean;
  /** More than one branch, so the map's "All lines" toggle is rendered. */
  hasOtherBranches: boolean;
  /** Working tree is dirty, so the map draws its pending-version preview node. */
  dirty: boolean;
}

const NO_CONDITIONS: TourConditions = {
  legacy: false,
  hasVersions: false,
  hasOtherBranches: false,
  dirty: false,
};

export interface TourStep {
  tourId: string;
  title: string;
  body: string;
  /** If set, the tour switches the Sidebar to this view while the step is active. */
  view?: ActivityView;
  /** Skip this step unless the shell is in a state where its target actually exists. A plain
   *  predicate rather than a key registry, so negation and compound gates need no syntax. */
  when?: (c: TourConditions) => boolean;
}

const legacyOnly = (c: TourConditions) => c.legacy;
const withVersions = (c: TourConditions) => c.hasVersions;

export const TOUR_STEPS: TourStep[] = [
  {
    tourId: "repo-switcher",
    title: "Your artworks",
    body: "Switch between the artworks you're tracking, or track/restore another one — this opens a searchable list.",
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
    body: "If a change you made doesn't show up, click here to check the artwork again.",
    view: "changes",
  },
  {
    tourId: "changes-branch",
    title: "Your current branch",
    body: "Every version you commit here is saved onto this branch.",
    view: "changes",
  },
  {
    tourId: "changes-unstaged",
    title: "What changed",
    body: "The layers you've painted, added or removed since your last saved version.",
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
    title: "Save this version",
    body: "Records the artwork as it is right now, so you can always come back to it.",
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
    body: "Reverts this artwork to its latest saved version. Anything painted since is lost.",
    view: "changes",
  },
  {
    tourId: "panel-option-stash-all",
    title: "Set this aside",
    body: "Parks your current changes off to the side without saving a version, so you can come back to them later.",
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

  // The Version Map — the default view. Ordered here to match the activity bar itself
  // (changes → map → history → …).
  {
    tourId: "map",
    title: "Version map",
    body: "Every version you've saved, laid out oldest to newest along a line. This is the main view — drag to move around it, scroll to zoom.",
    view: "map",
  },
  {
    tourId: "map-empty",
    title: "Nothing here yet",
    body: "Once you save your first version from the Changes tab it appears here, and every version after it joins the line.",
    view: "map",
    when: (c) => !c.hasVersions,
  },
  {
    tourId: "map-version",
    title: "One saved version",
    body: "Each card is a version: how the artwork looked, its number and your note, and a chip for every layer that changed. A ring around the artwork marks the newest one. Click a card to open the full before/after comparison, with all the details beside it.",
    view: "map",
    when: withVersions,
  },
  {
    tourId: "map-preview",
    title: "Your unsaved work",
    body: "The dashed card is where your current changes would land if you saved them. Click it to jump to Changes and save.",
    view: "map",
    when: (c) => c.dirty,
  },
  {
    tourId: "map-branch",
    title: "Which line you're on",
    body: "Every version line you've started is listed here — click one to switch to it. Hover a line to merge it into this one, or to remove it.",
    view: "map",
    when: withVersions,
  },
  {
    tourId: "map-branch-new",
    title: "Start a new line",
    body: "Branches off from where you are now, so you can try something without touching your main work.",
    view: "map",
    when: withVersions,
  },
  {
    tourId: "map-pick",
    title: "Go back and try again",
    body: "Pick any earlier version on the map and start a new line from there — the original stays exactly as it is.",
    view: "map",
    when: withVersions,
  },
  {
    tourId: "map-all-lines",
    title: "Show every line",
    body: "Draws all your version lines at once, each on its own row, so you can see where they split and joined.",
    view: "map",
    when: (c) => c.hasVersions && c.hasOtherBranches,
  },
  {
    tourId: "map-view-controls",
    title: "Getting around",
    body: "The percentage is your zoom — click it to snap back to 100%. Next to it: fit every version on screen, and jump straight to the newest one.",
    view: "map",
    when: withVersions,
  },
  {
    tourId: "map-minimap",
    title: "Overview",
    body: "The whole history in miniature. Drag inside it to move around a long line without zooming out.",
    view: "map",
    when: withVersions,
  },

  {
    tourId: "history",
    title: "History",
    body: "Browse every saved version as a graph, and open any one to see its diff.",
    view: "history",
    when: legacyOnly,
  },
  {
    tourId: "history-branch",
    title: "Switch branch",
    body: "Pick a different branch to see its own line of versions.",
    view: "history",
    when: legacyOnly,
  },
  {
    tourId: "history-versions",
    title: "Versions",
    body: "Each dot is a saved version — click one to see what changed.",
    view: "history",
    when: legacyOnly,
  },
  {
    tourId: "panel-options",
    title: "Panel options",
    body: "Undo your last save from here.",
    view: "history",
    when: legacyOnly,
  },
  {
    tourId: "panel-option-undo",
    title: "Undo",
    body: "Works the same as in Changes — removes the most recent version, ready to re-save.",
    view: "history",
    when: legacyOnly,
  },
  {
    tourId: "branches",
    title: "Branches",
    body: "Create, switch, and merge branches to work on variations side by side.",
    view: "branches",
    when: legacyOnly,
  },
  {
    tourId: "branches-new",
    title: "Start a new branch",
    body: "Click here to branch off and try something without touching your main work — hover any branch below to merge or delete it.",
    view: "branches",
    when: legacyOnly,
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
    when: legacyOnly,
  },
  {
    tourId: "settings",
    title: "Settings",
    body: "Appearance, the set-aside shelf, and storage preferences all live here.",
  },
  {
    tourId: "backup",
    title: "Back up",
    body: "Pick any of your artworks and zip them — art and full history — into one safety copy. Restoring one is in the artwork switcher.",
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
  /** Position among the *visible* steps, so the readout matches what the user will actually see. */
  stepIndex: number;
  totalSteps: number;
  next: () => void;
  back: () => void;
  skip: () => void;
  restart: () => void;
  beginIfFirstTime: () => void;
  /** Reported by `RepoShell` — decides which steps are visible. */
  setConditions: (c: TourConditions) => void;
}

const TourContext = createContext<TourValue | null>(null);

export function TourProvider({ children }: { children: React.ReactNode }) {
  // The cursor is an index into TOUR_STEPS, not into the filtered list: conditions change while
  // the tour runs (history finishes loading, a stash empties the tree), and an index into a list
  // that resizes underneath you silently jumps to a different step.
  const [stepIndex, setStepIndex] = useState<number | null>(null);
  const [conditions, setConditionsState] = useState<TourConditions>(NO_CONDITIONS);
  const active = stepIndex !== null;

  const visible = useCallback(
    (i: number) => {
      const s = TOUR_STEPS[i];
      return !!s && (!s.when || s.when(conditions));
    },
    [conditions]
  );

  const visibleSteps = useMemo(
    () => TOUR_STEPS.filter((s) => !s.when || s.when(conditions)),
    [conditions]
  );

  const setConditions = useCallback((c: TourConditions) => {
    // Compared by value, not identity: the caller builds a fresh object every render, and storing
    // it unconditionally would re-render the whole tree on every AppShell render.
    setConditionsState((prev) =>
      prev.legacy === c.legacy &&
      prev.hasVersions === c.hasVersions &&
      prev.hasOtherBranches === c.hasOtherBranches &&
      prev.dirty === c.dirty
        ? prev
        : c
    );
  }, []);

  const finish = useCallback(() => {
    markCompleted();
    setStepIndex(null);
  }, []);

  const next = useCallback(() => {
    setStepIndex((i) => {
      if (i === null) return i;
      for (let n = i + 1; n < TOUR_STEPS.length; n++) if (visible(n)) return n;
      markCompleted();
      return null;
    });
  }, [visible]);

  const back = useCallback(() => {
    setStepIndex((i) => {
      if (i === null) return i;
      for (let p = i - 1; p >= 0; p--) if (visible(p)) return p;
      return i;
    });
  }, [visible]);

  const firstVisible = useCallback(() => {
    for (let i = 0; i < TOUR_STEPS.length; i++) if (visible(i)) return i;
    return null;
  }, [visible]);

  const restart = useCallback(() => setStepIndex(firstVisible()), [firstVisible]);

  const beginIfFirstTime = useCallback(() => {
    if (!hasCompleted()) setStepIndex(firstVisible());
  }, [firstVisible]);

  // Conditions can flip while a step is on screen (history finishes loading, the tree goes clean).
  // Nudge forward off a step that just stopped applying rather than spotlighting a gone element.
  useEffect(() => {
    if (stepIndex === null || visible(stepIndex)) return;
    next();
  }, [stepIndex, visible, next]);

  const step = TOUR_STEPS[stepIndex ?? 0];
  const value: TourValue = {
    active,
    step,
    stepIndex: Math.max(0, visibleSteps.indexOf(step)),
    totalSteps: visibleSteps.length,
    next,
    back,
    skip: finish,
    restart,
    beginIfFirstTime,
    setConditions,
  };

  return <TourContext.Provider value={value}>{children}</TourContext.Provider>;
}

export function useTour(): TourValue {
  const ctx = useContext(TourContext);
  if (!ctx) throw new Error("useTour must be used within a TourProvider");
  return ctx;
}

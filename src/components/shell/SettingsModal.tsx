import { useEffect, useRef, useState } from "react";
import {
  Archive,
  Broom,
  CaretDown,
  Cpu,
  FolderOpen,
  Gauge,
  HardDrive,
  ShieldCheck,
  Trash,
} from "@phosphor-icons/react";
import { open as pickFolder } from "@tauri-apps/plugin-dialog";
import { invoke } from "@tauri-apps/api/core";
import { Modal } from "../ui/Modal";
import { Button } from "../ui/Button";
import { Checkbox } from "../ui/Checkbox";
import { Radio } from "../ui/Radio";
import { IconButton } from "../ui/IconButton";
import { Menu, Select } from "../ui/Menu";
import { stashSummary, stashTitle } from "../vcs/StashDialogs";
import { useArtistMode } from "../../lib/artistMode";
import { useAuthorName } from "../../lib/authorName";
import { THEMES, useTheme, type ThemeId } from "../../lib/theme";
import { useRepository, type CheckReport, type CleanupReport } from "../../lib/repository";
import { hasBeenChecked } from "../../lib/checkedRepos";
import { useRepoConfig, useStashes } from "../../lib/repoData";
import { useTour } from "../../lib/tour";
import { inTauri } from "../../lib/tauri";
import { useWindowChrome } from "../../lib/windowChrome";
import { useLegacyHistory } from "../../lib/legacyHistory";
import { CPU_BUDGETS, useCpuBudget } from "../../lib/cpuBudget";
import type { Repository, Stash } from "../../types";

type SettingsCategory = "appearance" | "stash" | "performance" | "storage";

const CACHE_PRESETS_MB = [128, 256, 512, 1024, 2048];

function ToggleRow({
  icon,
  label,
  detail,
  active,
  onToggle,
}: {
  icon?: React.ReactNode;
  label: string;
  detail?: string;
  active: boolean;
  onToggle: () => void;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={active}
      onClick={onToggle}
      className="flex w-full flex-col gap-0.5 rounded-button py-1.5 text-left"
    >
      <span className="flex w-full items-start justify-between gap-3">
        <span className="flex min-w-0 items-center gap-2 text-[13px] text-text">
          {icon && <span className="shrink-0 text-text-muted">{icon}</span>}
          {label}
        </span>
        <span
          aria-hidden
          className={[
            "inset-well relative h-5 w-9 shrink-0 rounded-full transition-colors duration-200",
            active ? "bg-accent" : "bg-surface-3",
          ].join(" ")}
        >
          <span
            className={[
              "raised absolute left-0.5 top-1/2 size-4 -translate-y-1/2 rounded-full bg-text transition-transform duration-200 ease-out",
              active ? "translate-x-4" : "translate-x-0",
            ].join(" ")}
          />
        </span>
      </span>
      {detail && <span className="block text-[11px] text-text-muted">{detail}</span>}
    </button>
  );
}

/** Small swatch: theme background with its accent dot — used in the trigger and each option. */
function ThemeChip({ bg, accent }: { bg: string; accent: string }) {
  return (
    <span
      aria-hidden
      className="flex size-4 shrink-0 items-center justify-center rounded-button ring-1 ring-inset ring-border"
      style={{ backgroundColor: bg }}
    >
      <span className="size-1.5 rounded-full" style={{ backgroundColor: accent }} />
    </span>
  );
}

function NoRepoFallback() {
  return <p className="text-[12px] text-text-muted">Open an artwork to see these settings.</p>;
}

/**
 * Where new artworks keep their version history. App-global (the `kvc` CLI reads the same
 * setting, so it can't live in a per-artwork config), which is why it renders outside the
 * artwork gate — you must be able to change it before opening anything.
 *
 * Deliberately does **not** move existing histories: this decides where the *next* artwork's
 * store is created, and where artworks created under it are looked up. Saying so plainly beats
 * implying a migration that didn't happen.
 */
function StoreRootRow() {
  const [root, setRoot] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!inTauri()) return;
    invoke<string | null>("get_store_root")
      .then(setRoot)
      .catch(() => setRoot(null));
  }, []);

  const apply = async (next: string | null) => {
    setBusy(true);
    try {
      await invoke("set_store_root", { path: next });
      setRoot(next);
    } finally {
      setBusy(false);
    }
  };

  const choose = async () => {
    if (!inTauri()) return;
    const picked = await pickFolder({
      directory: true,
      title: "Choose where to keep version history",
    });
    if (typeof picked === "string") await apply(picked);
  };

  return (
    <div className="mb-3">
      <span className="mb-1 flex items-center gap-1.5 text-[12px] text-text-muted">
        <FolderOpen size={13} />
        Where version history is kept
      </span>
      <div className="flex items-center gap-2">
        <span className="min-w-0 flex-1 truncate inset-well rounded-button border border-border bg-bg px-2 py-1.5 font-mono text-[12px] text-text">
          {root ?? <span className="font-sans text-text-muted">Beside each artwork</span>}
        </span>
        <Button onClick={choose} disabled={busy || !inTauri()}>
          Choose…
        </Button>
        {root && (
          <Button onClick={() => apply(null)} disabled={busy}>
            Reset
          </Button>
        )}
      </div>
      <span className="mt-1 block text-[11px] text-text-muted">
        By default each artwork's history lives in a hidden folder beside it, so it travels with
        your files and stays on the same drive. Changing this only affects artworks you start
        tracking from now on — existing history isn't moved.
      </span>
    </div>
  );
}

function AppearanceSettings({
  artistMode,
  toggleArtistMode,
  customTitleBar,
  toggleWindowChrome,
  legacyHistory,
  toggleLegacyHistory,
  authorName,
  setAuthorName,
  theme,
  setTheme,
  onReplayTour,
}: {
  artistMode: boolean;
  toggleArtistMode: () => void;
  customTitleBar: boolean;
  toggleWindowChrome: () => void;
  legacyHistory: boolean;
  toggleLegacyHistory: () => void;
  authorName: string;
  setAuthorName: (name: string) => void;
  theme: ThemeId;
  setTheme: (id: ThemeId) => void;
  onReplayTour: () => void;
}) {
  return (
    <>
      <ToggleRow
        label="Artist view"
        detail="Plain-language labels and version numbers instead of the technical view."
        active={artistMode}
        onToggle={toggleArtistMode}
      />
      <ToggleRow
        label="Custom title bar"
        detail="Use krita-vc's own title bar instead of your operating system's native window frame."
        active={customTitleBar}
        onToggle={toggleWindowChrome}
      />
      <ToggleRow
        label="Legacy version history"
        detail="Bring back the old History and Branches tabs alongside the version map."
        active={legacyHistory}
        onToggle={toggleLegacyHistory}
      />
      <label className="mt-2 block">
        <span className="mb-1 block text-[12px] text-text-muted">Your name</span>
        <input
          value={authorName}
          onChange={(e) => setAuthorName(e.target.value)}
          placeholder="You"
          className="w-full inset-well rounded-button border border-border bg-bg px-2 py-1.5 text-[13px] text-text placeholder:text-text-muted focus:border-accent !outline-none"
        />
        <span className="mt-1 block text-[11px] text-text-muted">
          Shown as the author of new versions.
        </span>
      </label>
      <div className="mt-3">
        <span className="mb-1 block text-[12px] text-text-muted">Theme</span>
        <Menu
          minWidth={200}
          items={THEMES.map((t) => ({
            id: t.id,
            label: t.label,
            icon: <ThemeChip bg={t.bg} accent={t.accent} />,
            selected: t.id === theme,
            onSelect: () => setTheme(t.id),
          }))}
          trigger={(open) => {
            const cur = THEMES.find((t) => t.id === theme) ?? THEMES[0];
            return (
              <span
                className={[
                  "tactile flex min-w-50 items-center gap-2 rounded-button border bg-surface-3 px-2 py-1.5 text-[13px] text-text",
                  open ? "border-accent" : "border-border",
                ].join(" ")}
              >
                <ThemeChip bg={cur.bg} accent={cur.accent} />
                <span className="min-w-0 flex-1 truncate text-left">{cur.label}</span>
                <CaretDown size={12} weight="bold" className="shrink-0 text-text-muted" />
              </span>
            );
          }}
        />
      </div>
      <Button className="mt-3" onClick={onReplayTour}>
        Replay tour
      </Button>
    </>
  );
}

function StashSettings({
  stashes,
  onConfirmDrop,
  onConfirmDropAll,
}: {
  stashes: Stash[];
  onConfirmDrop: (s: Stash) => void;
  onConfirmDropAll: () => void;
}) {
  return (
    <StashShelf
      stashes={stashes}
      onConfirmDrop={onConfirmDrop}
      onConfirmDropAll={onConfirmDropAll}
    />
  );
}

/**
 * App-global, unlike everything else in the Performance tab — so it renders outside the
 * repo gate below and is labelled as applying everywhere.
 */
function CpuBudgetRow() {
  const { budget, setBudget } = useCpuBudget();
  const hint = CPU_BUDGETS.find((b) => b.percent === budget)?.hint ?? "";
  return (
    <div className="mb-3">
      <span className="mb-1 flex items-center gap-1.5 text-[12px] text-text-muted">
        <Cpu size={13} />
        Background CPU use
      </span>
      <Select
        value={budget}
        onChange={setBudget}
        options={CPU_BUDGETS.map((b) => ({ value: b.percent, label: b.label }))}
      />
      <span className="mt-1 block text-[11px] text-text-muted">
        {hint}. Applies to every repository.
      </span>
    </div>
  );
}

function PerformanceSettings({
  config,
  updateConfig,
}: {
  config: ReturnType<typeof useRepoConfig>["config"];
  updateConfig: ReturnType<typeof useRepoConfig>["update"];
}) {
  return (
    <ToggleRow
      icon={<Gauge size={15} />}
      label="Low-memory diffs"
      detail="Loads each layer of a working-file preview one at a time instead of all at
        once. Uses noticeably less memory on large files, in exchange for a little extra
        time to open a preview. Helpful on low-end machines."
      active={config?.lowMemoryDiff ?? false}
      onToggle={() => config && updateConfig({ ...config, lowMemoryDiff: !config.lowMemoryDiff })}
    />
  );
}

function StorageSettings({
  config,
  updateConfig,
  onShowCleanup,
  onShowCheck,
}: {
  config: ReturnType<typeof useRepoConfig>["config"];
  updateConfig: ReturnType<typeof useRepoConfig>["update"];
  onShowCleanup: () => void;
  onShowCheck: () => void;
}) {
  const { current } = useRepository();
  return (
    <>
      <div className="mb-3">
        <span className="mb-1 flex items-center gap-1.5 text-[12px] text-text-muted">
          <HardDrive size={13} />
          Preview cache size
        </span>
        <Select
          value={config ? Math.round(config.cacheMaxBytes / (1024 * 1024)) : CACHE_PRESETS_MB[0]}
          onChange={(mb) => config && updateConfig({ ...config, cacheMaxBytes: mb * 1024 * 1024 })}
          disabled={!config}
          options={CACHE_PRESETS_MB.map((mb) => ({
            value: mb,
            label: mb >= 1024 ? `${mb / 1024} GB` : `${mb} MB`,
          }))}
        />
        <span className="mt-1 block text-[11px] text-text-muted">
          How much space diff previews may use on disk. Oldest previews are cleared first once you
          go over — they regenerate automatically when needed again.
        </span>
      </div>

      <ToggleRow
        icon={<Archive size={15} />}
        label="Compact storage for heavily-revised art"
        detail="Shrinks version history for files with many small edits by 2–10x, at the
          cost of a little extra time on each save and restore. Safe to turn on or off at
          any point — past versions are unaffected either way."
        active={config?.tilePixelDeltas ?? false}
        onToggle={() =>
          config && updateConfig({ ...config, tilePixelDeltas: !config.tilePixelDeltas })
        }
      />

      <div className="mt-3 flex flex-wrap gap-2">
        <Button onClick={onShowCleanup}>
          <Broom size={14} />
          Clean up storage…
        </Button>
        <Button onClick={onShowCheck}>
          <ShieldCheck size={14} />
          Check for problems…
        </Button>
      </div>
      <span className="mt-2 block text-[11px] text-text-muted">
        {backupAgeLabel(current?.lastBackupAt)} There's no cloud sync — a backup is the only copy
        that survives your disk failing.
      </span>
    </>
  );
}

export function SettingsModal({ onClose }: { onClose: () => void }) {
  const { current, refreshNonce } = useRepository();
  const { artistMode, toggle: toggleArtistMode } = useArtistMode();
  const { customTitleBar, toggle: toggleWindowChrome } = useWindowChrome();
  const { legacy: legacyHistory, toggle: toggleLegacyHistory } = useLegacyHistory();
  const { authorName, setAuthorName } = useAuthorName();
  const { theme, setTheme } = useTheme();
  const { restart: restartTour } = useTour();
  const { config, update: updateConfig } = useRepoConfig(current?.path ?? "");
  const stashes = useStashes(current?.path ?? null, refreshNonce);
  const [showCleanup, setShowCleanup] = useState(false);
  const [showCheck, setShowCheck] = useState(false);
  const [showDropAll, setShowDropAll] = useState(false);
  const [confirmDrop, setConfirmDrop] = useState<Stash | null>(null);
  const [category, setCategory] = useState<SettingsCategory>("appearance");

  const categories: { id: SettingsCategory; label: string }[] = [
    { id: "appearance", label: "Appearance" },
    { id: "stash", label: artistMode ? "Set-Aside" : "Stashes" },
    { id: "performance", label: "Performance" },
    { id: "storage", label: "Storage" },
  ];

  return (
    <>
      <Modal
        title="Settings"
        onClose={onClose}
        footer={<Button onClick={onClose}>Done</Button>}
        maxWidthClassName="max-w-2xl"
      >
        <div className="flex gap-5">
          <nav className="flex w-32 shrink-0 flex-col gap-0.5">
            {categories.map((c) => (
              <button
                key={c.id}
                type="button"
                onClick={() => setCategory(c.id)}
                className={[
                  "rounded-button px-2.5 py-1.5 text-left text-[13px] transition-colors",
                  category === c.id
                    ? "row-selected text-text"
                    : "text-text-muted hover:bg-state-hover hover:text-text",
                ].join(" ")}
              >
                {c.label}
              </button>
            ))}
          </nav>
          <div className="min-w-0 flex-1">
            {category === "appearance" && (
              <AppearanceSettings
                artistMode={artistMode}
                toggleArtistMode={toggleArtistMode}
                customTitleBar={customTitleBar}
                toggleWindowChrome={toggleWindowChrome}
                legacyHistory={legacyHistory}
                toggleLegacyHistory={toggleLegacyHistory}
                authorName={authorName}
                setAuthorName={setAuthorName}
                theme={theme}
                setTheme={setTheme}
                onReplayTour={() => {
                  restartTour();
                  onClose();
                }}
              />
            )}
            {category === "stash" &&
              (current ? (
                <StashSettings
                  stashes={stashes}
                  onConfirmDrop={setConfirmDrop}
                  onConfirmDropAll={() => setShowDropAll(true)}
                />
              ) : (
                <NoRepoFallback />
              ))}
            {category === "performance" && (
              <>
                <CpuBudgetRow />
                {current ? (
                  <PerformanceSettings config={config} updateConfig={updateConfig} />
                ) : (
                  <NoRepoFallback />
                )}
              </>
            )}
            {category === "storage" && (
              <>
                <StoreRootRow />
                {current ? (
                  <StorageSettings
                    config={config}
                    updateConfig={updateConfig}
                    onShowCleanup={() => setShowCleanup(true)}
                    onShowCheck={() => setShowCheck(true)}
                  />
                ) : (
                  <NoRepoFallback />
                )}
              </>
            )}
          </div>
        </div>
      </Modal>
      {showCleanup && <CleanupModal onClose={() => setShowCleanup(false)} />}
      {showCheck && <CheckModal onClose={() => setShowCheck(false)} />}
      {showDropAll && <DropAllStashesModal onClose={() => setShowDropAll(false)} />}
      {confirmDrop && <DropStashModal stash={confirmDrop} onClose={() => setConfirmDrop(null)} />}
    </>
  );
}

/**
 * The set-aside shelf: every stash in the repo with its origin branch and age. Removing one
 * here is a discard, not a restore — bringing work back lives in the Changes panel menu, so
 * this list stays a management view. Confirms are raised to `SettingsModal` and rendered as
 * siblings, never nested inside this modal's panel.
 */
function StashShelf({
  stashes,
  onConfirmDrop,
  onConfirmDropAll,
}: {
  stashes: Stash[];
  onConfirmDrop: (s: Stash) => void;
  onConfirmDropAll: () => void;
}) {
  const { saving } = useRepository();
  const { artistMode } = useArtistMode();

  if (stashes.length === 0) {
    return (
      <p className="text-[12px] text-text-muted">
        {artistMode
          ? "Nothing set aside. Use the ⋮ menu in Changes to tuck work away and pick it up later."
          : "No stashes. Use the ⋮ menu in the Changes panel to stash your working tree."}
      </p>
    );
  }

  return (
    <>
      <ul className="flex flex-col">
        {stashes.map((s) => (
          <li key={s.id} className="group flex items-center gap-2 rounded-button py-1.5">
            <span className="min-w-0 flex-1">
              <span className="block truncate text-[13px] text-text">{stashTitle(s)}</span>
              <span className="mt-0.5 block truncate text-[11px] text-text-muted">
                {stashSummary(s)}
              </span>
            </span>
            <span className="shrink-0 opacity-0 transition-opacity group-hover:opacity-100">
              <IconButton
                icon={Trash}
                label={artistMode ? "Remove this set-aside work" : "Drop this stash"}
                size={16}
                disabled={saving}
                onClick={() => onConfirmDrop(s)}
              />
            </span>
          </li>
        ))}
      </ul>
      <Button variant="destructive" className="mt-3" disabled={saving} onClick={onConfirmDropAll}>
        <Trash size={14} />
        {artistMode ? "Remove all" : "Drop all stashes"}
      </Button>
    </>
  );
}

function DropStashModal({ stash, onClose }: { stash: Stash; onClose: () => void }) {
  const { dropStash, saving } = useRepository();
  const { artistMode } = useArtistMode();
  const [error, setError] = useState<string | null>(null);

  const onDrop = async () => {
    setError(null);
    try {
      await dropStash(stash.id);
      onClose();
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <Modal
      title={artistMode ? "Remove this work for good?" : "Drop this stash?"}
      onClose={() => (saving ? undefined : onClose())}
      footer={
        <>
          <Button onClick={onClose} disabled={saving}>
            Cancel
          </Button>
          <Button variant="destructive" onClick={onDrop} disabled={saving}>
            {saving ? "Removing…" : "Remove"}
          </Button>
        </>
      }
    >
      <p className="text-[13px] leading-relaxed text-text-muted">
        {artistMode ? (
          <>
            <span className="text-text">{stashTitle(stash)}</span> will be gone for good — this
            doesn't bring the files back first. The space it uses is freed the next time you clean
            up storage.
          </>
        ) : (
          <>
            Drops <span className="text-text">{stashTitle(stash)}</span> without restoring it. Its
            objects are reclaimed by the next storage cleanup.
          </>
        )}
      </p>
      {error && <p className="mt-3 text-[12px] text-danger">{error}</p>}
    </Modal>
  );
}

function DropAllStashesModal({ onClose }: { onClose: () => void }) {
  const { current, refreshNonce, dropAllStashes, saving } = useRepository();
  const { artistMode } = useArtistMode();
  const stashes = useStashes(current?.path ?? null, refreshNonce);
  const count = stashes.length;
  const [error, setError] = useState<string | null>(null);

  const onDropAll = async () => {
    setError(null);
    try {
      await dropAllStashes();
      onClose();
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <Modal
      title={artistMode ? "Empty the shelf?" : "Drop all stashes?"}
      onClose={() => (saving ? undefined : onClose())}
      footer={
        <>
          <Button onClick={onClose} disabled={saving}>
            Cancel
          </Button>
          <Button variant="destructive" onClick={onDropAll} disabled={saving}>
            {saving ? "Removing…" : "Remove all"}
          </Button>
        </>
      }
    >
      <p className="text-[13px] leading-relaxed text-text-muted">
        {artistMode ? (
          <>
            All {count} {count === 1 ? "piece" : "pieces"} of set-aside work will be gone for good,
            without coming back to your files first. The space is freed the next time you clean up
            storage.
          </>
        ) : (
          <>
            Drops all {count} {count === 1 ? "stash" : "stashes"} without restoring them. Their
            objects are reclaimed by the next storage cleanup.
          </>
        )}
      </p>
      {error && <p className="mt-3 text-[12px] text-danger">{error}</p>}
    </Modal>
  );
}

function formatBytes(n: number): string {
  if (n >= 1024 * 1024 * 1024) return `${(n / (1024 * 1024 * 1024)).toFixed(1)} GB`;
  if (n >= 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  if (n >= 1024) return `${Math.round(n / 1024)} KB`;
  return `${n} B`;
}

/** "Last backup: ..." hint for the Storage tab's zip-icon backup action (in the activity bar). */
function backupAgeLabel(lastBackupAt: string | undefined): string {
  if (!lastBackupAt) return "You haven't backed up this repository yet.";
  const days = Math.floor((Date.now() - new Date(lastBackupAt).getTime()) / 86_400_000);
  if (days <= 0) return "Last backed up today.";
  if (days === 1) return "Last backed up 1 day ago.";
  return `Last backed up ${days} days ago.`;
}

/** Backend problem kinds in plain language — the raw `detail` follows as the technical line. */
const PROBLEM_LABEL: Record<string, string> = {
  missingObject: "Missing stored data",
  brokenChain: "A version can't be rebuilt",
  danglingTip: "A branch points at a version that isn't there",
  badLogLine: "The history file is damaged",
  badPack: "A storage bundle is unreadable",
  corruptContent: "Stored data doesn't match what it should be",
};

type CheckScope = "current" | "all" | "neverChecked";
type CheckPhase = "confirm" | "running" | "done";
interface CheckResult {
  repo: Repository;
  report?: CheckReport;
  error?: string;
}

/**
 * "Check for problems": a read-only pass over stored history. Confirm a scope (this repo, every
 * added repo, or only repos never checked before) before it runs; it changes nothing on disk
 * either way. A multi-repo run can be canceled between repos — the repo currently in flight
 * always finishes (nothing in this codebase can abort a check mid-flight), but every repo still
 * queued is skipped.
 */
function CheckModal({ onClose }: { onClose: () => void }) {
  const { repositories, current, checkRepository } = useRepository();
  const [phase, setPhase] = useState<CheckPhase>("confirm");
  const [scope, setScope] = useState<CheckScope>("current");
  const [scrub, setScrub] = useState(false);
  const [queue, setQueue] = useState<Repository[]>([]);
  const [index, setIndex] = useState(0);
  const [results, setResults] = useState<CheckResult[]>([]);
  const [cancelling, setCancelling] = useState(false);
  const cancelledRef = useRef(false);

  const neverChecked = repositories.filter((r) => !hasBeenChecked(r.path));

  const run = async () => {
    const repos =
      scope === "all"
        ? repositories
        : scope === "neverChecked"
          ? neverChecked
          : current
            ? [current]
            : [];
    if (repos.length === 0) return;
    cancelledRef.current = false;
    setCancelling(false);
    setQueue(repos);
    setResults([]);
    setPhase("running");
    for (let i = 0; i < repos.length; i++) {
      if (i > 0 && cancelledRef.current) break;
      setIndex(i);
      const repo = repos[i];
      try {
        const report = await checkRepository(scrub, repo.path);
        setResults((prev) => [...prev, { repo, report: report ?? undefined }]);
      } catch (e) {
        setResults((prev) => [...prev, { repo, error: String(e) }]);
      }
    }
    setPhase("done");
  };

  const cancel = () => {
    cancelledRef.current = true;
    setCancelling(true);
  };

  const currentRepo = queue[index];
  const batch = queue.length > 1;
  const cancelledEarly = phase === "done" && cancelledRef.current && results.length < queue.length;
  const scopeCount =
    scope === "all"
      ? repositories.length
      : scope === "neverChecked"
        ? neverChecked.length
        : current
          ? 1
          : 0;

  return (
    <Modal
      title="Check for problems"
      onClose={() => (phase === "running" ? undefined : onClose())}
      footer={
        phase === "confirm" ? (
          <>
            <Button onClick={onClose}>Cancel</Button>
            <Button variant="primary" disabled={scopeCount === 0} onClick={run}>
              Run check
            </Button>
          </>
        ) : phase === "running" ? (
          <Button disabled={cancelling} onClick={cancel}>
            {cancelling ? "Cancelling…" : "Cancel"}
          </Button>
        ) : (
          <Button variant="primary" onClick={onClose}>
            Done
          </Button>
        )
      }
    >
      {phase === "confirm" && (
        <>
          <p className="mb-2 text-[13px] text-text">
            Looks over every version in your history and confirms the stored data behind it is still
            there. Nothing is changed or removed.
          </p>
          <fieldset className="mt-2 flex flex-col gap-2">
            <label className="flex items-start gap-2 text-[13px] text-text">
              <Radio
                name="check-scope"
                checked={scope === "current"}
                onChange={() => setScope("current")}
                disabled={!current}
                className="mt-0.5"
              />
              <span>Current repo{current ? ` (${current.name})` : ""}</span>
            </label>
            <label className="flex items-start gap-2 text-[13px] text-text">
              <Radio
                name="check-scope"
                checked={scope === "all"}
                onChange={() => setScope("all")}
                disabled={repositories.length === 0}
                className="mt-0.5"
              />
              <span>All repos ({repositories.length})</span>
            </label>
            <label className="flex items-start gap-2 text-[13px] text-text">
              <Radio
                name="check-scope"
                checked={scope === "neverChecked"}
                onChange={() => setScope("neverChecked")}
                disabled={neverChecked.length === 0}
                className="mt-0.5"
              />
              <span>Repos never checked before ({neverChecked.length})</span>
            </label>
          </fieldset>
          <label className="mt-3 flex items-start gap-2 text-[13px] text-text">
            <Checkbox
              checked={scrub}
              onChange={(e) => setScrub(e.target.checked)}
              className="mt-0.5"
            />
            <span>
              Also read back every version (slower)
              <span className="block text-[11px] text-text-muted">
                Re-hashes every version's stored content, not just its index.
              </span>
            </span>
          </label>
        </>
      )}

      {phase === "running" && (
        <p className="text-[12px] text-text-muted">
          {batch
            ? `Checking ${currentRepo?.name ?? "…"} (${index + 1} of ${queue.length})…`
            : "Checking…"}
        </p>
      )}

      {(phase === "running" || phase === "done") && results.length > 0 && (
        <ul className="mt-2 max-h-72 space-y-1.5 overflow-y-auto">
          {results.map(({ repo, report, error }, ri) => (
            <li key={ri}>
              {batch && (
                <p className="mb-1 text-[12px] font-medium text-text">
                  {error || (report && report.problems.length > 0)
                    ? repo.name
                    : `✓ ${repo.name} — all clear`}
                </p>
              )}
              {error && <p className="text-[12px] text-danger">{error}</p>}
              {!error && report && report.problems.length === 0 && !batch && (
                <p className="text-[13px] text-text">
                  All clear — {report.commitsChecked} version
                  {report.commitsChecked === 1 ? "" : "s"} and {report.objectsChecked} stored piece
                  {report.objectsChecked === 1 ? "" : "s"} checked out fine
                  {report.scrubPerformed
                    ? `, including a full read-back of all ${report.versionsScrubbed} of them.`
                    : "."}
                </p>
              )}
              {!error && report && report.problems.length > 0 && (
                <ul className="space-y-1.5">
                  {report.problems.map((p, i) => (
                    <li key={i} className="rounded-button bg-surface-2 px-2 py-1.5">
                      <span className="block text-[12px] text-text">
                        {PROBLEM_LABEL[p.kind] ?? p.kind}
                      </span>
                      <span className="block break-all text-[11px] text-text-muted">
                        {p.detail}
                      </span>
                    </li>
                  ))}
                </ul>
              )}
            </li>
          ))}
        </ul>
      )}

      {phase === "done" && (
        <>
          {results.some((r) => (r.report?.problems.length ?? 0) > 0) && (
            <p className="mt-2 text-[12px] text-text-muted">
              Your current artwork on disk is untouched — restore from a backup if any version won't
              open.
            </p>
          )}
          {cancelledEarly && (
            <p className="mt-2 text-[12px] text-text-muted">
              Cancelled after checking {results.length} of {queue.length} repos.
            </p>
          )}
        </>
      )}
    </Modal>
  );
}

/**
 * "Clean up storage": a dry run on open shows what a real pass would free (space held by
 * versions no branch can reach — leftovers of undo and deleted branches), then one confirm
 * runs it for real. Cleaning never touches current artwork or any version still in history.
 */
function CleanupModal({ onClose }: { onClose: () => void }) {
  const { cleanupRepository } = useRepository();
  const [preview, setPreview] = useState<CleanupReport | null>(null);
  const [result, setResult] = useState<CleanupReport | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    cleanupRepository(true)
      .then((r) => {
        if (!cancelled) setPreview(r);
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [cleanupRepository]);

  const clean = async () => {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      setResult(await cleanupRepository(false));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const totalOf = (r: CleanupReport) => r.bytesReclaimed + r.cacheBytesReclaimed;
  const nothing = preview != null && totalOf(preview) === 0;

  return (
    <Modal
      title="Clean up storage"
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>{result ? "Done" : "Cancel"}</Button>
          {!result && (
            <Button variant="primary" disabled={busy || preview == null || nothing} onClick={clean}>
              {busy ? "Cleaning…" : "Clean up"}
            </Button>
          )}
        </>
      }
    >
      <p className="mb-2 text-[13px] text-text">
        Frees space held by versions no longer part of any history — leftovers from undone saves and
        deleted branches. Your current artwork and every version you can still see are never
        touched.
      </p>
      {error && <p className="text-[12px] text-danger">{error}</p>}
      {!error && result ? (
        <p className="text-[13px] text-text">
          Freed <span className="font-medium">{formatBytes(totalOf(result))}</span>
          {result.cacheBytesReclaimed > 0 && (
            <span className="text-text-muted">
              {" "}
              (including {formatBytes(result.cacheBytesReclaimed)} of preview images that can be
              regenerated)
            </span>
          )}
          .
        </p>
      ) : !error && preview == null ? (
        <p className="text-[12px] text-text-muted">Checking what can be cleaned…</p>
      ) : !error && nothing ? (
        <p className="text-[12px] text-text-muted">Nothing to clean up — storage is tidy.</p>
      ) : (
        !error && (
          <p className="text-[13px] text-text">
            About <span className="font-medium">{formatBytes(totalOf(preview!))}</span> can be freed
            {preview!.cacheBytesReclaimed > 0 && (
              <span className="text-text-muted">
                , including preview images that can be regenerated
              </span>
            )}
            .
          </p>
        )
      )}
    </Modal>
  );
}

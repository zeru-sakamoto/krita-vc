import { useEffect, useState } from "react";
import { Broom, CaretDown } from "@phosphor-icons/react";
import { Modal } from "../ui/Modal";
import { Button } from "../ui/Button";
import { Menu } from "../ui/Menu";
import { useArtistMode } from "../../lib/artistMode";
import { useAuthorName } from "../../lib/authorName";
import { THEMES, useTheme } from "../../lib/theme";
import { useRepository, type CleanupReport } from "../../lib/repository";
import { useRepoConfig } from "../../lib/repoData";

const CACHE_PRESETS_MB = [128, 256, 512, 1024, 2048];

function ToggleRow({
  label,
  detail,
  active,
  onToggle,
}: {
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
      className="flex w-full items-start justify-between gap-3 rounded-button py-1.5 text-left"
    >
      <span className="min-w-0 text-[13px] text-text">
        {label}
        {detail && <span className="mt-0.5 block text-[11px] text-text-muted">{detail}</span>}
      </span>
      <span
        aria-hidden
        className={[
          "relative mt-0.5 h-5 w-9 shrink-0 rounded-full transition-colors duration-200",
          active ? "bg-accent" : "bg-surface-3 ring-1 ring-inset ring-border",
        ].join(" ")}
      >
        <span
          className={[
            "absolute left-0.5 top-1/2 size-4 -translate-y-1/2 rounded-full bg-white shadow-sm transition-transform duration-200 ease-out",
            active ? "translate-x-4" : "translate-x-0",
          ].join(" ")}
        />
      </span>
    </button>
  );
}

/** Small swatch: theme background with its accent dot — used in the trigger and each option. */
function ThemeChip({ bg, accent }: { bg: string; accent: string }) {
  return (
    <span
      aria-hidden
      className="flex size-4 shrink-0 items-center justify-center rounded-[3px] ring-1 ring-inset ring-black/25"
      style={{ backgroundColor: bg }}
    >
      <span className="size-1.5 rounded-full" style={{ backgroundColor: accent }} />
    </span>
  );
}

function SectionHeading({ children }: { children: React.ReactNode }) {
  return (
    <h3 className="mb-2 text-[11px] font-medium uppercase tracking-wide text-text-muted">
      {children}
    </h3>
  );
}

export function SettingsModal({ onClose }: { onClose: () => void }) {
  const { current } = useRepository();
  const { artistMode, toggle: toggleArtistMode } = useArtistMode();
  const { authorName, setAuthorName } = useAuthorName();
  const { theme, setTheme } = useTheme();
  const { config, update: updateConfig } = useRepoConfig(current?.path ?? "");
  const [showCleanup, setShowCleanup] = useState(false);

  return (
    <>
      <Modal
        title="Settings"
        onClose={onClose}
        footer={<Button onClick={onClose}>Done</Button>}
        maxWidthClassName="max-w-lg"
      >
        <section className="mb-5">
          <SectionHeading>Appearance</SectionHeading>
          <ToggleRow
            label="Artist view"
            detail="Plain-language labels and version numbers instead of the technical view."
            active={artistMode}
            onToggle={toggleArtistMode}
          />
          <label className="mt-2 block">
            <span className="mb-1 block text-[12px] text-text-muted">Your name</span>
            <input
              value={authorName}
              onChange={(e) => setAuthorName(e.target.value)}
              placeholder="You"
              className="w-full rounded-button border border-border bg-surface-2 px-2 py-1.5 text-[13px] text-text placeholder:text-text-muted focus:border-accent focus:outline-none"
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
                      "flex min-w-[200px] items-center gap-2 rounded-button border bg-surface-2 px-2 py-1.5 text-[13px] text-text",
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
        </section>

        {current && (
          <section>
            <SectionHeading>This repository</SectionHeading>

            <label className="mb-3 block">
              <span className="mb-1 block text-[12px] text-text-muted">Preview cache size</span>
              <select
                value={config ? Math.round(config.cacheMaxBytes / (1024 * 1024)) : ""}
                onChange={(e) =>
                  config &&
                  updateConfig({ ...config, cacheMaxBytes: Number(e.target.value) * 1024 * 1024 })
                }
                disabled={!config}
                className="w-full rounded-button border border-border bg-surface-2 px-2 py-1.5 text-[13px] text-text focus:border-accent focus:outline-none disabled:opacity-50"
              >
                {CACHE_PRESETS_MB.map((mb) => (
                  <option key={mb} value={mb}>
                    {mb >= 1024 ? `${mb / 1024} GB` : `${mb} MB`}
                  </option>
                ))}
              </select>
              <span className="mt-1 block text-[11px] text-text-muted">
                How much space diff previews may use on disk. Oldest previews are cleared first once
                you go over — they regenerate automatically when needed again.
              </span>
            </label>

            <ToggleRow
              label="Compact storage for heavily-revised art"
              detail="Shrinks version history for files with many small edits by 2–10x, at the
                cost of a little extra time on each save and restore. Safe to turn on or off at
                any point — past versions are unaffected either way."
              active={config?.tilePixelDeltas ?? false}
              onToggle={() =>
                config && updateConfig({ ...config, tilePixelDeltas: !config.tilePixelDeltas })
              }
            />

            <Button className="mt-3" onClick={() => setShowCleanup(true)}>
              <Broom size={14} />
              Clean up storage…
            </Button>
          </section>
        )}
      </Modal>
      {showCleanup && <CleanupModal onClose={() => setShowCleanup(false)} />}
    </>
  );
}

function formatBytes(n: number): string {
  if (n >= 1024 * 1024 * 1024) return `${(n / (1024 * 1024 * 1024)).toFixed(1)} GB`;
  if (n >= 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  if (n >= 1024) return `${Math.round(n / 1024)} KB`;
  return `${n} B`;
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

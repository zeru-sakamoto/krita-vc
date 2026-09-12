import { useEffect, useRef, useState } from "react";
import { Check, ShieldCheck, User as UserIcon } from "@phosphor-icons/react";
import { Button } from "../ui/Button";
import { WindowControls } from "./TopBar";
import { ICON } from "../../lib/iconSize";
import { useAuthorName } from "../../lib/authorName";
import { useOnboarding } from "../../lib/onboarding";
import { THEMES, useTheme, type ThemeId } from "../../lib/theme";
import { useWindowChrome } from "../../lib/windowChrome";
import { inTauri } from "../../lib/tauri";

/**
 * First-launch interview: the artist's name, then a theme. Full-window and deliberately not a
 * `Modal` — Escape/scrim-dismiss would make it a thing you close by accident and never see
 * again. Both answers are saved the moment they change (through the existing providers), so
 * closing the window mid-way loses nothing; only the "done" flag waits for the last button.
 */
export function OnboardingOverlay() {
  const { active, finish } = useOnboarding();
  const [step, setStep] = useState<"name" | "theme">("name");

  // A replay from Settings starts from the top, not wherever the last run stopped.
  useEffect(() => {
    if (active) setStep("name");
  }, [active]);

  if (!active) return null;

  return (
    <div className="fixed inset-0 z-(--z-onboarding) flex flex-col bg-bg text-text">
      <TitleStrip />
      <div className="grid min-h-0 flex-1 place-items-center overflow-y-auto px-4 pb-8">
        {step === "name" ? (
          <NameStep onNext={() => setStep("theme")} onSkip={finish} />
        ) : (
          <ThemeStep onBack={() => setStep("name")} onDone={finish} />
        )}
      </div>
    </div>
  );
}

/** The overlay covers `TopBar`, which is also the window's title bar when the custom one is on —
 *  so it carries its own drag region and window buttons, or the window couldn't be moved/closed. */
function TitleStrip() {
  const { customTitleBar } = useWindowChrome();
  const show = customTitleBar && inTauri();
  return (
    <div
      className="flex h-11 shrink-0 items-center pr-2"
      {...(show ? { "data-tauri-drag-region": true } : {})}
    >
      {show && <WindowControls />}
    </div>
  );
}

function StepDots({ index }: { index: 0 | 1 }) {
  return (
    <div className="flex gap-1.5" aria-label={`Step ${index + 1} of 2`}>
      {[0, 1].map((i) => (
        <span
          key={i}
          className={[
            "h-1.5 rounded-full transition-[width,background-color] duration-(--dur-normal) ease-(--ease-out)",
            i === index ? "w-4 bg-accent" : "w-1.5 bg-border",
          ].join(" ")}
        />
      ))}
    </div>
  );
}

function NameStep({ onNext, onSkip }: { onNext: () => void; onSkip: () => void }) {
  const { authorName, setAuthorName } = useAuthorName();
  const inputRef = useRef<HTMLInputElement>(null);
  useEffect(() => inputRef.current?.focus(), []);

  return (
    <form
      className="raised flex w-full max-w-md flex-col gap-5 rounded-modal bg-surface p-8"
      onSubmit={(e) => {
        e.preventDefault();
        onNext();
      }}
    >
      <div className="flex flex-col items-center gap-3 text-center">
        <img src="/logo.svg" alt="" className="h-10 w-10" />
        <h1 className="text-title font-semibold">Welcome to krita-vc</h1>
        <p className="text-body leading-relaxed text-text-muted">
          Two quick things before you start. You can change both later in Settings.
        </p>
      </div>

      <label className="block">
        <span className="mb-1 flex items-center gap-1.5 text-dense text-text-muted">
          <UserIcon size={ICON.inline} />
          Your name
        </span>
        <input
          ref={inputRef}
          value={authorName}
          onChange={(e) => setAuthorName(e.target.value)}
          placeholder="You"
          maxLength={80}
          // Same field treatment as Settings → Appearance → Your name (see the note there on
          // why `!outline-none` stays).
          className="w-full inset-well rounded-button border border-border bg-bg px-2 py-1.5 text-body text-text placeholder:text-text-muted transition-[color,background-color,border-color] duration-(--dur-fast) ease-(--ease-out) focus-visible:border-accent focus-visible:bg-surface !outline-none"
        />
        <span className="mt-1 block text-caption text-text-muted">
          Shown as the author of each version you save.
        </span>
      </label>

      <div className="flex items-start gap-2.5 rounded-panel border border-border bg-surface-2 p-3">
        <ShieldCheck size={ICON.default} className="mt-0.5 shrink-0 text-accent" />
        <p className="text-dense leading-relaxed text-text-muted">
          <span className="font-medium text-text">Everything stays on this computer.</span> krita-vc
          has no accounts and sends nothing anywhere — your name, your art and its history never
          leave your machine.
        </p>
      </div>

      <div className="flex items-center justify-between">
        <StepDots index={0} />
        <div className="flex gap-2">
          <Button variant="ghost" onClick={onSkip}>
            Skip
          </Button>
          <Button variant="primary" type="submit">
            Next
          </Button>
        </div>
      </div>
    </form>
  );
}

function ThemeStep({ onBack, onDone }: { onBack: () => void; onDone: () => void }) {
  const { theme, setTheme } = useTheme();

  return (
    <div className="raised flex w-full max-w-3xl flex-col gap-5 rounded-modal bg-surface p-8">
      <div className="flex flex-col gap-1 text-center">
        <h1 className="text-title font-semibold">Pick a look</h1>
        <p className="text-body text-text-muted">
          The app changes as you click, so you can see it for real.
        </p>
      </div>

      <div role="radiogroup" aria-label="Theme" className="grid grid-cols-2 gap-3 sm:grid-cols-4">
        {THEMES.map((t) => (
          <ThemePreviewCard
            key={t.id}
            id={t.id}
            label={t.label}
            selected={t.id === theme}
            onSelect={() => setTheme(t.id)}
          />
        ))}
      </div>

      <div className="flex items-center justify-between">
        <StepDots index={1} />
        <div className="flex gap-2">
          <Button variant="ghost" onClick={onBack}>
            Back
          </Button>
          <Button variant="primary" onClick={onDone}>
            Get started
          </Button>
        </div>
      </div>
    </div>
  );
}

/**
 * A miniature of the app shell painted in theme `id`, whatever theme is active. The palette comes
 * from global.css's `[data-theme-preview]` selectors — the theme's custom properties re-declared
 * on this subtree, so plain Tailwind utilities inside resolve to it. Only the identity tokens are
 * re-scoped: `:root`-derived vars (`raised`/`--shadow-*`, `--glass-*`, `--color-state-*`) were
 * computed at `:root` from the *active* theme, so the mock uses plain fills and borders only.
 */
function ThemePreviewCard({
  id,
  label,
  selected,
  onSelect,
}: {
  id: ThemeId;
  label: string;
  selected: boolean;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={selected}
      onClick={onSelect}
      className="group flex flex-col gap-2 text-left"
    >
      <div
        data-theme-preview={id}
        className={[
          "relative flex aspect-8/5 w-full gap-1.5 overflow-hidden rounded-panel border bg-bg p-1.5",
          "transition-[border-color,box-shadow] duration-(--dur-fast) ease-(--ease-out)",
          selected
            ? "border-accent ring-2 ring-accent"
            : "border-border group-hover:border-text-muted",
        ].join(" ")}
      >
        {/* sidebar */}
        <div className="flex w-1/3 flex-col gap-1 rounded-badge border border-border bg-surface p-1">
          <span className="h-2 rounded-sm bg-accent" />
          <span className="h-2 rounded-sm bg-surface-2" />
          <span className="h-2 rounded-sm bg-surface-2" />
        </div>
        {/* main card */}
        <div className="flex flex-1 flex-col gap-1.5 rounded-badge border border-border bg-surface p-1.5">
          <span className="h-1.5 w-3/4 rounded-full bg-text" />
          <span className="h-1.5 w-1/2 rounded-full bg-text-muted" />
          <span className="flex-1 rounded-sm bg-surface-2" />
          <span className="h-2.5 w-2/5 self-end rounded-full bg-accent" />
        </div>
        {selected && (
          <span className="absolute top-1 right-1 grid h-4 w-4 place-items-center rounded-full bg-accent text-bg">
            <Check size={ICON.inline} weight="bold" />
          </span>
        )}
      </div>
      <span
        className={["text-dense", selected ? "font-medium text-text" : "text-text-muted"].join(" ")}
      >
        {label}
      </span>
    </button>
  );
}

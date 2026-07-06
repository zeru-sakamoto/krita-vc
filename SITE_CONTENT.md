# Krita VCS

### Version control for your art, not your code.

Krita VCS is a free, local-only version-control app for Krita painters. Every save becomes a
version you can revisit, compare, branch from, or roll back to — with zero cloud accounts, zero
uploads, and zero git jargon.

[Download for free →] [View source on GitHub →]

![Hero screenshot placeholder — main window with history graph and visual diff open](placeholder)

---

## Why artists use it

- **Nothing ever leaves your computer.** All history lives in a hidden folder inside your own
  project — no account, no sync, no remote server. Ever.
- **It understands your painting, not just your file.** Krita VCS reads `.kra` files down to the
  individual tile, so a single brush stroke only stores what actually changed — not a full copy
  of your painting every time you hit save.
- **See what changed, visually.** No line numbers, no diffs full of symbols — just your layers,
  before and after, side by side or on a swipe slider.
- **Explore without fear.** Try a new color direction or a redesign on a branch, keep painting on
  your original at the same time, and merge the two back together whenever you're ready.
- **Built for real Krita files.** Performance-tuned so commits, comparisons, and restores stay
  fast even on large, layer-heavy paintings — not just toy test files.

---

## Features

### Visual layer diffs
Compare any two versions of a painting layer by layer. Toggle side-by-side or swipe-slider
comparison, zoom and pan to inspect details — perfectly synced between both views — and see
exactly what changed via a highlighted overlay with a dashed outline tracing the edited pixels'
silhouette. Focus a single layer and the highlight narrows to just that layer's changes.
Composites load first so you're never staring at a blank panel; individual layers stream in
right after.

![Screenshot placeholder — layer stack panel + swipe slider diff](placeholder)

### Real history, real branches
Every save is a full point you can return to. Branch off to try something risky, switch back
instantly, and merge branches together when you're happy — conflicting edits are flagged, never
silently overwritten. A color-coded history graph shows exactly how your branches connect.

![Screenshot placeholder — branch history graph with multiple colored lanes](placeholder)

### Undo and rollback, without losing anything
Made a save you regret? Undo it. Want to go back three versions? Roll back to it. Krita VCS never
deletes your history behind your back — old versions stay recoverable until you decide otherwise.

### Clean up storage, on your terms
Because nothing is destroyed automatically, a project's history can grow over time. One button
shows you exactly how much space old, unreachable history is taking up, and reclaims it — only
when you say so.

![Screenshot placeholder — cleanup/storage confirmation dialog](placeholder)

### Palette diffs
Working with `.gpl` color palettes? Krita VCS shows a color-by-color swatch comparison, with hex
values, so palette tweaks are as easy to review as a painting.

### Artist Mode
Turn off the technical language entirely. Artist Mode (on by default) swaps commit hashes for
"Version 12," file paths for asset names, and change codes for plain words like "Updated" — so
version control reads like a tool for artists, not developers. Prefer the technical view? Turn it
off any time.

![Screenshot placeholder — Artist Mode toggle showing before/after labeling](placeholder)

---

## What's next

Krita VCS is actively developed. A few things we're still improving:

- **Smarter diffs for non-painting files** — text/config files in a project currently get a basic
  line view; a full visual diff for these is planned.
- **Deeper zoom for very large paintings** — comparisons render at a capped resolution to keep
  cached previews small; a way to go past that cap for pixel-level review on huge canvases is
  on the roadmap.

Have a feature request? [Open an issue on GitHub →]

---

## FAQ

**Does this replace Krita?**
No — Krita VCS runs alongside Krita as a separate app. You still paint in Krita; Krita VCS just
watches your project folder and manages its history.

**Is my art uploaded anywhere?**
No. Krita VCS is local-only by design — there's no remote server, no account, and no sync. All
version history is stored in a folder on your own machine.

**Is it free?**
Yes, Krita VCS is free and open source. _(License: TBD — check the repository for current
licensing terms.)_

**What platforms does it support?**
Krita VCS is a desktop app built with Tauri, currently targeting Windows, with macOS/Linux support
following the same cross-platform base.

**Does it work with any file, or just `.kra`?**
Krita VCS tracks your whole project folder, but its deep visual diffing — layer-by-layer
comparison — is built specifically for Krita's `.kra` format and `.gpl` palettes. Other files in
the folder are still tracked and versioned, with a simpler diff view.

**Will my history get huge over time?**
Krita VCS only stores what changed between saves, not a full copy each time, so history stays
compact. If you ever want to reclaim space from old, unreachable versions, the built-in
"Clean up storage" tool does it with your confirmation.

---

_Screenshots throughout this page are placeholders — final captures to be added._

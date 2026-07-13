# Krita VCS

### Version control for your art, not your code.

Krita VCS is a free, local-only version-control app for Krita painters. Every save becomes a
version you can revisit, compare, branch from, or roll back to, with zero cloud accounts, zero
uploads, and zero git jargon.

[Download for free →] [View source on GitHub →]

![Hero screenshot placeholder: main window with history graph and visual diff open](placeholder)

---

## Why artists use it

- **Nothing ever leaves your computer.** All history lives in a hidden folder inside your own
  project. No account, no sync, no remote server. Ever.
- **It understands your painting, not just your file.** Krita VCS reads `.kra` files down to the
  individual tile, so a single brush stroke only stores what actually changed, not a full copy
  of your painting every time you hit save.
- **See what changed, visually.** No line numbers, no diffs full of symbols. Just your layers,
  before and after, side by side or on a swipe slider.
- **Explore without fear.** Try a new color direction or a redesign on a branch, switch back to
  your original whenever you like, and merge the two back together when you're ready.
- **Built for real Krita files.** Performance-tuned so commits, comparisons, and restores stay
  fast even on large, layer-heavy paintings, not just toy test files.

---

## Features

### Visual layer diffs

Compare any two versions of a painting layer by layer. Toggle side-by-side or swipe-slider
comparison, zoom and pan to inspect details (perfectly synced between both views), and see
exactly what changed via a highlighted overlay with a dashed outline tracing the edited pixels'
silhouette. Focus a single layer and the highlight narrows to just that layer's changes.
Composites load first so you're never staring at a blank panel; individual layers stream in
right after. Click any layer to see its details: type, visibility, opacity, blend mode, and
painted area, or the whole canvas's size, resolution, and color space.

![Screenshot placeholder: layer stack panel + swipe slider diff](placeholder)

### Real history, real branches

Every save is a full point you can return to. Branch off to try something risky, switch back
instantly, and merge branches together when you're happy. Conflicting edits are flagged, never
silently overwritten. A color-coded history graph shows exactly how your branches connect.

![Screenshot placeholder: branch history graph with multiple colored lanes](placeholder)

### Undo and rollback, without losing anything

Made a save you regret? Undo it. Want to go back three versions? Roll back to it. Krita VCS never
deletes your history behind your back. Old versions stay recoverable until you decide otherwise.

### Clean up storage, on your terms

Because nothing is destroyed automatically, a project's history can grow over time. One button
shows you exactly how much space old, unreachable history is taking up, and reclaims it, only
when you say so.

![Screenshot placeholder: cleanup/storage confirmation dialog](placeholder)

### Palette diffs

Working with color palettes? Krita VCS shows a color-by-color swatch comparison, with hex
values, so palette tweaks are as easy to review as a painting. It works across every common
palette format: `.gpl` (GIMP), `.kpl` (Krita), and Adobe's `.aco` and `.ase`, no matter where
your palette comes from.

### Artist Mode

Turn off the technical language entirely. Artist Mode (on by default) swaps commit hashes for
"Version 12," file paths for asset names, and change codes for plain words like "Updated," so
version control reads like a tool for artists, not developers. Prefer the technical view? Turn it
off any time.

![Screenshot placeholder: Artist Mode toggle showing before/after labeling](placeholder)

### Sign your work, tune it to your machine

Everything lives in one Settings panel. Put your name on every version you save, so a shared
project reads like a record of who did what. Set how much disk the preview images may use, or
turn on compact storage to shrink the history of heavily-revised paintings. All optional, all
changeable any time. Prefer your own window frame over ours? Toggle the custom title bar off
any time, no restart needed.

### Pick a theme

Choose from eight color themes right in Settings: six dark, two light, from the moody default
Charcoal to Krita Blue, Tokyo Night, and True Black. Your pick is saved on your machine and
applied instantly, no restart needed.

| Theme              | Background | Accent    |
| ------------------ | ---------- | --------- |
| Charcoal (default) | `#131210`  | `#e07b39` |
| Krita Blue         | `#1e1e24`  | `#2e86de` |
| Electric Cyan      | `#1a1d24`  | `#00d2d3` |
| Sunset Coral       | `#201e22`  | `#ff6b6b` |
| Tokyo Night        | `#1a1b26`  | `#7aa2f7` |
| True Black         | `#000000`  | `#8b5cf6` |
| Charcoal Light     | `#f4f1ea`  | `#a8511a` |
| Studio Light       | `#f5f6fa`  | `#2e86de` |

![Screenshot placeholder: Settings panel with name field and storage options](placeholder)

---

## What's next

Krita VCS is actively developed. A few things we're still improving:

- **Diff stashing**: set aside an in-progress comparison and come back to it later without
  losing your place, so you can hop between reviews without re-picking the same two versions.
- **A guided first-launch tour**: a dynamic walkthrough of the app on first open, pointing out
  the repository switcher, Changes, History, and Settings so new users aren't left guessing.
- **The Krita plugin**: an optional in-Krita "Version Control" panel (commit, quick-checkpoint,
  branch switching, no window-switching) built on the same history as the main app. Available
  today as a manual build; see the [Krita plugin install guide on GitHub →].

Have a feature request? [Open an issue on GitHub →]

---

## Documentation

New to Krita VCS? A full walkthrough covering installation, saving your first version, using
every feature, and the guardrails that keep your work safe lives on its own page.

[Getting Started with Krita VCS →](SITE_CONTENT_GETTING_STARTED.md)

[Read the full docs on GitHub →]

---
## FAQ

**What does it do?**
It keeps every version of your painting as you save, like a save file for each stage of your art.
You can look back at any earlier version, compare two side by side, or go back to one if you
change your mind, all without leaving a mess of duplicate files on your computer.

**Is my art uploaded anywhere?**
No. Krita VCS is local-only by design: there's no server, no account, and no sync. Every version
lives in a folder on your own machine.

**Is it free?**
Yes, Krita VCS is free and open source. The license is still being finalised, so check the
repository for current terms.

**What platforms does it support?**
It's a desktop app built with Tauri, currently targeting Windows, with macOS and Linux following
on the same cross-platform base.

**Does it work with any file, or just `.kra`?**
It tracks the file types it understands and leaves the rest of your folder alone: Krita paintings
(`.kra`), with the deep layer-by-layer visual diff, and color palettes (`.gpl`, `.kpl`, `.aco`,
`.ase`), with a color-by-color swatch diff. Other files sitting in the project folder aren't
touched. They're never copied into its history.

**Will my history get huge over time?**
It only stores what changed between saves, not a full copy each time, so history stays compact.
And if you ever want the space back from old, unreachable versions, the built-in "Clean up
storage" tool does it, with your confirmation.

---


_Screenshots throughout this page are placeholders. Final captures to be added._

# Krita VCS

A desktop **version-control client for Krita art files**, built with Tauri 2 + React 19 +
TypeScript. Instead of code-style text patches, it shows artists what actually changed: the
**layer stack** and a **visual diff** (before/after, swipe slider, and change highlighting) of each
`.kra` file.

> **Status:** the frontend is fully built but **driven by mock data** — nothing talks to git or the
> filesystem yet. The Rust backend is still the scaffolded `greet` command. The component and data
> boundaries are designed so the real backend can be dropped in without reworking the UI.

## Features (frontend)

- **Visual layer diffs** for `.kra` files — a Krita-style layer panel beside a before/after canvas.
  - **Side-by-side** and **swipe slider** compare modes.
  - **Change highlighting** you can toggle on/off and switch between translucent region boxes and a
    precise changed-shape mask.
  - Click a layer to focus its diff, or view the composited artwork.
- **Functional sidebar tabs** — Changes (staged/unstaged working tree), History (commit timeline),
  and Branches (local-only) — all mock-only for now.
- A dark, Krita-inspired UI built against [`DESIGN.md`](DESIGN.md).
- Non-art files (palettes, config) still render as a familiar text diff.

## Getting started

Package manager is **npm**.

```bash
npm install          # install JS dependencies
npm run tauri dev    # run the full desktop app (Vite dev server + Tauri webview)
```

Frontend-only (in a browser, no Tauri shell):

```bash
npm run dev          # Vite dev server at http://localhost:1420
```

Build / package:

```bash
npm run build        # type-check (tsc) + build the frontend bundle to dist/
npm run tauri build  # production desktop bundle (frontend build + Rust binary + installers)
```

Rust side (from `src-tauri/`): `cargo check` / `cargo build` / `cargo test`.

## Project layout

```
src/
├─ components/
│  ├─ shell/   — AppShell, ActivityBar, Sidebar, Inspector, StatusBar, DockerPanel
│  ├─ vcs/     — diff viewer (ArtDiffView, ArtCanvas, CompareSlider, LayerStackPanel),
│  │            commit list, branch/changes panels, status chips
│  ├─ ui/      — IconButton, Button
│  └─ MainPanel.tsx
├─ data/       — MOCK data + generated SVG artwork (replace when the backend lands)
├─ styles/     — global.css (Tailwind v4 @theme tokens from DESIGN.md)
└─ types.ts    — domain types

src-tauri/     — Rust backend (Tauri 2)
docs/          — developer documentation
DESIGN.md      — visual + interaction spec
```

## Documentation

- [`docs/`](docs/README.md) — frontend architecture, the visual diff viewer, and the mock-data model.
- [`DESIGN.md`](DESIGN.md) — design tokens, components, and interaction spec.
- [`CLAUDE.md`](CLAUDE.md) — repo guidance and commands.

## Recommended IDE setup

[VS Code](https://code.visualstudio.com/) +
[Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) +
[rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer).

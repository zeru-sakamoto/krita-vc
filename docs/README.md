# Krita VCS — Documentation

Developer documentation for the Krita VCS desktop app (Tauri 2 + React 19 + TypeScript).

> **Status:** the frontend is fully built but **driven by mock data** — nothing talks to git or
> the filesystem yet. The Rust backend is still the scaffolded `greet` command. These docs describe
> the frontend as it stands and mark the seams where the real backend will plug in.

## Contents

- [**Frontend architecture**](frontend-architecture.md) — app shell, the four zones, state
  ownership, the component map, and **Artist Mode** (the global friendly-labels toggle).
- [**Visual diff viewer**](visual-diff-viewer.md) — how art (`.kra`) files render as layer images
  and visual diffs: the data model, the generated-SVG mock art, and the highlight/compare modes.
- [**Mock data model**](mock-data.md) — the domain types in `src/types.ts` and the mock modules in
  `src/data/`, plus the plan for replacing them with a real backend.

## See also

- [`../DESIGN.md`](../DESIGN.md) — the visual + interaction spec the UI is built against.
- [`../CLAUDE.md`](../CLAUDE.md) — repo guidance, commands, and Tauri architecture.

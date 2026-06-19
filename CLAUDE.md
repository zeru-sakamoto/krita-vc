# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project status

This is a freshly scaffolded Tauri 2 + React 19 + TypeScript desktop app (generated via `create-tauri-app`, not yet customized). There is no test runner or linter configured yet — if you add one, update this file with the relevant commands.

## Commands

Package manager is npm (`package-lock.json` is present).

- `npm install` — install JS dependencies
- `npm run dev` — start the Vite dev server only (frontend in browser, no Tauri shell)
- `npm run build` — type-check (`tsc`) then build the frontend bundle to `dist/`
- `npm run preview` — preview the built frontend
- `npm run tauri dev` — run the full desktop app (spawns the Vite dev server per `beforeDevCommand`, then opens the Tauri/webview window); this is the normal way to run the app end-to-end
- `npm run tauri build` — produce a production desktop bundle (runs `npm run build` first per `beforeBuildCommand`, then compiles the Rust binary and packages installers)

Rust side (run from `src-tauri/`):
- `cargo check` / `cargo build` — compile the Rust backend without going through the Tauri CLI
- `cargo test` — run Rust unit tests (none exist yet)

## Architecture

This is a Tauri 2 app: a React/TypeScript frontend rendered in a native webview, paired with a Rust backend process.

- **Frontend** (`src/`): standard Vite + React 19 + TypeScript app. Entry point `src/main.tsx` mounts `App.tsx` into `index.html`. Built output goes to `dist/`, which `src-tauri/tauri.conf.json` (`build.frontendDist`) points at for packaged builds.
- **Backend** (`src-tauri/`): Rust crate `krita_vc_lib`. `src-tauri/src/main.rs` is the binary entry point and just calls `krita_vc_lib::run()` defined in `src-tauri/src/lib.rs`, where the `tauri::Builder` is configured, plugins are registered, and Tauri commands are wired up via `invoke_handler(tauri::generate_handler![...])`.
- **Frontend ↔ backend IPC**: Rust functions annotated `#[tauri::command]` (e.g. `greet` in `lib.rs`) are exposed to the frontend and called via `invoke("command_name", { args })` from `@tauri-apps/api/core`. New backend functionality should be added as a `#[tauri::command]` in `lib.rs` (or a module it includes) and registered in `generate_handler!`.
- **Permissions/capabilities**: `src-tauri/capabilities/default.json` declares which Tauri permissions (e.g. `core:default`, `opener:default`) the main window is allowed to use. Any new Tauri plugin or privileged API needs its permission added here or the call will be rejected at runtime.
- **Dev server coupling**: `vite.config.ts` hardcodes port `1420` (`strictPort: true`) and `src-tauri/tauri.conf.json`'s `build.devUrl` points at `http://localhost:1420`. These must stay in sync — Tauri's dev shell loads the app from that fixed URL. `src-tauri/` is excluded from Vite's file watcher.
- **App identity/config**: window size, app identifier (`com.zeru-sakamoto.krita-vc`), and bundle/icon settings live in `src-tauri/tauri.conf.json`.

Recommended editor setup (from README): VS Code with the Tauri and rust-analyzer extensions (already listed in `.vscode/extensions.json`).

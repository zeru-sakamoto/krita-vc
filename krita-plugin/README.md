# Krita VC plugin

A "Version Control" docker for Krita: commit, quick-checkpoint, and branch-switch
against the same `.kvc` repository the desktop app uses, without leaving Krita.

It shells out to the `kvc` companion CLI (built from `src-tauri`, target `kvc`/`kvc.exe`)
rather than talking to the store directly, so both the plugin and the desktop app go
through the exact same engine code.

Out of scope here by design: creating/initializing a repository, browsing or restoring
history, and anything remote — all of that stays in the desktop app.

## Requirements

- Krita with Python scripting enabled (bundled by default in the official Krita builds;
  Settings → Configure Krita → Python Plugin Manager should already list some plugins).
- The `kvc` CLI, built from this repository (see below) — there's no separate download yet.

## Install

1. Build the CLI once, from `src-tauri/`:
   ```
   cargo build --release --bin kvc
   ```
   The binary lands at `src-tauri/target/release/kvc` (`kvc.exe` on Windows).
2. Copy this folder's contents into Krita's `pykrita` resource folder. From Krita:
   **Settings → Manage Resources → Open Resource Folder**, then into the `pykrita/`
   subfolder there (on Windows this is normally `%APPDATA%\krita\pykrita`):
   ```
   pykrita/kritavc.desktop
   pykrita/kritavc/
   ```
3. In Krita, enable it under **Settings → Configure Krita → Python Plugin Manager**
   → "Krita VC", then **restart Krita** (Python plugins are only loaded at startup).
4. Open the docker: **Settings → Dockers → Version Control**.
5. If the docker shows "kvc command-line tool wasn't found", click **Locate kvc…**
   and point it at the binary from step 1.

## Using it

Open a `.kra` file that lives inside a folder already tracked by the desktop app (i.e.
some ancestor folder has a `.kvc/` directory). The docker shows the current branch, the
working-tree changelist, and a message box with **Commit** and **⚡ Checkpoint** (a
one-tap commit with an auto-generated "Checkpoint HH:MM" message). Both are disabled
until the document is saved — the plugin only ever reads what's on disk, it never saves
your `.kra` for you.

## Troubleshooting

- **"Version Control" isn't in the Dockers menu** — the plugin didn't load. Re-check
  step 3, and confirm both `pykrita/kritavc.desktop` and `pykrita/kritavc/*.py` landed
  in the resource folder from step 2 (not one level up or down).
- **"kvc command-line tool wasn't found"** persists after Browse — make sure you picked
  the `kvc`/`kvc.exe` binary itself, not the folder, and that it's the one built in step 1
  (not the main `krita-vc`/`krita-vc.exe` app binary, which is a different target).
- **"repository is busy (locked by another kvc process)"** — a previous commit/switch
  didn't exit cleanly and left `.kvc/kvc.lock` behind. Safe to delete that file by hand
  as long as no other `kvc`/desktop-app write is actually in flight.

## Uninstall

Disable "Krita VC" in the Python Plugin Manager, then delete `pykrita/kritavc.desktop`
and `pykrita/kritavc/` from the resource folder.

## Notes

- Commit and Checkpoint are disabled while the active document has unsaved changes —
  the plugin only ever reads/commits what's on disk, it never writes your `.kra`.
- Author name is a plugin-local setting (Krita has no shared login with the desktop
  app); it defaults to `"You"`, matching the desktop app's own fallback.

# Krita VC plugin

A "Version Control" docker for Krita: commit, quick-checkpoint, discard, set work aside,
and branch-switch against the same `.kvc` repository the desktop app uses, without leaving
Krita.

It shells out to the `kvc` companion CLI (built from `src-tauri`, target `kvc`/`kvc.exe`)
rather than talking to the store directly, so both the plugin and the desktop app go
through the exact same engine code.

Out of scope here by design: creating/initializing a repository, browsing or restoring
history, undoing a version, merging/deleting branches, and anything remote — all of that
stays in the desktop app.

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
one-tap commit with an auto-generated "Checkpoint HH:MM" message).

**You don't need to press Ctrl+S first.** Versions are built from what's on disk, so the
docker saves for you: clicking into the panel saves every open document in the repository
that has unsaved changes, and Commit and Checkpoint save before they capture anything. The
**⟳** button does the same on demand, then rescans. So the changelist describes your canvas,
not your last manual save, and a commit can't quietly miss the last ten minutes of painting.

Two things follow from that. Krita's own **autosave and backup files are never versioned** —
they're ignored by the scanner, so they can't turn up in a version. And **anything the docker
saves is still not a version** until you commit it: saving is not committing, and Discard
(below) throws saved-but-uncommitted work away.

**Ticking files.** Every row in the changelist has a checkbox, and everything starts
ticked — ignore them and Commit saves the lot, exactly as it always has. Untick a file to
leave it out. Commit, Checkpoint, **Discard** and **Set aside** all act on the ticked rows.
This is deliberately simpler than the desktop app's staged/unstaged split: one list, one
tick, and no "commit everything anyway?" prompt to answer.

**The ⋯ menu** carries the rest, in the same three groups as the desktop's panel menu:
discard the ticked changes; set the ticked changes aside; bring back the latest set-aside
(or pick one from a list). Setting work aside parks it off to the side of history and puts
the files back to your last version — nothing is lost, and it's the fastest way past a
branch switch that's blocked by unsaved work (the docker offers **Set aside & switch**
when that happens).

**Discard is the one that bites.** It reverts files to their last *committed* version, so
everything since — including work the docker auto-saved for you — is gone, and it won't be
in the reopened document's undo history either. If you want it back later, set it aside
instead; that keeps it.

**Documents reload themselves.** Discarding, setting aside, bringing back, and switching
branches all rewrite `.kra` files on disk. Krita would otherwise carry on showing the copy
it read earlier, and your next Ctrl+S would write that stale art straight back over the new
state — silently undoing the operation. So the docker closes and reopens any open document
whose file it actually changed, which means **the reopened document starts with an empty
undo history**. These actions are also refused outright if a document in the repository
still has unsaved changes — normally impossible, since opening the ⋯ menu means you clicked
into the panel and everything got saved on the way in, but it's the backstop if a save
failed.

## Troubleshooting

- **"Version Control" isn't in the Dockers menu** — the plugin didn't load. Re-check
  step 3, and confirm both `pykrita/kritavc.desktop` and `pykrita/kritavc/*.py` landed
  in the resource folder from step 2 (not one level up or down).
- **"That isn't the kvc tool"** after **Locate kvc…** — the picker checks the file really
  is the CLI before saving it. Pick the `kvc`/`kvc.exe` binary itself, not the folder, and
  the one built in step 1 (not the main `krita-vc`/`krita-vc.exe` app binary, which is a
  different target).
- **"Krita VC tracks .kra documents"** — the active document is a `.png`/`.jpg`/etc. Only
  `.kra` files are versioned; save it as `.kra` inside the tracked folder first.
- **"repository is busy (locked by another process)"** — the desktop app is mid-write, or
  a previous commit/switch didn't exit cleanly and left `.kvc/kvc.lock` behind. Safe to
  delete that file by hand as long as no other `kvc`/desktop-app write is actually in flight.
- **"Save (Ctrl+S) or undo your changes in … first"** — a discard/set-aside/switch would
  rewrite a file you have unsaved edits in. You shouldn't normally see this, since clicking
  into the docker saves; if you do, the auto-save failed (see the next entry).
- **"Couldn't save …"** — Krita refused to write the file. Usually it's read-only (check the
  file and its folder), the disk is full, or something else has it locked. Commit refuses
  rather than capturing a stale version, so fix the file and hit **⟳**.

## Uninstall

Disable "Krita VC" in the Python Plugin Manager, then delete `pykrita/kritavc.desktop`
and `pykrita/kritavc/` from the resource folder.

## Notes

- Commit and Checkpoint are disabled only when nothing is ticked. The docker saves your
  documents for you (see "Using it"), so an unsaved `.kra` is no longer a reason to refuse.
- Saving only touches `.kra` documents inside the repository that Krita reports as modified.
  A clean document is never rewritten, so focusing the panel with nothing to save costs
  nothing, and a `.png` you happen to have open in the same folder is left alone.
- Author name is a plugin-local setting (Krita has no shared login with the desktop
  app); it defaults to `"You"`, matching the desktop app's own fallback.
- The changelist can include palette files (`.gpl`/`.kpl`/`.aco`/`.ase`) sitting next to
  your art, not just `.kra` documents — those are tracked too, even though the docker only
  appears when a `.kra` is active. Untick them if you don't want them in a version.
- `python krita-plugin/test_kvc_client.py` runs the client's self-check (no Krita needed).

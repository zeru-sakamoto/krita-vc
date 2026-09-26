# Krita VC plugin

A "Version Control" docker for Krita. It saves a version, discards changes, sets work aside and
brings it back, and switches or creates branches for the painting you have open, against the same
history the desktop app uses, without leaving Krita.

It runs the `kvc` companion CLI (the `kvc`/`kvc.exe` target in `src-tauri`) instead of reading the
store directly, so the plugin and the desktop app go through exactly the same engine code.

Out of scope on purpose: starting to track a painting, browsing or restoring history, undoing a
version, picking individual layers, merging or deleting branches, backups, and anything remote. All
of that stays in the desktop app.

The user-facing version of this guide is the
[plugin page on the Krita VCS website](https://krita-vc.zeru-sakamoto.codes/plugin).

## Requirements

- Krita with Python scripting enabled. Official Krita builds include it; Settings → Configure Krita →
  Python Plugin Manager should already list some plugins.
- The `kvc` CLI. Every Krita VCS installer puts it next to the app: `kvc.exe` in the install folder
  on Windows, `/usr/bin/kvc` from the `.deb` and `.rpm`, and `Krita-VC.app/Contents/MacOS/kvc` on
  macOS. The AppImage keeps it inside the image, where Krita can't reach it. You can also build it
  yourself (below).
- A painting the desktop app already tracks. The docker can't start tracking one.

## Install

1. Get the plugin: download `kritavc-plugin.zip` from a release (the release workflow zips this
   folder fresh for every release) and unzip it, or use this folder directly.
2. Copy the two plugin items into Krita's `pykrita` resource folder. In Krita choose Settings →
   Manage Resources → Open Resource Folder, then go into the `pykrita/` folder there (create it if
   it's missing; on Windows it's normally `%APPDATA%\krita\pykrita`):
   ```
   pykrita/kritavc.desktop
   pykrita/kritavc/
   ```
3. In Krita, enable it under Settings → Configure Krita → Python Plugin Manager → "Krita VC", then
   restart Krita. Python plugins are only loaded at startup.
4. Open the docker: Settings → Dockers → Version Control.
5. If the docker says "The kvc command-line tool wasn't found", click Locate kvc… and pick the `kvc`
   binary from the list above (on macOS, press Cmd+Shift+G in the file picker and paste
   `/Applications/Krita-VC.app/Contents/MacOS/kvc`).

### Building kvc from source

From `src-tauri/`:

```
cargo build --release --bin kvc
```

The binary lands at `src-tauri/target/release/kvc` (`kvc.exe` on Windows). Point Locate kvc… at it.
On Windows, `scripts/update-installed-kvc.ps1` rebuilds it and copies it over the installed
`%LOCALAPPDATA%\krita-vc\kvc.exe`, so a backend change reaches the plugin without a full
`npm run tauri build` and reinstall.

## Using it

Open a `.kra` the desktop app is tracking. Each painting has its own history, kept by default in the
hidden `.kvc/` folder beside it, so the docker follows the document you're working on, not a folder.
The top row shows the current branch (click it to switch, or choose "New branch…" to start one from
the version you're on), the painting's file name, a refresh button (⟳, or Krita's own refresh icon
when the theme has one) and a ⋮ menu. Below that are a status line (● Unsaved changes, or ✓ Saved),
the Changes list, an Author field and a message box with a Commit button. A message is required.

**You don't need to press Ctrl+S first.** Versions are built from what's on disk, so the docker saves
for you: clicking into the panel saves the tracked painting if it has unsaved changes, and Commit
saves before it records anything. The refresh button does the same on demand, then checks for
changes again. So the Changes list describes your canvas, not your last manual save, and a commit
can't quietly miss the last ten minutes of painting.

Two things follow from that. Krita's own autosave and backup files are never versioned; the engine
only ever looks at the one tracked document. And anything the docker saves is still not a version
until you commit it: saving isn't committing, and Discard (below) throws saved but uncommitted work
away.

**The Changes list** holds one row, the tracked painting and how it changed (Added, Modified or
Deleted), with a tick box that starts ticked. A store tracks exactly one document, so the tick is
effectively all or nothing: Commit, Discard and Set aside act on the ticked row, and nothing ticked
disables Commit. The list, and the ticks, are older than per-document tracking, when a changelist
could hold several files. Picking individual layers is only in the desktop app's Changes panel.

**The ⋮ menu** has the rest, in the same three groups as the desktop app's panel menu: "Discard
checked changes"; "Set aside checked changes…" (with an optional label); then "Bring back latest"
(showing how many are set aside) and "Bring back…" to pick one from a list. Setting work aside parks
it to the side of history and puts the painting back to your last version, so nothing is lost. It's
also the quickest way past a branch switch that's blocked by unsaved work: the docker offers "Set
aside & switch" when that happens. If the painting changed while work was set aside, bringing it back
merges the set-aside layers in on top instead of overwriting anything (see
[`docs/stashes.md`](../docs/stashes.md)).

**Discard is the one that bites.** It reverts the painting to its last committed version, so
everything since, including work the docker saved for you, is gone, and it won't be in the reopened
document's undo history either. The docker asks first. If you might want it back, set it aside
instead.

**Documents reload themselves.** Discarding, setting aside, bringing back and switching branches all
rewrite the `.kra` on disk. Krita would otherwise keep showing the copy it loaded earlier, and your
next Ctrl+S would write that stale art straight back over the new state, silently undoing the
operation. So the docker closes and reopens any open document whose file it actually changed, which
means the reopened document starts with an empty undo history. These actions are also refused
outright while the painting still has unsaved changes. That's normally impossible, because clicking
into the panel to open the menu saved everything on the way in, but it's the backstop if a save
failed.

## Troubleshooting

- **"Version Control" isn't in the Dockers menu.** The plugin didn't load. Check step 3, and confirm
  that both `pykrita/kritavc.desktop` and `pykrita/kritavc/*.py` landed in the folder from step 2,
  not one level up or down.
- **"That isn't the kvc tool."** The picker runs the file and checks that it really is the CLI before
  saving the path. Pick the `kvc` or `kvc.exe` binary itself, not its folder, and not the main
  `krita-vc` app binary, which is a different target.
- **"Krita VC tracks .kra documents."** The active document is a `.png`, `.jpg` or similar. Only
  `.kra` files are versioned: save it as `.kra` and start tracking it in the desktop app.
- **"This artwork isn't version-controlled."** Start tracking it in the desktop app. If the app
  already tracks it and you changed "Where version history is kept" in its Settings, that's a known
  gap: `find_doc` only recognizes a document that has a `.kvc` folder beside it, and a custom store
  root doesn't create one. Use the default location for paintings you want in the docker.
- **"repository is busy (locked by another process)"** The desktop app, or another `kvc`
  invocation, is writing right now. The rest of the message names the painting, the operation and
  how long it has been running. Retry once it finishes. There's nothing to clean up by hand: the lock
  is a real OS-level lock, released the moment the other process's write ends, even if it crashed or
  was force-closed. Don't delete `kvc.lock`.
- **"Save (Ctrl+S) or undo your changes in … first."** A discard, set-aside or switch would rewrite a
  document that has unsaved edits. You shouldn't normally see this, because clicking into the docker
  saves first; if you do, that save failed (see the next entry).
- **"Couldn't save …"** Krita refused to write the file. Usually it's read-only (check the file and
  its folder), the disk is full, or another program has it locked. Commit refuses rather than record
  a stale version, so fix the file and press refresh.
- **"kvc didn't finish within …"** A read took more than 30 seconds, or a write more than 5 minutes.
  Close the desktop app if it's open, then retry.
- **Anything unexpected.** Errors the docker doesn't recognize are shown in its status line, and the
  full traceback is appended to `krita-vc-error.log` in your home folder.

## Uninstall

Disable "Krita VC" in the Python Plugin Manager, then delete `pykrita/kritavc.desktop` and
`pykrita/kritavc/` from the resource folder. Nothing about your paintings or their history lives in
the plugin folder.

## How it works

- `kvc_client.py` is a thin subprocess wrapper. Every call runs `kvc` and parses the one JSON object
  it prints. `KvcError` is the only exception it raises, because its callers are Qt slots and anything
  else escaping one aborts Krita; `vc_docker.py`'s `guard` decorator turns errors into a status-line
  message and logs anything unexpected to `~/krita-vc-error.log`.
- **Finding kvc.** The path saved by Locate kvc… comes first (the picker already verified it), then
  `kvc` on `PATH`, then `krita-vc/kvc(.exe)` under `%LOCALAPPDATA%` and `%PROGRAMFILES%`.
  Auto-discovered binaries are verified before use, once per path: run with no arguments, `kvc`
  prints a usage error whose text starts with `usage: kvc`, and that prefix is the identity check, so
  a stray `kvc` earlier on `PATH` isn't run blindly. That's why the CLI's usage prefix must never
  change.
- **Process details.** Reads time out after 30 seconds and writes after 300. On Windows the call uses
  `CREATE_NO_WINDOW`, or the 1.5-second poll would flash a console window, and output is decoded as
  UTF-8 explicitly, because the Windows default code page mangles non-ASCII paths, branch names and
  messages. Calls block Krita's UI thread by design (see the header of `kvc_client.py`).
- **The poll.** A 1.5-second timer runs `kvc status`, which returns the changes, the branch list and
  the set-aside count in one process, so a tick costs one spawn. It returns early when the docker
  isn't visible.
- **Which document.** `find_doc` accepts the active document if it's a `.kra` with a `.kvc` folder
  beside it and lets `kvc` give the real answer. `is_tracked_document` compares exact paths, not a
  folder prefix, because a neighboring painting has a different history entirely.
- **Memory to disk** (`_save_tracked`): the tracked `.kra`, when Krita reports it modified, is saved
  on focus entering the docker (`QApplication.focusChanged`, not an event filter, because focus lands
  on child widgets and `FocusIn` never reaches the dock), on refresh, and before a commit. Only `.kra`,
  because saving a `.png` can raise an export dialog and hang the UI thread. Commit must call
  `refresh()` between the save and `_selected_paths()`, or it would skip the work it just wrote, and
  `busy` is set during the save because `doc.save()` spins the event loop, which would let the poll
  run `kvc status` on a half-written file.
- **Disk to memory** (`_rebuild_docs`, around switch, discard, set aside and bring back): it refuses
  while the document is unsaved, then closes and reopens the document if its file changed (compared
  by mtime and size, since `switch` doesn't report what it rewrote), and checks again after reopening
  that the file didn't change during the reopen. Without the reopen, the next Ctrl+S would silently
  revert the operation; without the refusal, the reopen would destroy work the engine's dirty-tree
  guard can't see. The reopen runs even when the operation fails: "Set aside & switch" is two `kvc`
  calls, and if the switch fails after the set-aside reverted the file, skipping the reopen would
  let the next Ctrl+S write the set-aside work back over it. `test_kvc_client.py` pins this against
  stub Qt modules.
- The tick state lives in `VcDocker.checked`, not in the list widget, because the poll rebuilds the
  list and would wipe a tick mid-edit; the rebuild is skipped when the list of paths hasn't changed.
- The author name is a plugin-local Krita setting (Krita has no login shared with the desktop app). It
  defaults to `"You"`, matching the desktop app's fallback.
- Standalone palette files (`.gpl`, `.kpl`, `.aco`, `.ase`) aren't tracked. A `.kra`'s own embedded
  palettes are part of the document and show up in the desktop app's diffs.

`python krita-plugin/test_kvc_client.py` runs the client's self-check, with no Krita needed.

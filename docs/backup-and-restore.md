# Backup and restore

There is no automatic backup, sync or cloud storage. The app is local-only, so any safety net has to
be either something the operating system provides or something the artist does on purpose. Two
mechanisms cover the two ways a history can be lost.

## Losing a history inside the app

Deleting a painting's history in the app (`delete_repository`, `Repo::delete` in
[`repo.rs`](../src-tauri/src/repo.rs)) moves the store to the operating system's Recycle Bin or
Trash (the `trash` crate) instead of removing it, so an accidental delete inside the app can be
undone the same way as one in Explorer or Finder. The `.kra` itself is never touched. Under the old
folder model the same action deleted the project folder, art included, which is why the confirm
dialog now asks the artist to type the painting's name: losing a history can't be undone, and the
painting looks perfectly fine afterwards, so a misclick has nothing to announce it. `Repo::delete`
falls back to a permanent `remove_dir_all` only if the move to the trash fails, and it reports which
happened (`Ok(true)` for trashed, `Ok(false)` for permanent) so the UI can warn instead of closing
quietly. It also removes the `.kvc` container once the last store in it is gone.

The dialog (`RemoveRepoModal` in [`TopBar.tsx`](../src/components/shell/TopBar.tsx)) defaults to
the safe choice, "Remove from list only", which forgets the painting in the app and leaves its file
and history on disk.

## Losing everything outside the app

Anything outside the app's control, such as the art folder deleted with `rm -rf` or Shift+Delete, a
failing disk, or another tool corrupting the store, can't be undone after the fact. The only
protection is a backup made before it happens.

### Backing up

Backup is a zip-icon `IconButton` in [`ActivityBar`](../src/components/shell/ActivityBar.tsx)
("Back up artworks…"), directly above the Settings gear. It is deliberately not in Settings, because
it's an action, not a preference. It opens [`BackupModal`](../src/components/shell/BackupModal.tsx),
a multi-select of every tracked painting with all of them ticked (for a safety feature, backing up
more is the better default), and asks for a destination through a native Save dialog. Then
`backupRepositories` → `export_repositories_zip` → `Repo::export_zip_multi` writes one archive:

```text
MANIFEST.json
<dir>/                  one folder per painting
  <name>.kra
  .kvc/<slug>/          that painting's store
```

Each folder is the single-document on-disk shape, so plain extraction still produces tracked
paintings (with the caveat under [Restoring](#restoring)). `skip_in_backup` leaves out `cache/`
(regenerable, and budgeted at 256 MB per store, the largest disposable chunk), `trash/`,
`kvc.lock*`, `worktree.json` (the scan's cache of the saved file's hash) and `*.tmp`.

The `.kra` (already a zip) and the store's objects, packs and chain shards (already zstd) are stored
in the archive as they are; only the JSON state files and logs are deflated. Every file streams from
its handle into the archive rather than being read whole first. Deflating everything at level 6 was
7.2 s of a 7.6 s backup of a 105 MB painting and its store, for an archive 13% smaller.

A backup that hasn't been checked isn't a backup. `MANIFEST.json` is versioned and records, per
painting, its folder, document, original directory, branch and tip commit, plus a timestamp and the
app version. After writing, `export_zip_multi` reopens the archive and checks the entry count and
that the manifest reads back (`verify_zip`) before it reports success, instead of trusting
`zw.finish()` alone. The archive is written to `<dest>.partial` and renamed over the destination
only after that check. The default name is one per day (`krita-backup-<date>.zip`), so a second
backup the same day replaces the first, and writing straight into it used to destroy the good one
before a run that then failed. The export collects failures per painting rather than aborting the
batch, so the modal can say "6 of 7 backed up" and name the one that failed. An archive that would
hold nothing is an error, never a file on disk looking like a backup.

Each painting is zipped under its store lock (`RepoLock`, "backing up"), like every write. The
desktop app blocks its own writes during a backup, but the Krita docker can still commit, switch or
set work aside through `kvc`, and zipping across one of those pairs the painting from one side of
it with the history from the other, which restores as a painting that doesn't match its own
history. The docker gets "busy: backing up" instead, and a painting that's busy when the backup
reaches it is listed as one that couldn't be backed up. Settings → Storage shows "last backed up
N days ago" (`Repository.lastBackupAt`, kept in the frontend's `localStorage`) so a stale backup
doesn't go unnoticed. When the export finishes, the modal stays open as "Backup complete", with the
destination and any paintings that couldn't be backed up.

This replaced an older "back up all" that wrote one zip per painting (`export_repository_zip`,
`backupAllRepositories` and `BackupAllResultModal` are gone).

### Restoring

Restore is "Restore from a backup…" in the artwork switcher (`SwitchArtworkModal`, opened from
`TopBar`). The start screen shown before any painting is tracked points there too, because that's
where a reinstall lands you. It opens [`RestoreModal`](../src/components/shell/RestoreModal.tsx), a
checklist with one row per painting in the archive.

`read_backup_manifest` lists the archive, and `plan_restore` (`Repo::plan_restore`) resolves each
painting to a destination: its original folder if that still exists, otherwise a subfolder of a
folder you pick. Nothing is written yet. The modal states where the history will go, beside the
painting or under the custom store root from Settings → Storage, because with a store root set,
that isn't where plain extraction would have put it. A row whose destination is already occupied
starts unticked. Skip is the safe default there, because Replace swaps out the painting and its
history (both are kept beside the restore, see [below](#where-restored-history-goes), but it should
still be a choice).

### Comparing versions before replacing

When the occupant is a painting that's already tracked, its row also offers "Compare versions",
which opens [`RestoreCompareModal`](../src/components/shell/RestoreCompareModal.tsx). This is what
makes Replace a decision instead of a coin flip: Replace swaps out the history on disk (it's kept
only until a cleanup ages it out), and nothing else in the UI says whether the backup is ahead of it
or behind.

One `compare_restore_versions` call fills two text columns, the backup on the left and this computer
on the right, each listing `Version N`, the note and the age, newest first. Versions that exist on
only one side are tagged, and a summary above says how many are unique to each side ("the backup
has 8, this computer has 5"). Its footer buttons, "Keep mine" and "Use the backup", only write the
row's existing tick in `RestoreModal`; there's no second selection model. It renders as a sibling of
`RestoreModal`, not a child, because `Modal` has no portal (the same reason `CleanupModal` and the
set-aside confirms are siblings of `SettingsModal`).

- It's text only, on purpose. Per-version images would need a `commit_diff` for each version, which
  runs through `run_heavy` and isn't cached, and counts plus notes already answer "which history is
  further along?".
- Each column is numbered within itself, the same positional `total - i` as `versionNumbers` in
  [`friendly.ts`](../src/lib/friendly.ts). So the two columns line up only when one history contains
  the other, which is exactly the case this dialog exists for, and a mismatch reads as "the backup
  is 3 ahead" rather than noise.
- Both sides are scoped to their own branch tip, the same default scope as `list_commits`, so the
  two counts mean the same thing. `Repo::backup_versions` reads `commits.log` straight out of the
  zip, finding it by shape (`<dir>/.kvc/<slug>/commits.log`; the slug is payload, as everywhere else)
  among the names the central directory already holds in memory, rather than opening every entry to
  read its name, then parses it through the shared `parse_commit_log`, scoped to the manifest's
  `tipCommit` with `commit::ancestors`. The comparison opens the local side with `open_light`.
  Nothing is extracted. Unpacking a store on import filters entries by name the same way, so only
  that painting's entries are opened.

### Where restored history goes

Then `import_repository_zip` → `Repo::import_zip` restores the ticked rows, and one rule matters
most: import treats the archive's `.kvc/<slug>/` path as payload, not as a destination. It extracts
the `.kra`, then asks this machine where that painting's history belongs by recomputing
`store_dir_for(&dest)`. With a custom store root set, the history lands under it and no `.kvc/` is
created beside the painting. With none, it lands beside the painting. Writing history anywhere other
than what `store_dir_for` returns would make every later `open`, `is_repo`, scan and commit, and the
`kvc` CLI, look straight past it. Plain extraction under a custom root does exactly that: the
extracted `.kvc/` is a folder `store_dir_for` never looks at, so the painting reads as untracked and
the app offers to start tracking it, creating an empty store next to intact history. Closing that
gap is why the restore command exists.
`import_follows_this_machines_store_root_in_both_directions` in
[`tests/backup_store_root.rs`](../src-tauri/tests/backup_store_root.rs) pins both directions. It is
a test binary of its own because the store root is process-global state, and it redirects the
app-data folder to a temporary one so the suite never touches the developer's real setting.

Import never renames a painting. The filename is baked into `doc.json`, the `index.json` keys, the
chain shard filenames, `Commit.files[].path` and every `kra:{relpath}:…` stream key, so a clash is
resolved by folder, or by Replace or Skip, never by renaming.

The rest of the import is guarded. Entry names are joined through `safe_join` (against zip-slip)
and inflated through `read_entry_capped` (against decompression bombs). A store root that was moved
or renamed is reported as unreachable rather than recreated. And every restored store must pass a
full `check::check_repository` before `addRepositoryPath` puts it back in the list.

Replace keeps what it replaces, and a restore that fails changes nothing (`import_one`):

1. The incoming history is unpacked into `<store>.restoring`, beside where it will live, and its
   `doc.json` checked against the archive. A bad archive fails here, before anything that's already
   there has moved.
2. The current history is renamed to `<store>.replaced-<time>` and the current painting to
   `<name>.replaced-<time>.kra`, which Krita still opens with a double-click. A rename in the same
   folder can't quietly turn into a permanent delete, which is what the old Recycle Bin move did on
   network shares and drives without a Recycle Bin, and it keeps the painting's saved-but-unversioned
   work, which overwriting it used to lose.
3. The restored history and painting take their places. If any step fails, the earlier ones are put
   back.

`ImportResult` reports both kept paths (`replacedArtwork`, `replacedHistory`) and the "Restore
complete" screen shows them. The next "Clean up storage" more than 14 days later removes the old
history (`gc::prune_aged`); the old painting is the artist's to keep or delete.

## Commands

| Command | What it does |
|---|---|
| `export_repositories_zip(paths, dest)` | Write the given paintings, each `.kra` plus its store, into one archive at `dest`: `MANIFEST.json` and one `<dir>/` per painting holding `<name>.kra` and `.kvc/<slug>/`. Skips `cache/`, `trash/`, lock sidecars and temp files. Zips each painting under its store lock, writes `<dest>.partial` and renames it over `dest` only after reopening and verifying it, and returns the paintings that failed instead of aborting the batch. |
| `read_backup_manifest(archive)` | What's inside an archive. Cheap and read-only, so the restore UI can list it before writing anything. |
| `plan_restore(archive, fallbackDir)` | Where each painting would land (its original folder if it still exists, otherwise a subfolder of `fallbackDir`) and what's already there. A proposal only; nothing is written. |
| `compare_restore_versions(archive, dir, destPath)` | Both sides of a clash: the versions in one painting of the archive and the versions already tracked at the destination, newest first, each scoped to its own branch tip. Read-only; the archive's `commits.log` is parsed straight out of the zip. |
| `import_repository_zip(archive, items)` | Restore the chosen paintings. Each history goes where this machine keeps history (`store_dir_for` is recomputed, not copied from the archive). Unpacks beside the destination first, keeps whatever it replaces (`replacedArtwork`, `replacedHistory` in the result), and leaves everything as it was if it fails. Guarded by `safe_join` and `read_entry_capped`, and each restored store is checked with `check_repository`. |
| `delete_repository(path)` | Delete a painting's store, preferring the Recycle Bin, and remove the container if it's now empty. Never touches the `.kra`. Returns `true` if the Recycle Bin was used. |

## Tests

`export_multi_round_trips_two_documents`, `import_without_a_custom_root_lands_beside_the_artwork`,
`import_replaces_an_existing_artwork_in_place`, `backup_skips_the_raster_cache` and
`import_rejects_zip_slip` cover the round trip of two paintings with their history, a restore with
no custom root, Replace, the skipped cache, and an archive whose entry names try to escape the
destination. `import_replace_keeps_what_it_replaces`,
`failed_restore_leaves_the_existing_artwork_and_history_alone`, `backup_takes_each_artworks_lock`
and `failed_backup_leaves_the_previous_one_intact` pin the kept copies, the untouched destination
after a bad archive, the lock, and the `.partial` write; `a_missing_store_root_is_reported_not_recreated`
(`tests/store_root_missing.rs`, its own binary for the same reason as `backup_store_root.rs`) pins
the store-root rule for tracking and restoring.
`verify_zip_rejects_entry_count_mismatch` and `verify_zip_rejects_missing_manifest`
unit-test the check against hand-built bad archives. See also
[data-integrity.md](data-integrity.md#6-working-tree-safety) for how backup and restore fit into the
wider set of integrity measures.

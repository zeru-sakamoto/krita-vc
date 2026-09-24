# Data-integrity hardening: v1.2.0 and v1.3.0

Dates: 2026-08-13 to 2026-08-20. Commits: `eb87171` to `5a1e677`.

The era opens with a hotfix, `eb87171` ("Hotfix for Critical Data Integrity Gaps"), which also adds
the read-only repository check (`src-tauri/src/check.rs` first appears here). File safeguards
follow (`9601c61`, "Implemented Quality of Life File Safe Guards"), then the 1.2.0 version bump
(`ccd9e3b`).

According to `content/RELEASE_NOTES.md`, v1.2.0 ships three things you can see. There is "Check for
problems", a read-only health check with an optional deeper pass that reads back and verifies every
stored version. Backups are verified: each one is reopened and checked right after it is written,
and Settings shows how long ago the last one was made, since without cloud sync a backup is the
only copy that survives a failing disk. And "Clean up storage" no longer deletes outright: what it
reclaims goes to a hidden trash folder first and is only removed for good after 14 days.

Underneath those, a wider pass makes the store survive crashes and power cuts. Every write goes to
a temporary file, is flushed to disk, and only then replaces the real file. The restore paths
re-verify data as they read it back instead of trusting it. The small state files keep a copy of
their previous version to fall back on if the current one turns out damaged.

v1.3.0 follows a week later, and here the version-bump commit carries the code itself: `5a1e677`
("Bumped Version to 1.3.0") adds `src-tauri/src/diskspace.rs` and `src-tauri/src/ops_log.rs`.
Its changes, per the commit message and the release notes:

- a free disk space check before commits, restores and branch operations, which refuses up front
  with a clear message instead of failing halfway through a write (it only runs on Windows, as the
  check is implemented with a Windows API);
- a small audit log for undo, discard, cleanup and branch delete, kept for support and recovery
  (nothing in the app reads it);
- a length field in new pack files, so the health check catches a truncated pack immediately
  instead of it turning up later as a missing object somewhere else;
- in the Krita docker, a re-check right after a document is reopened, so a file that changes again
  during that window produces a clear error rather than a quietly stale copy.

See also: [`data-integrity.md`](../data-integrity.md) for the current mechanics (the lock, the
generation counter, atomic and fsynced writes, the check and scrub model, cleanup's trash folder),
and the [v1.2.0](https://github.com/zeru-sakamoto/krita-vc/releases/tag/app-v1.2.0) and
[v1.3.0](https://github.com/zeru-sakamoto/krita-vc/releases/tag/app-v1.3.0) release notes in full.

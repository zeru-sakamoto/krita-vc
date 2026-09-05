# Data-integrity hardening: v1.2.0 and v1.3.0

**Timeframe:** 2026-08-13 – 2026-08-20 · **Commits:** `eb87171` … `5a1e677`

A dedicated hotfix opens this era: `eb87171` "Hotfix for Critical Data Integrity Gaps," followed
by quality-of-life file safeguards (`9601c61`) and version 1.2.0 (`ccd9e3b`). Per
`content/RELEASE_NOTES.md`, v1.2.0 ships a read-only "Check for problems" health check (with an
optional deep byte-verify "scrub" pass), verified backups (a backup is now reopened and checked
right after writing, plus a "last backed up N days ago" reminder; there's no cloud sync, so a
backup is the only copy that survives a disk failure), and trash-based rather than immediate
"Clean up storage" deletion (a 14-day grace period). Alongside those user-facing features, a
broader crash/power-loss survivability pass lands: every write goes temp-then-fsync-then-rename,
restore paths re-verify data as it's read back rather than trusting it, and internal state files
keep a previous-generation `.bak` to recover from if the current one turns out damaged.

Version 1.3.0 follows a week later (`5a1e677`): a preflight free-disk-space check before commits,
restores, and branch operations (refusing up front with a clear message instead of failing
partway through a write), a lightweight audit-trail ops log for undo/discard/cleanup/branch-delete
(support/recovery use only; nothing in the app surfaces it directly), truncated/corrupted pack
files now caught immediately by the health check instead of surfacing later as a missing piece
elsewhere, and the Krita docker re-checking a document right after reopening it so a same-window
disk change surfaces as a clear error instead of a silent stale copy.

**See also:** [`data-integrity.md`](../data-integrity.md) for the full current mechanics (repo
lock, generation counters, atomic+fsynced writes, the check/scrub model, GC's trash-not-delete
behavior); [`content/RELEASE_NOTES.md`](../../content/RELEASE_NOTES.md) for the v1.2.0/v1.3.0
changelog entries verbatim.

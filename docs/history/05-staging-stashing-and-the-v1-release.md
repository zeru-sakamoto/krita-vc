# Staging, stashing, and the run-up to v1.0

**Timeframe:** 2026-07-13 – 2026-07-18 · **Commits:** `8af1657` … `3fbea32`

**Per-file staging and discard.** `8af1657` adds per-file staging (partial commits: save some
changed files but not others) and discard-changes support, the first time a commit stops being
all-or-nothing for the whole working tree.

**The Performance tab.** Two "Developed Performance Tracking for Dev" commits land from what look
like separate branches later merged back together (`e38ab90`, `1ba8b08`, merged via
`9d24b8b`/`09d1853`/`f38d928`). Despite the "for Dev" framing, this is where the user-facing
**Performance tab** is born (`PerformancePanel.tsx` and `lib/perf.ts` both first appear in
`1ba8b08`). The v2.1-beta release notes describe it as showing what the delta store saves you:
total storage saved with a percentage, plus each version's stored size against what a full copy
would have cost. It's the feature that makes the tile-delta bet from
[02](02-the-custom-tracking-engine.md) legible to the person using the app.

**A flurry of housekeeping and a version-numbering fork.** A run of "Updated Application Details"
and version-bump commits follows (`ec8e42b`, `a10ed01`, `9116e27`, `8f049bc`), the first
`RELEASE_NOTES.md` is added (`6e87683`), a loading-screen fix bumps the version to 2.1 (`eddb8f8`),
the MIT license lands (`3a22621`), and release notes are written and then rewritten for "V2.1-beta"
(`9a81ac8`, `21a46d4`).

The version numbers stop making sense around here. `content/RELEASE_NOTES.md`'s sequence today
reads v1.0-beta → v2.0-beta → v2.1-beta → **v3.0-beta** → v1.0.0-release → v1.1.0 → ... → v2.0.0,
with the "v3.0-beta" entry sitting chronologically *between* v2.1-beta and v1.0.0-release. The
project renamed its versioning scheme from ad-hoc `vN-beta` tags to semver partway through, so
v3.0-beta reads as the last beta tag before that rename rather than a fourth major beta line.

**The stashing backend.** `078d8f7` "Developed Stashing & Popping Backend" is the plumbing behind
what `v3.0-beta`'s release notes call "Set aside": parking uncommitted working-tree changes off
to the side and bringing them back later, without committing them.

**The plugin/safety overhaul and v1.0.0's hardening.** `af689cd` combines a Krita-plugin, Settings,
and repository-safety overhaul, and is also where **`.kra` layer merging** enters the engine
(`src-tauri/src/merge.rs` first appears here), the machinery that lets a stash pop onto an edited
file fold in only the layers the set-aside version actually changed instead of hard-refusing. It
matches v3.0-beta's release-note items (per-file ticks in the
docker, the docker saving documents before every commit, documents reloading themselves after a
disk-changing operation, autosave files no longer being tracked, one-click backup, repositories
refusing to nest). The first-launch spotlight tour ships next (`cc7ca73`), then a process-lock
hotfix using FL2 flock (`749eee0`), and finally a security-fixes commit explicitly labeled **"for
v1.0.0"** (`3fbea32`), matching the release notes' v1.0.0-release entry: non-English text handling
on Windows, crash-hardening against malformed/malicious `.kra` and palette files, the Krita docker
verifying its `kvc` helper binary before running it, and tightened production CSP/permissions.

**See also:** [`version-control.md`](../version-control.md#stashes--setting-work-aside) for how
stashing works today; [`data-integrity.md`](../data-integrity.md) for the process-lock model;
[`content/RELEASE_NOTES.md`](../../content/RELEASE_NOTES.md) for the full v1.0.0-release changelog.

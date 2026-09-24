# Staging, stashing, and the run-up to v1.0

Dates: 2026-07-13 to 2026-07-18. Commits: `8af1657` to `3fbea32`.

## Per-file staging and discard

`8af1657` adds per-file staging (a partial commit saves some changed files and leaves the rest) and
a way to discard changes. It is the first time a commit is not all-or-nothing for the whole working
tree. The idea comes back a month and a half later in a different shape, as layer-subset staging
([12](12-layer-subset-staging.md)), after the per-document rewrite leaves only one file to stage.

## The Performance tab

The same commit, "Developed Performance Tracking for Dev", appears twice with the same timestamp
(`e38ab90`, `1ba8b08`). They are copies of one piece of work on the local and remote
`test/performance-metrics` branch, and `9d24b8b` merges them; `09d1853` then brings in `main`, and
`f38d928` merges two edits to the site copy made in parallel (`af0ca90`, `929156c`).
Despite the "for Dev" in the message, this is where the user-facing Performance tab starts
(`PerformancePanel.tsx` and `lib/perf.ts` both first appear in `1ba8b08`). The v2.1-beta release
notes describe it as showing what the delta store saves you: total storage saved as a percentage,
plus each version's stored size next to what a full copy would have cost. It is the feature that
makes the tile-delta bet from [02](02-the-custom-tracking-engine.md) visible to the person using
the app.

## Housekeeping, and how the version numbers worked

A run of "Updated Application Details" and version commits follows (`ec8e42b`, `a10ed01`,
`9116e27`, `8f049bc`). The first `RELEASE_NOTES.md` is added (`6e87683`). `eddb8f8` fixes the
loading screen and bumps the app to 0.2.1, `3a22621` adds the MIT license (replaced by GPL-3.0 two
months later, see [13](13-welcome-license-and-v2.1.md)), and the release notes for "V2.1-beta" are
written and then rewritten (`9a81ac8`, `21a46d4`).

The release names look out of order, and `content/RELEASE_NOTES.md` still lists them that way:
v1.0-beta, v2.0-beta, v2.1-beta, v3.0-beta, then v1.0.0. The app's own version number never went
backwards, though. The four pre-releases were app versions 0.1.0, 0.2.0, 0.2.1 and 0.3.0, published
under the tags `beta`, `v2.0-beta`, `v2.1-beta` and `v3.0-beta`. From 1.0.0 on, the tag and the app
version match (`v1.0.0`, then `app-v1.1.0` and later, the name the release workflow generates). The
[history index](README.md#releases) maps every tag to its commit.

## The stashing backend

`078d8f7` ("Developed Stashing & Popping Backend") is the plumbing behind what the v3.0-beta release
notes call "Set aside": parking uncommitted changes to the side of history and bringing them back
later, without committing them.

## The plugin and safety overhaul, and v1.0.0

`af689cd` combines an overhaul of the Krita plugin, Settings and repository safety, and it is the
commit tagged `v3.0-beta` (app 0.3.0). It is also where `.kra` layer merging enters the engine
(`src-tauri/src/merge.rs` first appears here). That code lets bringing back set-aside work onto an
edited painting fold in only the layers the set-aside version changed, instead of refusing. The
commit matches the v3.0-beta release notes item for item: per-file ticks in the docker, the docker
saving documents before every commit, documents reopening themselves after an operation rewrites
them, Krita's autosave files no longer being tracked, one-click backup, and repositories that refuse
to nest. It also removes the docker's "⚡ Checkpoint" button from
[04](04-settings-theming-palettes-and-the-first-plugin.md), which the release notes don't mention.

The first-launch spotlight tour ships next (`cc7ca73`). Then comes `749eee0`, titled "Process Lock
Hotfix - FL2 FLOCK". The old lock was a create-new marker file whose code comment named "fs2 flock"
as the upgrade to reach for if a stale lock ever caused trouble. The hotfix replaced it with an
OS-level lock from the standard library (`File::try_lock`, which is `LockFileEx` on Windows and
`flock` elsewhere), so a crashed process can no longer leave a repository stuck as busy. The same
commit bumps the app to 1.0.0, and the `v1.0.0` tag points at it.

Last is `3fbea32`, "Security Fixes for v1.0.0". Its changes match the "Security & reliability"
section that the local release notes (`content/RELEASE_NOTES.md`) list under v1.0.0: non-English
text on Windows, hardening against malformed or malicious `.kra` and palette files, the docker
verifying its `kvc` helper before running it, and a tighter production content-security policy. It
landed three hours after the commit the `v1.0.0` tag points at, though, so that tag doesn't contain
it, and the v1.0.0 release as published on GitHub has no security section (only Highlights and
Fixes). The first tag that contains the fixes is `app-v1.1.0`.

See also: [`stashes.md`](../stashes.md) for how setting work aside works today;
[`data-integrity.md`](../data-integrity.md) for the lock model; the
[v1.0.0 release](https://github.com/zeru-sakamoto/krita-vc/releases/tag/v1.0.0) for what was
published.

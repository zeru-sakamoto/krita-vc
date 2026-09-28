# Project history

The docs in the folder above describe how the app works today. These describe how it got there: the
decisions, rewrites and course corrections behind the current architecture, drawn from the full
commit history (`git log --all --reverse`, 96 commits from 2026-06-18 to 2026-09-26) and
cross-checked against [`CLAUDE.md`](../../CLAUDE.md) and the release notes. The notes are
published on [GitHub Releases](https://github.com/zeru-sakamoto/krita-vc/releases), written from
`content/RELEASE_NOTES.md`, a local file that isn't in git (`content/` is gitignored). The release
notes remain the changelog of record, version by version. These files are the story between their
entries.

Read them in order for the whole story, or jump to an era:

| Era | Dates | Commits | What happened |
|---|---|---|---|
| [01. Origins and the git2 prototype](01-origins-and-the-git2-prototype.md) | 06-18 to 06-29 | `423f366` to `5b9e482` | A bare Tauri scaffold, then a `git2` (libgit2) prototype that was dropped. |
| [02. The custom tracking engine](02-the-custom-tracking-engine.md) | 06-29 to 07-02 | `1763886` to `4144dac` | The from-scratch `.kra` tile and layer engine that replaced it. |
| [03. Branching and early performance](03-branching-and-early-performance.md) | 07-03 to 07-06 | `433f292` to `998d8c3` | Local branching and merging, then three quick passes on switch latency. |
| [04. Settings, theming, palettes and the first plugin](04-settings-theming-palettes-and-the-first-plugin.md) | 07-07 to 07-13 | `20599a6` to `5226db4` | Settings, themes, the custom title bar, the first Krita docker, palette tracking. |
| [05. Staging, stashing and the v1.0 release](05-staging-stashing-and-the-v1-release.md) | 07-13 to 07-18 | `8af1657` to `3fbea32` | Per-file staging, set aside, the plugin and safety overhaul, the tour, v1.0.0. |
| [06. CI/CD pipeline](06-ci-cd-pipeline.md) | 07-19 to 08-02 | `c2031d9` to `2bef324` | The GitHub Actions release workflow and its first fixes. |
| [07. CPU headroom (v1.1.0)](07-cpu-headroom-v1.1.md) | 08-01 | `e6cae2e`, `c16a5e4` | The engine stops starving Krita of CPU during a commit or diff. |
| [08. Data-integrity hardening (v1.2.0, v1.3.0)](08-data-integrity-v1.2-and-v1.3.md) | 08-13 to 08-20 | `eb87171` to `5a1e677` | Health checks, verified backups, cleanup via trash, crash survival. |
| [09. The Bento redesign](09-the-bento-redesign.md) | 08-23 to 08-30 | `558fd6a` to `8250fb1` | From a flat VS Code look to tactile "Bento Box Neumorphism", enforced app-wide. |
| [10. Version Map and the per-document rewrite (v2.0.0)](10-version-map-and-the-per-document-rewrite.md) | 08-26 to 08-27 | `fb65ca2` to `d4d821d` | The React Flow Version Map, then the breaking one-document, one-history rewrite. |
| [11. Backup and restore overhaul](11-backup-restore-overhaul.md) | 08-28 to 08-29 | `655b992` to `af73620` | Several artworks in one archive, and restore with a version comparison. |
| [12. Layer-subset staging](12-layer-subset-staging.md) | 09-01 to 09-03 | `c686231` to `e5b02ea` | Saving only the ticked layers, made three times faster, plus a doc audit. |
| [13. The welcome screen, a license change, and v2.1.0](13-welcome-license-and-v2.1.md) | 09-05 to 09-13 | `5fccb88` to `b324391` | This series, the first-launch welcome, MIT to GPL-3.0, and the 2.1.0 release. |
| [14. The September audit and its fixes](14-the-september-audit-and-its-fixes.md) | 09-25 to 09-26 | `38bf63a` to `TODO` | The docs reorganized by feature, an audit of v2.1.0, and fixes for its 18 stability and 16 performance findings. |

All dates are 2026. Some eras overlap instead of following one another. Eras 09, 10 and 11 were
worked on in the same week at the end of August, which is why v2.0.0 ships the redesign, the
Version Map, the per-document rewrite and the new backup together. Era 06's CI work also carries on
past era 07's single-day CPU fix, because pipeline commits kept landing between engine work. Each
chapter points out its overlaps instead of forcing a strict order.

## Releases

The app's version number only ever went up, but the pre-1.0 release tags were named differently
from the version inside the app, which makes `RELEASE_NOTES.md` look out of order. This table maps
each release to its tag, its commit and the version the app reported.

| Release | Tag | Commit | App version | Published |
|---|---|---|---|---|
| KVC v1.0-beta | `beta` | `e1dfc6d` | 0.1.0 | 2026-07-09 |
| KVC v2.0-beta | `v2.0-beta` | `8f049bc` | 0.2.0 | 2026-07-13 |
| KVC v2.1-beta | `v2.1-beta` | `21a46d4` | 0.2.1 | 2026-07-14 |
| Krita VC v3.0-beta | `v3.0-beta` | `af689cd` | 0.3.0 | 2026-07-17 |
| Krita VC v1.0.0 release | `v1.0.0` | `749eee0` | 1.0.0 | 2026-07-19 |
| Krita VC v1.1.0 | `app-v1.1.0` | `c16a5e4` | 1.1.0 | 2026-08-01 |
| Krita VC v1.2.0 | `app-v1.2.0` | `ccd9e3b` | 1.2.0 | 2026-08-13 |
| Krita VC v1.3.0 | `app-v1.3.0` | `5a1e677` | 1.3.0 | 2026-08-22 |
| Krita VC v2.0.0 | `app-v2.0.0` | `8250fb1` | 2.0.0 | 2026-08-30 |
| Krita VC v2.1.0 | `app-v2.1.0` | `b324391` | 2.1.0 | 2026-09-13 |

"Published" is the date GitHub shows for the release, in UTC. Two quirks are covered in
[05](05-staging-stashing-and-the-v1-release.md): the four betas are app versions 0.1.0 to 0.3.0,
and the `v1.0.0` tag sits three hours before the "Security Fixes for v1.0.0" commit, so that tag
doesn't contain it.

## Coverage

Every commit in the repository is cited by at least one chapter. Every hash, date and quoted commit
message in these files was checked against `git log --all` on 2026-09-26.

See also: [`../README.md`](../README.md) for the docs on the current architecture that these files
go with.

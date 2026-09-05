# Project history

The docs above this folder describe **how the app works today**. These describe **how it got
there**: the sequence of decisions, rewrites, and course-corrections behind the current
architecture, drawn from the project's full commit history (`git log --all --reverse`, 89 commits,
2026-06-18 → 2026-09-03) and cross-referenced against [`CLAUDE.md`](../../CLAUDE.md) and
[`content/RELEASE_NOTES.md`](../../content/RELEASE_NOTES.md). `RELEASE_NOTES.md` stays the
authoritative version-by-version changelog; these files are the connective narrative between its
entries, not a replacement for it.

Read in order for the full story, or jump to whichever era you're curious about:

| Era | Timeframe | Commits | What happened |
|---|---|---|---|
| [01. Origins & the git2 prototype](01-origins-and-the-git2-prototype.md) | 2026-06-18–06-29 | `423f366`…`5b9e482` | Bare Tauri scaffold, then an abandoned `git2`/libgit2-based prototype. |
| [02. The custom tracking engine](02-the-custom-tracking-engine.md) | 2026-06-29–07-02 | `1763886`…`4144dac` | The from-scratch `.kra` tile/layer engine that replaced git2. |
| [03. Branching & early performance](03-branching-and-early-performance.md) | 2026-07-03–07-06 | `433f292`…`998d8c3` | Local branch/merge lands, then three rapid latency-fix passes. |
| [04. Settings, theming, palettes & the first plugin](04-settings-theming-palettes-and-the-first-plugin.md) | 2026-07-07–07-13 | `20599a6`…`5226db4` | Settings modal, theming, custom title bar, the first Krita docker, palette tracking. |
| [05. Staging, stashing & the v1.0 release](05-staging-stashing-and-the-v1-release.md) | 2026-07-13–07-18 | `8af1657`…`3fbea32` | Per-file staging, the stash backend, plugin/safety overhaul, tour, v1.0.0 security hardening. |
| [06. CI/CD pipeline](06-ci-cd-pipeline.md) | 2026-07-19–08-02 | `c2031d9`…`2bef324` | GitHub Actions release workflow and cross-platform build fixes. |
| [07. CPU headroom (v1.1.0)](07-cpu-headroom-v1.1.md) | 2026-08-01 | `e6cae2e`, `c16a5e4` | The engine stopped starving Krita for CPU during a commit or diff. |
| [08. Data-integrity hardening (v1.2.0, v1.3.0)](08-data-integrity-v1.2-and-v1.3.md) | 2026-08-13–08-20 | `eb87171`…`5a1e677` | Health checks, verified backups, trash-based cleanup, crash-survivability pass. |
| [09. The Bento redesign](09-the-bento-redesign.md) | 2026-08-23–08-30 | `558fd6a`…`470ccff` | Flat/VS Code style → tactile "Bento Box Neumorphism," enforced app-wide. |
| [10. Version Map & the per-document rewrite (v2.0.0)](10-version-map-and-the-per-document-rewrite.md) | 2026-08-26–08-27 | `fb65ca2`…`d4d821d` | The React-Flow Version Map, then the breaking one-document-one-history rewrite. |
| [11. Backup & restore overhaul](11-backup-restore-overhaul.md) | 2026-08-28–08-29 | `2d3d59e`…`af73620` | Multi-artwork single-archive backup, restore-with-compare. |
| [12. Layer-subset staging](12-layer-subset-staging.md) | 2026-09-01–09-03 | `c686231`…`e5b02ea` | Commit only selected layers of a `.kra`; a 3.3x speedup and a doc audit. |

Some eras overlap in date rather than sitting in strict sequence. Eras 09, 10, and 11 were all
worked on within the same roughly one-week window in late August, which is why
`content/RELEASE_NOTES.md` bundles the redesign, the Version Map, the per-document rewrite, and
multi-artwork backup into one v2.0.0 release; and era 06's CI work trails on past era 07's
single-day CPU fix, since release-pipeline commits kept landing alongside engine work. Each file
notes this where relevant rather than forcing a false linear order.

Coverage: every commit in the repository's history is cited by one of these files except two
`SITE_CONTENT.md` copy updates (`af0ca90`, `929156c`, both 2026-07-15), which changed no code and
belong to the project's marketing site rather than the app.

**See also:** [`../README.md`](../README.md) for the current-architecture docs these files
complement.

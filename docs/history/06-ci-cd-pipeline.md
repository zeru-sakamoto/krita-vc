# CI/CD: GitHub Actions and Copilot-assisted build fixes

Dates: 2026-07-19 to 2026-08-02. Commits: `c2031d9` to `2bef324`.

This era is infrastructure only, with no change to how the engine behaves. The goal is getting CI
to reliably produce the release artifacts on every platform: the desktop app, the headless `kvc`
CLI that ships next to it, and the Krita plugin zip.

`c2031d9` creates the GitHub Actions release workflow. Three pull requests opened by GitHub Copilot
follow, each starting with an empty "Initial plan" commit (`8eecfde`, `b2068a7`, `cf3a6b1`) and
merged the same evening (`aff7154`, `4afec6b`, `869eea1`):

1. PR #1 fixes a build failure. The macOS job builds a universal app, so it needs a universal
   (arm64 + x86_64) build of the `kvc` CLI to bundle next to it (`4673e18`).
2. PR #2 grants the release workflow's token write access (`5bc7fd8`).
3. PR #3 fixes the token permissions again (`cb3ba75`).

So the workflow broke in two different ways: one platform couldn't build, and then the job couldn't
publish its own release, which took two attempts to fix. A `.gitignore` update (`c575675`) closes
the day.

Two weeks later, right after v1.1.0 ([07](07-cpu-headroom-v1.1.md)), `0596d4a` updates the
hand-built plugin zip that had been committed to the repo since `3fbea32`, and `2bef324` deletes
it. From then on the release job zips the plugin fresh from source and attaches it to each release,
and a new `notify-site.yml` workflow tells the marketing site's repository when a release is
published so it can pull the new installers.

One later fix belongs with this pipeline too: `8250fb1` (2026-08-30) moves the release workflow to
`actions/checkout@v5`, `actions/setup-node@v5` and Node 24, and it is the commit `app-v2.0.0` was
tagged at ([09](09-the-bento-redesign.md)).

See also: `.github/workflows/` for the current CI configuration, and the Commands section of
[`CLAUDE.md`](../../CLAUDE.md) for the local equivalents of what CI runs (`npm run tauri build`,
`cargo build --release --bin kvc`).

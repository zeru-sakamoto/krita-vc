# CI/CD: GitHub Actions and Copilot-assisted build fixes

**Timeframe:** 2026-07-19 – 2026-08-02 · **Commits:** `c2031d9` … `2bef324`

Pure infrastructure, no engine behavior: getting CI to reliably produce release artifacts (the
desktop app, the headless `kvc` CLI sidecar, and the Krita plugin zip) across platforms.

A GitHub Actions release-build workflow is created (`c2031d9`), then three GitHub-Copilot-authored
"Initial plan" commits (`8eecfde`, `b2068a7`, `cf3a6b1`) and their fixes are merged through PRs
#1–#3 (`aff7154`, `4afec6b`, `869eea1`). The concrete fixes: a universal (arm64 + x86_64) macOS
build of the `kvc` sidecar (`4673e18`), and two rounds of release-workflow token-permission fixes
(`5bc7fd8` "grant release workflow token write access," `cb3ba75` "Fix release workflow token
permissions"). Needing three follow-up PRs to resolve it is the signature of a "the workflow ran
but couldn't publish its own output" failure. A `.gitignore` update (`c575675`) and
later a step to notify the project site and attach the plugin zip to releases (`2bef324`,
alongside `0596d4a`) round out the pipeline.

**See also:** `.github/workflows/` for the current CI configuration; `CLAUDE.md`'s Commands section
for the local equivalents of what CI runs (`npm run tauri build`, `cargo build --release --bin
kvc`).

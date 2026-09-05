# Origins, and the abandoned git2 prototype

**Timeframe:** 2026-06-18 – 2026-06-29 · **Commits:** `423f366` … `5b9e482`

The project starts as a bare Tauri 2 + React scaffold (`423f366`, `9ec466d`), then gets a UI shell
wired to hand-written mock data (`54908fe`) so the frontend could be built before any backend
existed, plus Playwright tooling to screenshot it (`0e89698`).

That mock-data commit is also where **Artist Mode** is born (`src/lib/artistMode.tsx` first appears
in `54908fe`): the global toggle that swaps technical strings for plain-language labels
("Version 5" instead of a hash, asset names instead of file paths). It predates every line of the
real engine. The decision that the audience is artists rather than developers was made before
there was anything to version-control.

The first real attempt at version control, on the `feature/repository-manager` branch, reached for
the obvious tool: `git2`, the Rust binding to libgit2. `src-tauri/Cargo.toml` at the time pinned
`git2 = { version = "0.19", features = ["vendored-libgit2"] }`, and the branch wired real commands
(`lib.rs` gained 753 lines) through to `AppShell`, `Sidebar`, `TopBar`, `BranchesPanel`, and
`ChangesPanel`. This wasn't a spike. It was most of a working repository manager.

It was dropped. The branch survives in history only as two `git stash`-shaped commits
(`432b730` "untracked files on feature/repository-manager...", `e2f86dd` "index on
feature/repository-manager...") folded into one merge commit whose message says plainly what
happened: `5b9e482` **"On feature/repository-manager: git2 crate solution stash."** The very next
commit, `1763886` "Developed File Tracking System for .kra files," is where the from-scratch
engine that the rest of this project is built on begins.

The reasoning isn't spelled out in the commit message, but `docs/version-control.md`'s opening line
states it directly: *"`git2` was evaluated and dropped."* It fits the shape of the problem git was
never designed for. Git (and libgit2) diffs and stores whole blobs, repacking them wholesale when
they change. A `.kra` file is a zip archive of many large, mostly-unchanged raster tiles, so
treating the whole archive as one opaque blob on every save means every commit re-stores the entire
document, and history bloats fast on exactly the kind of large binary files Krita produces. That's
the gap the custom engine (next: [02](02-the-custom-tracking-engine.md)) was built to close by
decomposing `.kra` archives down to the individual 64×64 tile, so a commit only stores what
actually changed.

**See also:** [`version-control.md`](../version-control.md) for how the resulting tile-delta
engine works today.

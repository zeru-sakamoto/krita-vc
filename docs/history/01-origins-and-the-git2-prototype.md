# Origins, and the abandoned git2 prototype

Dates: 2026-06-18 to 2026-06-29. Commits: `423f366` to `5b9e482`.

The project starts as a bare Tauri 2 + React scaffold (`423f366`, `9ec466d`). A UI shell wired to
hand-written mock data follows (`54908fe`), so the frontend could be built before any backend
existed, and then Playwright tooling to take screenshots of it (`0e89698`).

Artist Mode is born in that mock-data commit: `src/lib/artistMode.tsx` first appears in `54908fe`.
It is the global toggle that swaps technical strings for plain-language labels ("Version 5" instead
of a hash, an asset name instead of a file path). So the decision that the audience is artists, not
developers, was made before a single line of the real engine existed.

The first real attempt at version control lived on a `feature/repository-manager` branch and
reached for the obvious tool, `git2`, the Rust binding to libgit2. `src-tauri/Cargo.toml` on that
branch pins `git2 = { version = "0.19", features = ["vendored-libgit2"] }`, and the branch wires
real commands through to `AppShell`, `Sidebar`, `TopBar`, `BranchesPanel` and `ChangesPanel`
(`lib.rs` alone gains 753 lines). It was most of a working repository manager, not a spike.

It was dropped anyway. The branch survives only as the three commits `git stash` creates: `432b730`
("untracked files on feature/repository-manager..."), `e2f86dd` ("index on
feature/repository-manager...") and the stash commit itself, `5b9e482`, whose message says what
happened: "On feature/repository-manager: git2 crate solution stash". The next commit, `1763886`
("Developed File Tracking System for .kra files"), starts the from-scratch engine the rest of the
project is built on.

No commit message explains the decision. [`version-control.md`](../version-control.md) records only
the outcome in its opening lines ("`git2` was evaluated and dropped"). The likely reason is the file
format. A `.kra` is a zip archive of compressed layer tiles, and git treats it as one opaque binary
blob. A small brush stroke rewrites compressed bytes all through the archive, so git's delta
compression finds little to share between versions and each commit ends up storing close to the
whole document again. The custom engine ([02](02-the-custom-tracking-engine.md)) avoids that by
taking the archive apart down to individual 64×64 tiles, so a commit only stores the tiles that
changed.

See also: [`version-control.md`](../version-control.md) for how the tile-delta engine works today.

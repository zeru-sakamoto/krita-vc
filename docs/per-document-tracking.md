# Per-Document Tracking (design note — not implemented)

**Status: proposal.** Nothing in this document is built. It records the shape of the change so a
future session doesn't have to re-derive it.

## The idea

Today a *repository* is a folder, and one history covers every tracked file in it. The proposal
is to make **one `.kra` document = one history**: the artist opens `painting.kra` and sees that
painting's versions, its own branches, its own set-asides — no folder-level mental model, no
git-shaped "which files go in this commit" step.

Motivation: the audience is artists who have never used git. A repo-with-staging is the single
biggest conceptual tax the app charges, and it buys almost nothing here, because the tracked set
is already nearly always *one document plus palettes it embeds anyway* (a `.kra`'s document
palettes already surface as `Palette` diff entries off the `.kra` itself —
`commands::kra_palette_dtos`).

## Two ways to get there

**Option A — per-file as a lens.** Leave the store alone, filter the UI to one path. Commits stay
repo-wide; `list_commits` gets an optional `path` and the frontend shows only commits whose
`files` mention it. Cheap (a few frontend files, one Rust arg), zero CLI/plugin change.
**But branches and stashes stay repo-wide** — switching a branch rewrites every tracked file, not
just the open document, and no amount of filtering hides that.

**Option B — a real per-document store.** What this document specifies. Chosen because the
branch/stash seam in A is exactly the part that confuses the target user.

## Design

### Key decision: split the history, share the object store

`.kvc/` stays **one folder at the project root**. Only the *history state* becomes per-document.

```text
.kvc/
  config.json     unchanged — engine config is per-project, not per-document
  kvc.lock        unchanged — one repo-wide advisory lock (see "Locking" below)
  objects/        unchanged, SHARED across documents (loose + packs)
  cache/          unchanged, SHARED (raster cache, content-addressed)
  chains/         unchanged — already sharded per tracked file, so already per-document
  docs/
    <docId>/
      doc.json      { relpath, displayName, createdAt }
      index.json    the one TrackedFile entry for this document
      commits.log   append-only JSON-lines, this document's history only
      branches.json this document's branches + current + generation counter
      stashes.json  this document's shelf
```

Sharing `objects/` is what makes B affordable. Tile dedup, packs, the `kvcimg://` raster cache and
the `RepoLock` all keep working unchanged. Sharding objects per document would throw away
cross-document tile dedup and buy nothing.

`chains/` needs no change at all — stream keys already embed the tracked file's relpath
(`kra:{rel}:tile:{e}:{x},{y}`) and `shard_of` already buckets one file's keys together.

### `Repo` becomes two types

`repo::Repo` currently carries `index`, `commits`, `branches`, `stashes` — all four are exactly
the per-document state. The split:

- **`Project`** — root, `config`, `chains`, `packs`, the lock, `objects/`, `cache/`.
- **`Document`** — `doc_id`, `relpath`, `index` (one entry), `commits`, `branches`, `stashes`,
  plus a borrow of its `Project`.

Everything in `commit.rs` / `branch.rs` / `stash.rs` that today takes `&mut Repo` splits along
that line mechanically. `delta.rs`, `kra.rs`, `tiles.rs`, `raster.rs`, `palette.rs`, `merge.rs`,
`cpu.rs`, `diskspace.rs` are storage/format layers and should need **no** logic change — only the
type they're handed.

The load-bearing save-ordering invariants (see [`data-integrity.md`](data-integrity.md)) survive
verbatim, because each is *within* one document's state files: tips go last, fsync before rename,
a stash must not write `index`, `create` saves before reverting, `pop` writes files before
dropping the record.

### Document identity

`docId` is an opaque short id minted when a document is first tracked — **not** the relpath, so
renaming or moving a `.kra` doesn't orphan its history. `doc.json` holds the current relpath;
lookup is by relpath, with the id as the durable key.

Rename detection: the scanner already hashes changed files, so an untracked `.kra` whose content
hash equals a tracked-but-now-missing document is a rename. Phase 5 — earlier phases can treat it
as a new document and let the user re-point it, which is recoverable; silently guessing wrong
isn't.

### Non-`.kra` files

Drop standalone palettes from *new* tracking (one line in `scan::is_supported`) — a `.kra`'s
embedded palettes are already diffed, which is the observation that motivated this whole change.
Existing repos keep whatever they already track; those files migrate into their own single-file
documents rather than being pruned — same guardrail philosophy as today's `is_supported`.

### Locking

Stays **one repo-wide lock**. Mutations are rare and human-paced, and a shared `objects/` plus a
shared `chains/` means two concurrent document commits touch shared state anyway.

> `ponytail:` repo-wide lock, one writer at a time across all documents. Upgrade to per-document
> locks only if a real workload shows contention — the shared object/pack writer would need its
> own lock first, which is most of that work.

### GC and check

`gc::mark_live` and `check.rs` become repo-wide loops over every document's roots (each document's
branch tips **plus** its stashes) unioned before the sweep. Objects are shared, so **the sweep must
stay repo-wide** — a per-document GC would delete objects another document still references. This
is the most dangerous part of the change and needs a test that commits two documents sharing tiles,
deletes history in one, and asserts the other still reconstructs.

## Migration from an existing repo

Per document, project the existing repo-wide history onto that file:

1. For each branch, walk its ancestor chain newest→oldest.
2. Keep commits whose `files` mention this document's relpath; rewrite `files` to just that entry.
3. Re-parent each kept commit to the previous kept commit on that walk — preserving the
   `files == diff vs first parent` invariant that `tree_at_commit` folds along.
4. A merge commit keeps its second parent only if that parent also survived projection; otherwise
   it flattens to an ordinary commit.
5. Stashes project the same way; a stash touching N documents becomes N stashes.

**Honest caveat, and the reason to do this before histories get long:** a repo-wide branch does
*not* project cleanly. A branch `experiment` holding changes to both `A.kra` and `B.kra` becomes
an `experiment` branch on each — and switching only `A`'s produces a working tree the project
never actually had. That's inherent to the model change, not a migration bug. Say so in the
migration prompt; it is precisely the coupling B exists to remove.

Migration is one-way. Force a backup (`export_repository_zip` already exists) before running it.

## Blast radius

| Area | Change |
|---|---|
| `repo.rs` | Split `Repo` → `Project` + `Document`; new `docs/<docId>/` layout; migration. The bulk of the work. |
| `commit.rs`, `branch.rs`, `stash.rs` | Retype to `Document`; logic largely intact. |
| `gc.rs`, `check.rs` | Roots become a union across documents; sweep stays repo-wide. |
| `commands.rs` | ~20 commands gain a `doc` arg alongside `path`; new `list_documents`. |
| `bin/kvc.rs` | `--doc <id\|relpath>` on every subcommand; `status` with no `--doc` lists all documents. **Keep the `"usage: kvc"` prefix** — the plugin's "Locate kvc…" picker matches on it. |
| `krita-plugin/` | Gets *simpler*: the docker scopes to the active document, so the per-file checkbox list collapses. `_save_tracked` / `_rebuild_docs` and their two traps each stay as-is. |
| `src/lib/repository.tsx` | A project still exists (the folder holding `.kvc/`), but the selected noun becomes the document. |
| `src/lib/repoData.ts` | Every hook keys on `(repoPath, docId, refreshNonce)`. |
| `VersionMapPanel`, `ChangesPanel`, `BranchesPanel` | Version Map gets *better* — genuinely one line of versions. `ChangesPanel` collapses to one row; staging and its partial-commit confirm modal can go. |
| Docs | `version-control.md`, `backend-architecture.md`, `data-integrity.md`, `CLAUDE.md`. |

The Rust split and migration is the long pole; the frontend is wide but shallow; the plugin and
CLI are small. Weeks, not days — and it doesn't split safely across a release boundary, because
the on-disk layout changes.

## Suggested phasing

1. **`Project`/`Document` split, with exactly one document per project.** No UI change, no
   migration prompt — a pure refactor where the existing tests must pass unchanged. The risk lives
   here.
2. **N documents per project**: `list_documents`, GC/check union, the shared-object GC test above.
3. **Migration** of existing repos, behind a confirm + forced backup.
4. **Frontend + CLI + plugin** re-pointed at documents; drop the staging UI.
5. **Rename detection.**

## Deliberately not doing

- Sharding `objects/` per document — kills cross-document tile dedup, buys nothing.
- Per-document locks — see the `ponytail:` note above.
- Keeping the repo-wide history alive alongside the per-document one. Two sources of truth for
  "what happened" is how you get a history that disagrees with itself.

# Per-Document Tracking

**Status: built** (v2.0.0). This document records the shape of the change and, more usefully, why
it is not the shape the original proposal specified.

## The idea

A *repository* used to be a folder, with one history covering every tracked file in it. Now
**one `.kra` document = one history**: the artist opens `painting.kra` and sees that painting's
versions, its own branches, its own set-asides — no folder-level mental model, no git-shaped
"which files go in this commit" step.

Motivation: the audience is artists who have never used git. A repo-with-staging is the single
biggest conceptual tax the app charges, and it bought almost nothing here, because the tracked set
was already nearly always *one document plus palettes it embeds anyway* (a `.kra`'s document
palettes surface as `Palette` diff entries off the `.kra` itself — `commands::kra_palette_dtos`).

## What was built, and why not "Option B"

The original proposal weighed a per-file UI *lens* (Option A — rejected, because branches and
stashes stay repo-wide and that is exactly the seam that confuses the target user) against a real
per-document store (Option B). Option B specified `.kvc/docs/<docId>/` **sharing one `objects/`
across every document in a project**, which forced:

- splitting `Repo` into `Project` + `Document`, and retyping `commit.rs` / `branch.rs` /
  `stash.rs` along that line;
- minting opaque document ids;
- unioning GC roots across documents while keeping the sweep repo-wide — which the note itself
  called *"the most dangerous part of the change"*, since a per-document GC would delete objects
  another document still referenced.

Its own estimate was weeks.

**What shipped instead: one container folder, many self-contained stores.**

```text
artfolder/
  painting.kra
  study.kra
  .kvc/                    hidden; holds README.txt and one store per tracked document
    README.txt
    painting-a3f9c1/       config.json, doc.json, index.json, commits.log,
                           branches.json, stashes.json, objects/, cache/, chains/
    study-7b02e4/          ditto, entirely independent
```

The clutter problem that self-contained stores would normally cause — seven tracked paintings
meaning seven folders in the art folder — is solved by the shared **container**, not by a shared
**store**. Nothing is shared between the stores inside it.

That one decision removes the whole expensive half of Option B. `Repo` keeps its shape: one root,
one index, one commit log, one branch set, one GC. There is no `Project`/`Document` split, no
docId registry, no root union, and no way for collecting one painting's garbage to touch another's
blobs.

What it costs is cross-document tile dedup, which is worth approximately nothing: dedup pays off
*within* one painting's history (the same tiles across versions), and that is untouched. Two
different paintings share essentially no tiles.

## Design

### `Repo` gains a second path

`root` used to mean both "the working tree" and "where the store lives". Those are now two fields:

- **`root`** — the folder holding the document. Everything written back to disk is `safe_join`ed
  onto it.
- **`store`** — where history lives: `<root>/.kvc/<slug>/`, or anywhere at all under a custom
  store root.

`objects_dir`/`cache_dir`/`chains_dir` take the store. `kvc_dir()` is gone — the store *is* the
directory those used to derive.

Everything in `delta.rs`, `kra.rs`, `tiles.rs`, `raster.rs`, `merge.rs`, `palette.rs`, `gc.rs`,
`check.rs`, `commit.rs`, `branch.rs` and `stash.rs` needed **no logic change** — only the path it
was handed. The load-bearing save-ordering invariants (see [`data-integrity.md`](data-integrity.md))
survive verbatim, because each was already *within* one store's state files.

### Document identity

`store_slug()` is a sanitised file stem plus a short hash of the filename, so `a b.kra` and
`a-b.kra` don't collide. `doc.json` inside each store records `{ relpath, displayName, createdAt }`
— the durable answer to "which document is this?", and the hook a future rename re-point would
use. `Repo::doc` carries it, so the tracked document is known even before the first commit (the
index only knows about files that have been *committed*).

### `scan.rs` collapsed

With one designated document there is nothing to *discover*, so the `WalkDir` walk over the
project folder is gone: `scan_detailed` stats one file, keeps the racy-clean guard against the
index's own mtime, hashes on mismatch, and reports `M`, `D`, or nothing. Scanning an art folder
holding fifty 400 MB `.kra` files now costs one `stat`.

`is_supported` reduced to `.kra` plus the `-autosave.kra` rejection. It no longer gates a walk —
it gates `Repo::init`, which is the only place new tracking can begin.

### Standalone palettes are no longer tracked

`.gpl`/`.kpl`/`.aco`/`.ase` dropped out of tracking entirely. `palette.rs` is untouched and still
parses a `.kra`'s **embedded** document palettes, which is where the value always was — that
observation is what motivated this whole change.

### Where the store lives

Default is the container beside the document, so history travels with the art, lands on the same
drive, and survives an OS reinstall. A **custom store root** (Settings → Storage) moves every
*new* store under one folder instead; it lives in `%LOCALAPPDATA%/com.zeru-sakamoto.krita-vc/
storeRoot.json` rather than in any `Config`, because the `kvc` CLI never sees the app's settings
and has to resolve the same store for the same document. The lookup is cached in a `RwLock` and
invalidated on write, since `store_dir_for` now runs on every command.

The container is hidden (`SetFileAttributesW`, Windows-only) and carries a `README.txt` explaining
what deleting it would destroy. Both are best-effort: neither failing is a reason to refuse to
create a store.

### The distinction that matters most

`Repo::locate` must tell three states apart, and conflating two of them loses data:

| State | Meaning | UI answer |
|---|---|---|
| opens | tracked | show the history |
| `NotARepo` | never versioned | offer to start tracking |
| `StoreUnreachable` | history is on a drive that isn't mounted | say so, and do nothing else |

Answering the third with the second would mint an empty store and orphan every version the artist
ever saved. `locate_failure` is split out as a pure function precisely so this rule is testable
without mutating the process-global store root (`unreachable_store_is_not_reported_as_untracked`).

### Deleting

`Repo::delete` removes the **store**, never the artwork — and takes the container with it once the
last store in it is gone. Under the folder model this deleted the project tree, art files
included. The confirm dialog asks the artist to type the artwork's *name*, because deleting
history is unrecoverable and the artwork keeps looking perfectly fine afterwards, so a misclick
has nothing to announce it.

### Locking

Still one advisory lock, now per store (`<store>/kvc.lock`). `RepoLock::acquire` takes the
*document* path and derives the store itself, so every call site was unchanged and the `Locked`
error still names the artwork rather than an internal directory.

## Blast radius, as built

| Area | Change |
|---|---|
| `repo.rs` | `store` field, `doc.json`, `store_slug`/`store_dir_for`/`locate_failure`, custom store root, hidden container. The bulk of the work. |
| `scan.rs` | Walk deleted; one `stat`. `is_supported` → `.kra` only. |
| `commands.rs` | **No signature changes** — `path` simply became the `.kra`'s path. `init`/`delete`/`export_zip` changed behaviour; `get_store_root`/`set_store_root` added. |
| `bin/kvc.rs` | Docs only. `--repo` takes a `.kra`; `status` also reports `document`. The `"usage: kvc"` prefix is untouched. |
| `krita-plugin/` | `find_repo` (walk up for `.kvc`) → `find_doc`; `in_repo` (folder prefix) → `is_tracked_document` (exact identity). |
| Frontend | File picker instead of folder picker; "create repository" flow deleted; staging deleted; `ChangesPanel` shows changed **layers**. |

## Not done

- **Layer-level staging.** The Changes tab lists changed layers read-only. Ticking them needs a
  write path that synthesises a `.kra` holding only the selected layers, which
  `merge::merge_layers` already mostly is — it folds selected **top-level** layers from one `.kra`
  onto another with uuid-keyed matching, content comparison via `canon_entry`, name-clash
  suffixing and data-file remapping. Top-level is the grain `merge.rs` natively speaks
  (`layers_node` is the `<layers>` directly under `<IMAGE>`), and it keeps the unit whole: a group
  either comes or it doesn't, so you can never emit XML referencing a data file you didn't copy.
  Full recursion means partial groups, added/removed group rules, mask handling and ancestor
  forcing — every one able to produce a `.kra` Krita won't open, discovered by the artist, in
  their art, later.
- **Rename re-pointing.** Renaming a tracked `.kra` currently reads as untracked. `doc.json` holds
  what a content-hash re-point would need. The same gap is why restoring a backup never renames an
  artwork on the way in — the filename is baked into `doc.json`, `index.json` keys, chain shard
  filenames, `Commit.files[].path` and every `kra:{relpath}:…` stream key, so a clash at the
  destination is Replace-or-skip. A content-hash re-point would unlock both at once.
- **Migration from folder repositories.** There was none to do — the app had no users at v1.
- **Sharing `objects/` between sibling stores.** See above; it is the thing this design exists to
  avoid.

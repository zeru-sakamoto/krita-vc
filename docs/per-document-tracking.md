# Per-document tracking

Status: built, in v2.0.0. This page records the shape of the change and, more usefully, why it isn't
the shape the original proposal specified.

## The idea

A repository used to be a folder, with one history covering every tracked file in it. Now one `.kra`
document is one history: the artist opens `painting.kra` and sees that painting's versions, its own
branches and its own set-aside work, with no folder-level mental model and no git-shaped "which files
go in this commit" step.

The motivation is the audience: artists who have never used git. A repository with staging was the
biggest conceptual tax the app charged, and it bought almost nothing here, because the tracked set
was nearly always one document plus the palettes it embeds anyway (a `.kra`'s document palettes
appear as `Palette` diff entries from the `.kra` itself, through `commands::kra_palette_dtos`).

## What was built, and why not "Option B"

The original proposal weighed a per-file UI lens (Option A, rejected because branches and stashes
would stay repository-wide, which is exactly the seam that confuses the target user) against a real
per-document store (Option B). Option B specified `.kvc/docs/<docId>/`, with one `objects/` shared by
every document in a project, which forced:

- splitting `Repo` into `Project` and `Document`, and retyping `commit.rs`, `branch.rs` and
  `stash.rs` along that line;
- minting opaque document ids;
- taking the union of GC roots across documents while keeping the sweep repository-wide, which the
  proposal itself called "the most dangerous part of the change", since a per-document GC would
  delete objects another document still referenced.

Its own estimate was weeks. What shipped instead is one container folder holding many
self-contained stores:

```text
artfolder/
  painting.kra
  study.kra
  .kvc/                    hidden; holds README.txt and one store per tracked document
    README.txt
    painting-a3f9c1/       config.json, doc.json, index.json, commits.log,
                           branches.json, stashes.json, objects/, cache/, chains/
    study-7b02e4/          the same, entirely independent
```

Self-contained stores would normally mean clutter (seven tracked paintings, seven folders in the art
folder). That's solved by the shared container, not by a shared store; nothing is shared between the
stores inside it. That one decision removes the whole expensive half of Option B. `Repo` keeps its
shape: one root, one index, one commit log, one set of branches, one GC. There's no `Project` and
`Document` split, no document-id registry, no union of roots, and no way for collecting one
painting's garbage to touch another's blobs.

What it costs is dedup of tiles across documents, which is worth close to nothing. Dedup pays off
within one painting's history (the same tiles across versions), and that's untouched; two different
paintings share essentially no tiles.

## Design

### `Repo` gets a second path

`root` used to mean both "the working tree" and "where the store lives". Those are now two fields:

- `root`: the folder holding the document. Everything written back to disk is `safe_join`ed onto it.
- `store`: where history lives, `<root>/.kvc/<slug>/`, or anywhere at all under a custom store root.

`objects_dir`, `cache_dir` and `chains_dir` take the store. `kvc_dir()` is gone; the store is the
directory those used to derive.

Everything in `delta.rs`, `kra.rs`, `tiles.rs`, `raster.rs`, `merge.rs`, `palette.rs`, `gc.rs`,
`check.rs`, `commit.rs`, `branch.rs` and `stash.rs` needed no logic change, only the path it's
handed. The save-ordering rules that matter (see [data-integrity.md](data-integrity.md)) survived
unchanged, because each was already within one store's state files.

### Document identity

`store_slug()` is a sanitized file stem plus a short hash of the filename, so `a b.kra` and `a-b.kra`
don't collide. `doc.json` in each store records `{ relpath, displayName, createdAt }`, the lasting
answer to "which document is this?" and the hook a future rename re-point would use. `Repo::doc`
carries it, so the tracked document is known even before its first commit (the index only knows
about files that have been committed). Under a custom store root the slug is salted with the
document's folder path, because stores for documents in different folders share one parent there.

### The scanner collapsed

With one chosen document there's nothing to discover, so the `WalkDir` walk over the project folder
is gone. `scan_detailed` stats one file, keeps the racy-clean guard against the index's own mtime,
hashes on a mismatch, and reports `M`, `D` or nothing. Scanning an art folder holding fifty 400 MB
`.kra` files now costs one `stat`.

`is_supported` shrank to `.kra` plus the rejection of `-autosave.kra`. It no longer gates a walk; it
gates `Repo::init`, the only place new tracking can start.

### Standalone palettes are no longer tracked

`.gpl`, `.kpl`, `.aco` and `.ase` files dropped out of tracking entirely. `palette.rs` is unchanged
and still parses a `.kra`'s embedded document palettes, which is where the value always was; that
observation is what motivated the whole change.

### Where the store lives

By default the store sits in the container beside the document, so history travels with the art,
lands on the same drive, and survives an OS reinstall. A custom store root (Settings → Storage →
"Where version history is kept") puts every new store under one folder instead. It lives in
`%LOCALAPPDATA%/com.zeru-sakamoto.krita-vc/storeRoot.json` rather than in any `Config`, because the
`kvc` CLI never sees the app's settings and has to resolve the same store for the same document. The
lookup is cached in a `RwLock` and invalidated on write, since `store_dir_for` now runs on every
command.

The container is hidden (`SetFileAttributesW` on Windows; the leading dot does it elsewhere) and
carries a `README.txt` explaining what deleting it would destroy. Both are best effort: neither
failing is a reason to refuse to create a store.

### The distinction that matters most

`Repo::locate` has to tell three states apart, and treating two of them the same loses data:

| State | Meaning | What the UI does |
|---|---|---|
| opens | tracked | shows the history |
| `NotARepo` | never versioned | offers to start tracking |
| `StoreUnreachable` | the history is on a drive that isn't mounted | says so, and does nothing else |

Answering the third like the second would create an empty store and orphan every version the artist
ever saved. `locate_failure` is split out as a pure function precisely so this rule can be tested
without changing the process-global store root (`unreachable_store_is_not_reported_as_untracked`).

### Deleting

`Repo::delete` removes the store, never the artwork, and removes the container too once the last
store in it is gone. Under the folder model this deleted the project folder, art files included. The
confirm dialog asks the artist to type the artwork's name, because deleting a history can't be undone
and the artwork looks perfectly fine afterwards, so a misclick has nothing to announce it.

### Locking

There's still one advisory lock, now per store (`<store>/kvc.lock`). `RepoLock::acquire` takes the
document path and derives the store itself, so no call site changed, and the `Locked` error still
names the artwork rather than an internal folder.

## What changed, as built

| Area | Change |
|---|---|
| `repo.rs` | The `store` field, `doc.json`, `store_slug`, `store_dir_for`, `locate_failure`, the custom store root, the hidden container. Most of the work. |
| `scan.rs` | The walk deleted; one `stat`. `is_supported` accepts `.kra` only. |
| `commands.rs` | No signature changes: `path` simply became the `.kra`'s path. `init`, `delete` and the backup export changed behavior; `get_store_root` and `set_store_root` were added. |
| `bin/kvc.rs` | Docs only. `--repo` takes a `.kra`, and `status` also reports `document`. The `"usage: kvc"` prefix is untouched. |
| `krita-plugin/` | `find_repo` (walking up to find a `.kvc`) became `find_doc`, and `in_repo` (a folder-prefix test) became `is_tracked_document` (exact identity). |
| Frontend | A file picker instead of a folder picker; the "create repository" flow and file staging deleted; `ChangesPanel` shows changed layers. |

## Done since

- **Layer-level staging.** The Changes panel's rows are checkboxes now.
  [`stage::stage_kra`](../src-tauri/src/stage.rs) synthesizes a `.kra` holding the ticked layers plus
  the committed form of every other one, and `commit_selected`'s new `layers` argument stores it. It
  landed at the top-level grain this page predicted, reusing `merge.rs`'s zip and XML helpers, but
  not `merge_layers` itself, whose splice turned out to be wrong for the job. A merge adds layers on
  top with fresh uuids and ` [2]` names because it's adding a second copy. Staging substitutes a layer
  in place, so it has to keep the uuid (that's what lets the next diff recognize the layer) and keep
  the committed `layerN` filename unless something actually collides (a rename would re-store every
  reverted tile under new stream keys and lose dedup against the history it came from). The
  prediction was right about the grain and about `layers_node`, and wrong that the existing function
  was "already mostly it". See [layer-staging.md](layer-staging.md).

## Not done

- **Staging inside a group.** A group is still saved whole. Going finer means partial groups, rules
  for added and removed groups, mask handling and forcing ancestors, and any of those can produce a
  `.kra` Krita won't open, discovered by the artist, in their art, later.
- **Re-pointing after a rename.** A renamed `.kra` currently reads as untracked. `doc.json` holds
  what a content-hash re-point would need. The same gap is why restoring a backup never renames an
  artwork: the filename is baked into `doc.json`, the `index.json` keys, the chain shard filenames,
  `Commit.files[].path` and every `kra:{relpath}:…` stream key, so a clash at the destination is
  Replace or Skip. A content-hash re-point would solve both at once. Moving the folder with its
  `.kvc` container works today, because the default slug depends only on the filename.
- **The Krita plugin under a custom store root.** The plugin's `find_doc` only treats a document as
  tracked when a `.kvc` folder exists beside it, and with a custom store root none is created there.
  So the docker reports a painting whose history lives under a custom root as not version-controlled,
  unless another painting in the same folder happens to have a `.kvc` container. The fix is to ask
  `kvc` instead of checking for the folder; until then, the plugin needs the default location.
- **Migration from folder repositories.** There was none to do; the app had no users at v1.
- **Sharing `objects/` between sibling stores.** See above. It's the thing this design exists to
  avoid.

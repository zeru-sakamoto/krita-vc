# Stashes: setting work aside

A stash parks the working tree's changes to the side of history, puts the files on disk back to the
current tip, and brings the changes back later. In the UI it is "Set aside" when Artist Mode is on
and "Stash" when it's off. The engine lives in [`stash.rs`](../src-tauri/src/stash.rs), the `.kra`
layer merge used when bringing work back onto an edited painting lives in
[`merge.rs`](../src-tauri/src/merge.rs), and the dialogs live in
[`StashDialogs.tsx`](../src/components/vcs/StashDialogs.tsx).

## Why a stash is not a commit

A stash never enters `commits.log`. As a commit it would show up as a spurious version row in the
storage report (`compute_storage_stats` walks every commit, unfiltered), and because it would be
parented on the tip, it would silently block "Undo the last version". Instead a `Stash` record (id,
label, author, timestamp, origin branch, and `files: Vec<CommittedFile>`) lives in the store's
`stashes.json`. A missing `stashes.json` means an empty shelf, which is the whole migration for
stores older than stashing.

Storage is borrowed entirely from the commit path. `commit::store_change`, shared with
`commit_selected`, stores each changed file through the same relpath-keyed streams a commit uses
(`kra:{rel}:*`). So a stashed `.kra` dedups its unchanged tiles against committed history, and
setting aside a lightly edited painting costs almost nothing. It also means `gc.rs` marks a stash's
content with the same walk it uses for commits, and the commit path's restore
(`commit::write_committed`) writes it back with no new code. Stashes are GC roots: nothing in `commits.log`
refers to them, so without that rule "Clean up storage" would collect the shelf.

## Three orderings that matter

Each of these has a test that fails without it.

1. A stash must not touch `index.json`. The index is the committed head, and a stash commits nothing.
   `store_change` returns the index entry instead of applying it, and `stash::create` drops it.
   Recording the stashed hash would make the revert below scan the file as clean and skip it, so the
   set-aside would silently fail to clear the tree.
2. `create` saves before it reverts. `discard_working_changes` erases the work from disk before its
   own save, so the stash record must already be durable when it runs. Otherwise a crash mid-revert
   leaves the files reverted with no record of what was in them. Saving first turns that into a
   harmless failure: the stash is on the shelf and the files are still dirty.
3. `pop` writes the files before it drops the record, for the mirror-image reason. A crash between
   the two only means the next pop reports a conflict, which is recoverable.

## Operations

- **Create** (`stash::create`). Scans, filters to `only` (unused by the UI now that a store tracks
  one document, but still the CLI's `--paths`), stores the content, records the stash, then reverts
  through `commit::discard_working_changes`. Returns `Nothing` if nothing in scope is dirty, and
  `NoCommit` on a store with no commits, because there's no committed state to revert to; the UI
  gates the menu item on `commits.length` the same way undo does.
- **Pop** (`stash::pop`). Writes each file back (a `"D"` record deletes instead), leaves the index
  alone, which is exactly what makes the restored work scan as changed again, and then drops the
  record. Popping onto a different branch is allowed: `branch` is recorded for display only and
  nothing is ever looked up by it, so a stash outlives the branch it came from.
- **Drop and drop all** (`stash::drop_one`, `drop_all`). Take a stash off the shelf without
  restoring it. They write `stashes.json` alone (`save_stashes()`), never the full `save()`, the
  same narrow flush `save_branches` gives branch edits. The content stays until the next "Clean up
  storage".

## Conflicts when bringing work back

A conflict is a stashed path that has been edited since it was set aside.

- A conflicting `.kra` is merged, not refused. The layers the set-aside version actually added or
  modified are folded into the working file by `merge::merge_layers`, so the artist reconciles them
  by hand in Krita. See the next section.
- Any other conflict still refuses the whole pop with `StashConflict` before a byte is written: a
  file that isn't a `.kra` (there are no layers to merge) or a stashed deletion landing on edited
  work. Overwriting either would destroy the current work with no way back. The error's stable
  `"stash conflict"` prefix is distinct from the branch commands' `"unsaved changes"` prefix, and
  the frontend and the plugin both match on it.
- Every file's inputs (the set-aside version and the merge's ancestor) are gathered before the
  first write, and the result, merged or plain, is built straight into the temp file beside the
  artwork (`merge::merge_layers_into`, `commit::write_committed`) rather than held whole in memory.
  A merge that can't be done cleanly (`MergeFailed`) fails there and deletes its temp, so the
  working tree and the stash stay untouched. That's all-or-nothing because a store tracks one
  document, so a stash holds at most one file.
- The merge's ancestor is rebuilt from only what `merge_layers` compares against it: `maindoc.xml`
  and the data files of the top-level layers the set-aside version also has
  (`stash::merge_ancestor`, via `merge::shared_layer_files`), not the whole committed document. The
  merged archive raw-copies every entry but `maindoc.xml` (compressed bytes, crc32 and size carried
  over), where it used to inflate and deflate the whole painting again at level 6. Bringing set-aside
  work back onto an edited 105 MB painting went from 15.8 s to about 6 s, and its peak memory from
  624 MB to 440 MB: the merged output, bigger than any of its inputs, now goes straight to the temp
  file, and the ancestor's decompressed layers are let go before the repack. The set-aside version,
  the working file and the ancestor are still whole documents in memory while it runs, and when the
  set-aside version shares every layer with the ancestor, as it usually does, the ancestor subset is
  nearly the whole painting anyway.

No frontend or CLI change was needed for merging: a merged pop returns normally through `pop_stash`
or `kvc stash-pop`, so the ordinary "brought back" path applies, and the artist sees the merged
layers directly in Krita.

## Merging set-aside `.kra` work

When a pop finds that the working `.kra` changed since it was set aside, both versions are the same
painting taken two ways. `merge::merge_layers` folds the set-aside version's added and modified
layers into the working file instead of making the artist choose which version to lose. The result
opens in Krita with the working stack plus just those set-aside changes. Combining whole layers is
automatic; reconciling pixels inside a layer is left to the artist.

Only changed layers cross over, not the whole stack. `merge_layers` takes the committed ancestor
that both sides diverged from (the file's version at the branch tip, passed in by `stash::pop`) and
skips any incoming top-level layer unchanged since then. Layers are matched by uuid, then compared
on content rather than on any byte-for-byte form:

- Each `layers/layerN…` data file is canonicalized to its tiles sorted by position (`canon_entry`).
  Krita doesn't keep a layer's tile order stable across saves, so two saves that wrote the same
  tiles in a different order rebuild to different bytes but must compare equal. That's the case that
  made an untouched layer fold in as a duplicate.
- Entries that aren't tiled (`.defaultpixel`, `.icc`, a shape layer's SVG) compare verbatim, and the
  per-layer blobs are collected independently of their filenames, since Krita may renumber `layerN`.
- On top of the tiles, a small set of meaningful attributes must match: `name`, `opacity`,
  `compositeop`, `visible`, `x` and `y`.

It deliberately doesn't compare the raw `<layer>` XML. Krita rewrites volatile attributes on every
save (`selected` on the active layer, `collapsed`, timeline flags), so an untouched layer's XML
differs between two saves and a text compare would fold every layer in, which was one of the bugs
this code had. Anything whose tiles or meaningful attributes differ, or a layer with no match in the
ancestor (a genuinely new layer), folds in, so a real change is never dropped. An obscure attribute
left off the list is at worst not folded, never folded twice. With no ancestor (`None`), every
incoming layer folds, which was the behavior before the ancestor check existed. If every incoming
layer matches the ancestor (the set-aside change was outside the layer stack, a canvas resize for
example), there is nothing to fold and the merge returns `MergeFailed`, touching nothing.

The mechanics work on the raw `.kra`, with no engine internals:

- The working file is the base. The set-aside version's added and modified top-level layers are
  spliced in as the first children of the base's `<layers>`, so they land on top of the stack.
- `roxmltree` (read-only) locates each incoming `<layer>` subtree by byte range (`Node::range`) and
  finds the base's insertion point. The id and name edits are whole-token string replacements
  (`filename="…"`, `uuid="…"`). That is safe because uuids (`{hex}`) and filenames (`layerN`) never
  contain XML-special characters, so no XML writer is needed.
- Every incoming layer's data files and uuid are remapped to fresh ids that can't collide: a new
  `layerN` above the base's highest, checked against the archive itself so an orphaned file can't
  clash, and uuids derived from `blake3`, so there's no `uuid` crate. The layer's archive entries are
  copied into the base's `layers/` folder. A top-level layer whose name clashes with a base layer is
  suffixed ` [2]`, ` [3]` and so on, in the opening tag only, so nested layer and mask names are left
  alone.
- It refuses (`MergeFailed`, nothing written) when the two versions use different color spaces,
  rather than write a file Krita can't open. A different canvas size is allowed through; Krita opens
  it and the incoming layers may just sit at an offset.

## In the desktop app

Set-aside actions live in `Sidebar`'s panel-options `Menu` (the ⋮ button). In the Changes view it has
three divider-separated groups: undo and discard, then set aside, then bring back. `Menu` has no
submenus, but a `MenuItem.separator` flag draws a rule above a row, because one `footer` group can
only draw one divider and this menu needs two.

- **Set this aside** stashes everything. It used to be two rows, staged files and everything, and
  with one tracked painting those became the same action. `StashScope` kept its parameter but lost
  its `"staged"` arm, so layer-scoped set-aside has somewhere to go if it's ever built. The row is
  gated on `commits.length`, like undo, since there's no committed state to revert to otherwise.
- **Bring back latest** and **Bring back…** are the menu's footer items. The list is newest first,
  so "latest" is `stashes[0]`.
- All three are Changes-view only, because they act on the working tree. The History view's menu
  only has undo.

`SettingsModal`'s Set-Aside tab ("Stashes" when Artist Mode is off) lists every stash with its
label or file summary, origin branch and age, with a remove button per row and "Remove all". It is
a management view, not a restore path; bringing work back stays in the panel menu. The confirm
dialogs (`DropStashModal`, `DropAllStashesModal`) render as siblings of `SettingsModal`, the pattern
`CleanupModal` uses, because `Modal` has no portal.

`StashDialogs.tsx` holds `SetAsideModal` (the label prompt, used by the panel menu and by the
save-first prompt's "Set it aside" button), `PickStashModal` (choose which stash to bring back),
`StashConflictModal` with `isStashConflictError`, the `StashIcon` and `UnstashIcon` glyphs, and the
`stashTitle` and `stashSummary` label helpers that the Settings shelf reuses. Data comes from
`useStashes` in [`repoData.ts`](../src/lib/repoData.ts), which calls `list_stashes` (newest first).
The mutations (`createStash`, `popStash`, `dropStash`, `dropAllStashes`) live on the repository
context with the other write actions.

A branch switch or merge blocked by unsaved work raises `SaveFirstModal` through
`useBranchActions`, whose "Set it aside" option stashes everything and retries the blocked action
(see [version-map.md](version-map.md#branch-actions-and-pick-a-version-mode)).

## In the Krita docker

The docker's ⋮ menu mirrors the desktop's groups: "Discard checked changes", "Set aside checked
changes…" (with an optional label), then "Bring back latest" (showing how many are set aside) and
"Bring back…". Both bring-back rows use `kvc stash-list`, which reuses `commands::stash_dtos` for its
newest-first order, the order "bring back latest" depends on. A switch blocked by unsaved work offers
"Set aside & switch", which stashes everything and retries the switch. Every one of these rewrites
the `.kra` on disk, so the docker wraps them in `_rebuild_docs`, which reopens the painting in Krita
afterwards (see [`krita-plugin/README.md`](../krita-plugin/README.md)).

## Commands

| Command | What it does |
|---|---|
| `list_stashes(path)` | The shelf as `StashDto[]` (`{ id, label, author, timestamp, branch, changes }`), newest first. Stream hashes stay in the backend. |
| `create_stash(path, label, author, paths?)` | Set aside changes and revert those files. `paths` limits it to those relative paths; omitted or `null` sets aside everything dirty. Needs `Repo::open`, because storing content writes streams, which a light repo forbids. Returns the shelf. |
| `pop_stash(path, id)` | Bring a stash back and drop it from the shelf. Errors with a `"stash conflict: …"` message when a non-`.kra` path or a stashed deletion conflicts. Returns the shelf. |
| `drop_stash(path, id)`, `drop_all_stashes(path)` | Remove stashes without restoring them. The next `cleanup_repository` reclaims their storage. |

The CLI equivalents are `kvc stash`, `kvc stash-pop` and `kvc stash-list`.

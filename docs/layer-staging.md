# Layer-subset staging: saving only some layers

The Changes panel lists the layers that changed since the last version, and each row is a checkbox.
Ticking a subset sends those layer ids as `commit_snapshot`'s `layers` argument. The store versions
whole documents, so the version gets a `.kra` synthesized for the occasion by
[`stage.rs`](../src-tauri/src/stage.rs) (`stage::stage_kra`): the working file with every unticked
top-level layer reverted to its committed form. The file on disk is never touched, so the layers
left out stay uncommitted and the painting still scans as changed afterwards. A partial commit
defers work; it never discards it.

The same thing is available from the CLI as `kvc commit --layers '["<id>"]'`, a JSON array like
`--paths`. The Krita docker never passes it, because it has no layer picker, but the CLI stays a
complete surface over the engine. `commit_snapshot`'s separate `paths` argument
(`commit::commit_selected`) is unrelated; it survives for the CLI. Tests are in
[`src-tauri/tests/staging.rs`](../src-tauri/tests/staging.rs), and the measured cost is in
[performance.md](performance.md#layer-subset-staging).

## The Changes panel

[`ChangesPanel`](../src/components/vcs/ChangesPanel.tsx) shows "the layers that changed", not a
file list. A store tracks one `.kra`, so a file list would always be a single row, and staging a
subset of a one-file working tree means nothing. What the artist wants to know is what moved in the
painting. The rows come from `useWorkingDiff`'s existing per-layer `change`, so there is no new
backend command, rolled up so that a changed group reads as one row with a "+N inside" count instead
of spilling its children.

Three details hold the panel together.

- **The rollup is exact.** It is keyed on `ArtLayer.topLevelId`. The backend enumerates layers with
  `.descendants()`, so a group's children arrive as siblings of the group, and `kra::LayerNode.top`
  records each layer's top-level ancestor so `LayerDto` can ship it. The panel used to guess the
  grouping from "changed layers following a changed group", which over-counted with several groups
  and showed a changed layer inside an unchanged group as a row of its own. That was harmless while
  the rows were read-only. It isn't once a row's id is what actually gets committed.
- **Rows list top first**, matching the diff navigator (`LayerStackPanel` renders
  `[...diff.layers].reverse()`) rather than the backend's raw bottom-to-top `.kra` order.
  `layerRows` builds the top-level ids in raw order (first seen per group) and reverses the result
  once at the end, so the two panels never disagree about which layer is on top.
- **Everything starts ticked**, and the state the panel tracks is what has been unticked. A layer
  that shows up in a later scan is included by default, and doing nothing saves the whole painting
  exactly as before (`layers: null`, the same call the panel made before layer picking existed).
  Ticks reset when the painting, the branch or `refreshNonce` changes, since all three replace the
  rows.

Changes outside the layer stack (canvas size, animation, document settings) always go into the
version. There's no row to untick them with, and a line of text says so once the selection is
partial. The button counts the selection ("Save 3 of 7 layers"), turns into "Choose at least one
layer" when nothing is ticked, and a "Choose all" or "Choose none" link sits above the rows. A first
version has no committed side to revert to, so it always saves the whole painting, and the panel
says so.

## How the version is synthesized

Staging works at the top-level grain only, the same grain [`merge.rs`](../src-tauri/src/merge.rs)
speaks (`layers_node` is the `<layers>` element directly under `<IMAGE>`). A group is taken or left
whole, which makes it impossible to emit XML that references a data file that wasn't copied.
Recursing into groups would mean partial groups, rules for added and removed groups, mask handling
and forcing ancestors, and each of those can produce a `.kra` Krita won't open, discovered by the
artist, in their art, later.

Layers are keyed by id (the uuid, or the name when a layer has none), the same rule
`commands::layer_id` uses, so a tick always matches. There are three cases:

- **Modified and unticked.** The committed `<layer>` subtree is spliced in its place and its data
  files are copied across.
- **Added and unticked.** Dropped, subtree and data files together.
- **Deleted and unticked.** Put back, after whichever committed predecessor survives. Leaving a
  deletion unticked can only mean "don't save that deletion".

Everything outside the layer stack, including canvas size, animation, document settings and
embedded palettes, comes from the working file unconditionally. A version is "the document, minus
some layers", and reverting `<IMAGE>` attributes or animation blocks would be a separate kind of
surgery.

### Rebuilding the committed side

The committed side is rebuilt in two passes. `stage::committed_subset` reconstructs `maindoc.xml` on
its own, plans the output stack against it with `stage::plan_pieces` (the same function `stage_kra`
uses, so the two can't disagree about which layers get reverted), and then reconstructs only the
data files that plan names through `kra::reconstruct_kra_from`'s entry filter. That's typically one
or two layers out of a stack. The first implementation called `commit::bytes_of`, a full
`reconstruct_kra`: every tile of the whole document replayed through its delta chains, plus a
re-encode of `mergedimage.png` from its pixel blocks, almost all of it thrown away a moment later.
It was the largest single cost of a partial commit.

The subset archive is deliberately not an openable `.kra`. It is an input to `stage_kra` and nothing
else. One consequence is that `stage_kra`'s `taken` collision set no longer sees committed data
files that belong to layers that aren't being reverted. It doesn't need to, because those can't
appear in the output either.

### How it differs from a merge

Staging reuses `merge.rs`'s zip and XML helpers, but it differs from `merge_layers` in two ways,
because a merge adds a second copy of a layer while staging substitutes the same one.

- **Uuids are never remapped.** The preserved uuid is exactly what lets the next diff recognize the
  layer.
- **Committed `layerN` filenames are kept** unless they collide with a layer that survived from the
  working file. Tile streams are keyed `kra:{rel}:tile:{image}/layers/{layerN}:{x},{y}`, so renaming
  unconditionally (which `merge_layers` does, correctly, for its own case) would re-store every
  reverted layer's tiles under new keys and lose dedup against the history they came from. Krita
  does renumber `layerN` between saves, so the collision path is real and tested; it just isn't the
  common case. Keeping the names also enables a shortcut at the entry level: a reverted layer comes
  back byte-identical, so `commit_kra`'s crc32 and size check matches the previous manifest and the
  entry is reused as-is without ever being inflated.

### Output details

Every entry of the synthesized archive but `maindoc.xml` is **raw-copied** (`raw_copy_file`, and
`raw_copy_file_rename` for a reverted layer's data files): its compressed bytes, method, crc32 and
size go across as they are, from the working file or the committed subset, never inflated and
deflated again. The archive is never written to disk (`commit_kra` reopens it immediately), and the
crc32 and size its reuse check compares describe the uncompressed bytes, so nothing about what gets
stored changes. The one entry it writes itself, `maindoc.xml`, uses deflate level 1
(`stage::out_opts`). Recompressing every entry was 961 ms of work on a 105 MB painting, against
35 ms for the copy; before that, at the zip crate's default level 6, it was the largest cost of a
partial commit. `tests/staging.rs::kept_entries_are_copied_compressed` pins it.

`mergedimage.png` and `preview.png` are dropped from the synthesized archive. They are Krita's
renders of the whole stack, which the engine can't redo, so carrying the working copies would ship a
preview that shows layers the version doesn't contain, visible even in the artist's file manager
once that version is restored. Both regenerate: Krita rewrites them on the next save. Until then,
`commands::stacked_composite_url` composites the stack itself (`raster::composite_stack`, keyed by
`kra::KraManifest::version_key` and cached in the regenerable `cache/`) so the Version Map node isn't
blank. That compositor models source-over blending, per-layer opacity, `visible`, one level of group
opacity and the five blend modes `svgArt.ts` maps; masks and filter, clone and vector layers render
as plain paint. That's the same ceiling the frontend's SVG stacker has always had, and it's
acceptable because the result is only ever a preview, never document data.

### What it refuses

It refuses rather than write a broken file, with `KvcError::StageFailed` (its own variant, so the
message can talk about picking layers rather than set-aside work):

- a color-space change between the two versions, since the copied layer data would be in the wrong
  pixel format;
- a malformed `maindoc.xml`;
- a first version, which has no committed side to revert unticked layers to.

Ticking only layers that didn't actually change returns `Nothing`, not an empty commit.

## Keeping the painting dirty afterwards

A partial commit stores content that is deliberately not what sits on disk, so the painting has to
keep reading as modified. But the working file's size and mtime match the index, which is exactly
the scanner's "unchanged" signal (see
[version-control.md](version-control.md#file-tracking-the-scanner)). So the index says so outright:
the entry is flagged `TrackedFile.partial` (set through `ScanChange::partial` in
`commit::store_change`). On a size and mtime match, `scan_detailed` reports a `partial` entry as
`"M"` from the `stat` alone, with no read and no hash. Callers that want the bytes (the commit path)
fall through and read as usual.

The first design zeroed `size` and `mtime` instead, to fail the fast path's `(size, mtime) != (0,
0)` clause. It was correct, but it made every later scan read and blake3 the whole document, and
`kvc status` runs on the Krita docker's 1.5-second poll, so one partial commit meant reading the
whole painting twice a second, forever. The recorded `hash` stays the synthesized document's hash,
so a scan that does fall through (a rewrite in the same mtime tick trips the racy-clean rule) still
comes out `"M"`.

Two traps, each with a test in `tests/staging.rs`: the partial flag must be set, or the next scan
calls the painting clean and the held-back layers vanish from the Changes panel with nothing to
announce it; and the preview renders must be dropped, or a restored version shows layers it doesn't
have.

## Not built yet

- **Staging inside a group.** A group is still saved whole, for the reasons above.
- **Manifest-level splicing.** `stage_kra` still repacks the entire working document and hands it to
  `commit_kra`, which decomposes it again. The upgrade is to commit the surviving working entries as
  usual and substitute the previous manifest's `KraEntry` values for the reverted layers, so no
  document is ever built. It would also retire the roughly three-times-the-document peak memory of
  `stage::repackage`. See [performance.md](performance.md#layer-subset-staging) for why it wasn't
  worth building yet.

# Bugs found and fixed during evaluation testing

Dates: 2026-11-04 to 2026-11-26. Commits: `60238c1` (the pixel-comparison fix below), `93ce7c4` and
`0417d63` (a solid-accent After pane, group layers, and showing a layer's previous properties;
neither is described in this chapter) and `d9a2078` (the later fixes at the end). All sit on top of
`36e3060` ("Bump the version to 2.2.0").

This chapter records bugs found while v2.2.0 was evaluated for the BSc project report that the app
was built for. The evaluation compares the app with Git and Git LFS on repository size, diff
accuracy, processing performance and CPU use while Krita is open. Its test cases are listed in the
report's Testing Tracker, and any bug found has to be fixed and the fixed build re-measured before
its figures are used. Later bugs from the same testing belong here too.

To drive those measurements, `tests/eval.rs` adds ignored tests: `eval_prepare`, `eval_commit`,
`eval_ops`, `eval_replay` and `eval_s200`. PowerShell scripts in the report's own folder call them.
They time each user-facing operation the way one click in the app or one `kvc` call runs it, as a
fresh `Repo::open` plus the operation inside the CPU budget pool. They also add what
`tests/bench.rs` lacks: a 200-version history, a replay of real numbered revisions, and a
single commit at any CPU budget. `kvc` always runs at the 75% default and has no budget flag.

## A layer marked modified with nothing to show

The first bug was reported by the researcher. Tapping the brush just outside the canvas, then
saving, made the diff mark a layer as modified, but the diff view showed no change anywhere.

Krita keeps pixels painted past the canvas edge, so the tap really did change the layer's tile data,
just not anywhere visible. `kra::diff_tile_indexes` marked a layer modified if any tile's stored
bytes differed, or the tile count did, wherever the tile was. Everything the viewer draws was
clipped to the canvas, though: the before and after rasters, the per-layer mask and the region box.
A throwaway engine test on a 128 x 128 canvas confirmed it. One tile added at x = -64 or x = 128
left the layer `modified` with no highlighted region and a document region box 0 px wide.

Looking for other ways to reach the same state found four more, all inside the canvas. The diff
decided "modified" from compressed tile bytes, but drew the highlight from rasters downscaled to at
most 2048 px (`raster::MAX_RASTER_DIM`), and counted a pixel only if a channel moved by more than 16
(`raster::DIFF_THRESHOLD`). Anything that passed the first test and failed the second showed as an
edit with nothing to see. A second probe, committing each case and diffing it as the app does, gave:

| Edit | Before the fix |
|---|---|
| Control: one pixel +100 | modified, highlighted |
| A fully transparent tile appears over a transparent default | modified, nothing highlighted |
| The same pixels stored raw instead of LZF-compressed | modified, nothing highlighted |
| One pixel +10, below the threshold of 16 | modified, nothing highlighted |
| A whole 64 x 64 tile +10 (a faint wash) | modified, nothing highlighted |
| A4 canvas (2560 x 3520): one pixel +60 | modified, nothing highlighted (lost in the downscale) |
| A4 canvas: one pixel +127 | modified, highlighted |

The cases fall into two groups. In the first three rows, including the off-canvas tap, nothing
visible changed, so the label was wrong. In the last three, the label was right and the highlight
missed a real edit. The second group is the one that mattered for the evaluation: a single-pixel
edit (test DA-3) on the A4 and A3 test paintings would have shown a modified layer with no mark.
That also undercut the reason the methodology gives for comparing pixels directly rather than with
SSIM, which is that SSIM under-reports small deliberate edits.

The probes show which states are reachable, not which ones Krita produces in practice. The
off-canvas tap is confirmed by the original report. A transparent tile depends on whether Krita
writes out a tile it allocated but never painted. Re-encoding the same pixels differently should be
rare, since Krita's encoder is deterministic, but a pixel-delta ref (`Config.tile_pixel_deltas`)
hashes differently from the working tile it came from, which reaches the same state. Faint edits
and single pixels on large canvases come from ordinary painting.

## The fix

The fix has three parts, and each one covers different rows of the table.

1. **"Modified" means a pixel inside the canvas changed.** The new `kra::diff_tile_pixels` keeps
   the tile-hash comparison as a pre-filter, so unchanged tiles are never decoded. It then decodes
   only the tiles that differ, on both sides, and compares their pixels exactly, within the canvas.
   A missing tile reads as that side's `.defaultpixel`, or transparent. This covers the off-canvas
   tap, the transparent tile, and the same pixels stored differently. A tile that can't be decoded
   (not RGBA8) counts as changed over its in-canvas area, so the change can only be over-reported,
   never hidden. It reads tiles through the request's `TileCache`, which the layer rasters then
   reuse, and handles committed sides, working sides and low-memory working files alike.
   `art_diff_dto` uses it for every diff: the history, the Changes panel and the Version Map.
2. **No threshold for layer data.** The threshold of 16 stays for the composite, because the
   two `mergedimage.png` files can differ by rounding. A layer's tiles are lossless, and both sides
   go through the same decode and downscale, so `raster::LAYER_DIFF_THRESHOLD` is 0, through
   `raster::diff_overlay_full_with`. Layer masks get their own cache key
   (`kra::layer_diff_cache_key`), so masks cached under the old threshold are never served.
3. **Regions come from the full-resolution comparison and have a minimum size.** Both the
   per-layer region and the document region are now the boxes `diff_tile_pixels` measured, grown by
   `kra::region_with_min_size`: at least 1.5% of the canvas's longer side and at least 8 px,
   centred on the change and kept inside the canvas. A one-pixel edit on an A4 canvas would
   otherwise be well under one screen pixel in the preview pane. When nothing visible changed there
   is no region, rather than an empty box. As a separate guard, `diff_tile_indexes` now returns no
   region when its box clamps to zero size.

`tests/diff_visibility.rs` turns every row of the table into a regression test. It adds a tile
straddling the canvas edge, where only the inside half counts, a tile disappearing, a one-step
colour change, and the minimum-size rule. Each case is diffed three ways: as a committed version
against its parent, and as unsaved work in normal and in low-memory mode. All three must agree.
The new tests and the existing suite pass: 212 tests, plus the ignored benchmarks.

Commits still store every tile, off-canvas ones included, because Krita needs them to rebuild the
file exactly. Only what the diff reports changed. A save whose only change is invisible still makes a
version, because the file changed, but no layer is flagged in it, and the inspector shows its
existing "No changes to show." message.

## What it costs, and what is left

The pixel comparison runs in the first, cheap stage of a diff as well, since the `modified` label
is part of that stage. Its cost scales with the size of the edit, not of the painting: a stroke
decodes a few tiles, and a fill decodes every tile of the layer. It has not been timed on the test
paintings yet. The evaluation's diff timings (PF-5, PF-6) have to come from this build.

## Installing for all users

Installing the 2.3.0 build showed that the NSIS setup `.exe` could not install into
`C:\Program Files`: with no install mode set, Tauri builds a per-user installer that never asks for
admin rights. `tauri.conf.json` now sets `bundle.windows.nsis.installMode` to `"both"`, so the
installer asks whether to install for the current user or for all users. The MSI was already
per-machine.

Checked on 2026-11-26 by building the installer (`tauri build --bundles nsis`) and reading the
result, without running it, since running it needs a UAC prompt and would replace the installed
app:
- The generated `installer.nsi` defines `INSTALLMODE "both"`, includes the install-mode page, and
  defaults the all-users directory to `$PROGRAMFILES64\Krita-VC`, which is where the installed copy
  is.
- The embedded manifest asks for `highestAvailable`, where the 2.2.0 installer's said `asInvoker`.
  So an administrator gets the UAC prompt as soon as the installer starts, before the page that
  chooses between current user and all users, and so also for a per-user install. A standard user
  gets no prompt and can only install for themselves.

Not checked: stepping through both choices in the installer, and the upgrade path from a per-user
install to an all-users one.

## Later fixes (2026-11-26)

Three more bugs came from using the build on the developer's machine, all in the Changes view and
the app shell rather than the engine (`d9a2078`).

- **A second launch opened a second window.** Starting the app while it was already running created
  another instance. `tauri-plugin-single-instance` is now registered first in `lib.rs` (desktop
  targets only), so the second launch exits at once and the running app restores, shows and focuses
  its `main` window. The `kvc` CLI is a separate binary and isn't affected.
- **The save button was usable before the diff had loaded.** The Changes panel already tracked a
  `checking` state (scan or diff in flight) but the button ignored it, so a version could be saved
  before the artist had seen what was in it. The button is now disabled while checking and reads
  "Checking changes…", and the save handler returns early too.
- **The Changes list and layer list flashed on refresh, most on a fast machine.** A refresh swapped
  the list for a "Looking at what changed…" line, and reset every layer thumbnail to a spinner, even
  when the answer arrived a few milliseconds later. Three changes fix it:
  1. A refresh keeps the previous content on screen. `useArtLayers` returns the previous layers with
     `stale: true`, and `ArtDiffView` takes only their pixels (`withRasters`), never their metadata,
     since an old `changes` or `prev` would hide the property edit the refresh just found.
  2. Spinners and loading text wait 180 ms (`useDelayedFlag`), so a fast refresh shows none and a
     slow one shows them as before. The cost is a thumbnail that has no pixels yet stays blank for
     that long on a first load.
  3. A refresh that returns identical data (`sameValue`) keeps the old objects, so memoized rows and
     canvases don't re-render.

  `useWorkingDiff` also starts as loading when there is a file to diff. It started as not loading,
  and its one frame of that drew the "outside the layer stack" note.

A smaller change in the same commit: the Inspector's Name row now shows a renamed layer as a
highlighted was → now, like Visible, Opacity and Blend, instead of leaving the old name in the
Change row's sentence, which a new user missed.

None of the three has a regression test, since the frontend has no test runner. They were checked
with `tsc` and by reading the code, not by running them in the app.

See also: [`visual-diff-viewer.md`](../visual-diff-viewer.md) for how a diff is built now, and
[`performance.md`](../performance.md) for where the pixel comparison sits in the diff's cost.

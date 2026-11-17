# Bugs found and fixed during evaluation testing

Dates: 2026-11-04 to 2026-11-09. Commits: none yet. The changes below are uncommitted on top of
`3bb1cbf` ("Merge branch 'staging'").

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

Still open:
- **Which cases real Krita saves produce.** Comparing two saves from either side of a tap would
  show this.
- **Layer-subset staging.** A layer whose only change is off-canvas no longer appears as changed
  in the Changes panel. How a layer-subset version treats that layer hasn't been checked.
- **Committing.** None of this is committed. The app version is bumped to 2.3.0 for the build
  with the fix (`package.json`, `tauri.conf.json`, `Cargo.toml` and both lock files), but there is
  no release tag or release note yet.
- **Installing for all users.** Installing the 2.3.0 build showed that the NSIS setup `.exe` could
  not install into `C:\Program Files`: with no install mode set, Tauri builds a per-user installer
  that never asks for admin rights. `tauri.conf.json` now sets `bundle.windows.nsis.installMode` to
  `"both"`, so the installer asks whether to install for the current user or for all users. As a
  result it now asks for admin rights even for a per-user install. The MSI was already per-machine.

See also: [`visual-diff-viewer.md`](../visual-diff-viewer.md) for how a diff is built now, and
[`performance.md`](../performance.md) for where the pixel comparison sits in the diff's cost.

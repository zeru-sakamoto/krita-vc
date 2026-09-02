//! Layer-level partial staging: saving a version that holds only the layers the artist ticked.
//!
//! The unit under test is `stage::stage_kra` (synthesizing the `.kra`) plus `commit::commit_selected`'s
//! `layers` argument (storing it without lying to the scanner about what's on disk).

use krita_vc_lib::{branch, commit, error::KvcError, kra, raster, repo, scan, stage};
use std::collections::HashSet;

mod common;
use common::{kra_layered, pack_kra, tiled};

const BG: &str = "bg";
const LINES: &str = "lines";

fn keep(ids: &[&str]) -> HashSet<String> {
    ids.iter().map(|s| s.to_string()).collect()
}

fn sel(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|s| s.to_string()).collect()
}

/// Track `art.kra` at `bg`/`lines` and commit it, so there is a committed side to revert to.
fn setup(root: &std::path::Path, bg: i64, lines: i64) -> std::path::PathBuf {
    let path = root.join("art.kra");
    std::fs::write(&path, kra_layered(bg, lines)).unwrap();
    repo::Repo::init(&path).unwrap();
    let mut r = repo::Repo::open(&path).unwrap();
    commit::commit_snapshot(&mut r, "first", "t").unwrap();
    path
}

/// The bytes of one archive entry in the version stored at the current branch tip.
fn entry_at_tip(r: &repo::Repo, entry: &str) -> Option<Vec<u8>> {
    let tip = r.branches.tip().unwrap().to_string();
    let bytes = commit::file_at_commit(r, "art.kra", &tip).unwrap();
    kra::read_entry(&bytes, entry).ok()
}

fn entry_of(kra_bytes: &[u8], entry: &str) -> Option<Vec<u8>> {
    kra::read_entry(kra_bytes, entry).ok()
}

// --- stage_kra, on raw bytes ------------------------------------------------------------

/// The heart of it: an unticked layer comes back from the committed version, a ticked one keeps
/// the working pixels, and everything else in the document rides along from the working file.
#[test]
fn unticked_layers_revert_and_ticked_layers_are_kept() {
    let committed = kra_layered(10, 10);
    let working = kra_layered(20, 20); // both layers edited

    let out = stage::stage_kra(&working, &committed, &keep(&[LINES])).unwrap();

    assert_eq!(
        entry_of(&out, "img/layers/layer2"),
        entry_of(&working, "img/layers/layer2"),
        "the ticked layer must hold the working pixels"
    );
    assert_eq!(
        entry_of(&out, "img/layers/layer1"),
        entry_of(&committed, "img/layers/layer1"),
        "the unticked layer must be back at its committed pixels"
    );
}

/// The composite and the preview are Krita's renders of the whole stack and we can't redo them,
/// so a partial version ships neither rather than showing layers it doesn't contain.
#[test]
fn composite_and_preview_are_dropped() {
    let committed = pack_kra(&[
        ("mimetype", b"application/x-krita".to_vec()),
        ("maindoc.xml", common::maindoc_layered(10, 10)),
        ("img/layers/layer1", tiled(&[(0, 0, b"bg010")])),
        ("img/layers/layer2", tiled(&[(0, 0, b"ln010")])),
        ("mergedimage.png", b"not-really-a-png".to_vec()),
        ("preview.png", b"nor-this".to_vec()),
    ]);
    let working = pack_kra(&[
        ("mimetype", b"application/x-krita".to_vec()),
        ("maindoc.xml", common::maindoc_layered(20, 20)),
        ("img/layers/layer1", tiled(&[(0, 0, b"bg020")])),
        ("img/layers/layer2", tiled(&[(0, 0, b"ln020")])),
        ("mergedimage.png", b"working-composite".to_vec()),
        ("preview.png", b"working-preview".to_vec()),
    ]);

    let out = stage::stage_kra(&working, &committed, &keep(&[LINES])).unwrap();
    assert!(entry_of(&out, "mergedimage.png").is_none());
    assert!(entry_of(&out, "preview.png").is_none());
    // The document itself is intact.
    assert!(entry_of(&out, "maindoc.xml").is_some());
    assert!(entry_of(&out, "img/layers/layer1").is_some());
}

/// A layer added since the last version, left unticked, is simply not in this version — and
/// neither is its data file, or the archive would carry an orphan.
#[test]
fn an_unticked_added_layer_is_left_out_entirely() {
    let committed = kra_layered(10, 10);
    let working = pack_kra(&[
        ("mimetype", b"application/x-krita".to_vec()),
        (
            "maindoc.xml",
            br#"<!DOCTYPE DOC>
<DOC><IMAGE name="img" colorspacename="RGBA" width="128" height="128"><layers>
<layer name="Sketch" uuid="sketch" opacity="255" compositeop="normal" nodetype="paintlayer" filename="layer3"/>
<layer name="Lines" uuid="lines" opacity="10" compositeop="normal" nodetype="paintlayer" filename="layer2"/>
<layer name="Background" uuid="bg" opacity="10" compositeop="normal" nodetype="paintlayer" filename="layer1"/>
</layers></IMAGE></DOC>"#
                .to_vec(),
        ),
        ("img/layers/layer1", tiled(&[(0, 0, b"bg010")])),
        ("img/layers/layer2", tiled(&[(0, 0, b"ln010")])),
        ("img/layers/layer3", tiled(&[(0, 0, b"sketch")])),
    ]);

    let out = stage::stage_kra(&working, &committed, &keep(&[BG, LINES])).unwrap();
    let xml = String::from_utf8(entry_of(&out, "maindoc.xml").unwrap()).unwrap();
    assert!(!xml.contains("sketch"), "the added layer must not be saved");
    assert!(
        entry_of(&out, "img/layers/layer3").is_none(),
        "its data file must go with it — an orphan would be a file Krita loads oddly"
    );
    assert!(xml.contains("uuid=\"bg\"") && xml.contains("uuid=\"lines\""));
}

/// The mirror case: a layer *deleted* since the last version and left unticked means "don't save
/// that deletion", so it has to come back — with its pixels.
#[test]
fn an_unticked_deleted_layer_comes_back() {
    let committed = kra_layered(10, 10);
    let working = pack_kra(&[
        ("mimetype", b"application/x-krita".to_vec()),
        (
            "maindoc.xml",
            br#"<!DOCTYPE DOC>
<DOC><IMAGE name="img" colorspacename="RGBA" width="128" height="128"><layers>
<layer name="Lines" uuid="lines" opacity="20" compositeop="normal" nodetype="paintlayer" filename="layer2"/>
</layers></IMAGE></DOC>"#
                .to_vec(),
        ),
        ("img/layers/layer2", tiled(&[(0, 0, b"ln020")])),
    ]);

    // Tick only "lines" — the deletion of "bg" is not saved.
    let out = stage::stage_kra(&working, &committed, &keep(&[LINES])).unwrap();
    let xml = String::from_utf8(entry_of(&out, "maindoc.xml").unwrap()).unwrap();
    assert!(xml.contains("uuid=\"bg\""), "the deletion wasn't ticked");
    assert_eq!(
        entry_of(&out, "img/layers/layer1"),
        entry_of(&committed, "img/layers/layer1"),
        "and its pixels must come back with it"
    );

    // Tick it, and the deletion *is* saved.
    let out = stage::stage_kra(&working, &committed, &keep(&[BG, LINES])).unwrap();
    let xml = String::from_utf8(entry_of(&out, "maindoc.xml").unwrap()).unwrap();
    assert!(!xml.contains("uuid=\"bg\""));
}

/// Reverting a layer must not renumber it when nothing forces it to: the tile streams are keyed
/// `kra:{rel}:tile:{image}/layers/{layerN}:{x},{y}`, so a gratuitous rename would re-store every
/// reverted tile under fresh keys and lose dedup against the history it just came from.
#[test]
fn a_reverted_layer_keeps_its_data_file_name() {
    let committed = kra_layered(10, 10);
    let working = kra_layered(20, 20);
    let out = stage::stage_kra(&working, &committed, &keep(&[LINES])).unwrap();
    let xml = String::from_utf8(entry_of(&out, "maindoc.xml").unwrap()).unwrap();
    assert!(xml.contains("filename=\"layer1\""));
    assert!(entry_of(&out, "img/layers/layer1").is_some());
}

/// Reverting must not silently drop a layer whose committed data file name is already taken by a
/// layer that survived — Krita renumbers `layerN` between saves, so this really happens.
#[test]
fn a_colliding_data_file_name_is_reassigned_not_clobbered() {
    // Committed: "bg" owns layer1. Working: Krita renumbered, so "lines" now owns layer1 and
    // "bg" owns layer2. Ticking only "lines" means reverted "bg" wants layer1 — already in use.
    let committed = kra_layered(10, 10);
    let working = pack_kra(&[
        ("mimetype", b"application/x-krita".to_vec()),
        (
            "maindoc.xml",
            br#"<!DOCTYPE DOC>
<DOC><IMAGE name="img" colorspacename="RGBA" width="128" height="128"><layers>
<layer name="Lines" uuid="lines" opacity="20" compositeop="normal" nodetype="paintlayer" filename="layer1"/>
<layer name="Background" uuid="bg" opacity="20" compositeop="normal" nodetype="paintlayer" filename="layer2"/>
</layers></IMAGE></DOC>"#
                .to_vec(),
        ),
        ("img/layers/layer1", tiled(&[(0, 0, b"ln020")])),
        ("img/layers/layer2", tiled(&[(0, 0, b"bg020")])),
    ]);

    let out = stage::stage_kra(&working, &committed, &keep(&[LINES])).unwrap();
    let xml = String::from_utf8(entry_of(&out, "maindoc.xml").unwrap()).unwrap();

    // The ticked layer keeps the working file's pixels under its own name.
    assert_eq!(
        entry_of(&out, "img/layers/layer1"),
        entry_of(&working, "img/layers/layer1")
    );
    // The reverted layer got a fresh name, and its committed pixels came with it.
    let renamed = xml
        .split("uuid=\"bg\"")
        .nth(1)
        .and_then(|s| s.split("filename=\"").nth(1))
        .and_then(|s| s.split('"').next())
        .unwrap()
        .to_string();
    assert_ne!(renamed, "layer1", "must not collide with the kept layer");
    assert_eq!(
        entry_of(&out, &format!("img/layers/{renamed}")),
        entry_of(&committed, "img/layers/layer1"),
        "the reverted layer's committed pixels must follow it to the new name"
    );
}

/// Copied layer data is in the document's pixel format, so a color-space change would produce a
/// file Krita can't open. Refuse, writing nothing, rather than emit one.
#[test]
fn a_color_space_change_is_refused() {
    let committed = kra_layered(10, 10);
    let working = pack_kra(&[
        ("mimetype", b"application/x-krita".to_vec()),
        (
            "maindoc.xml",
            br#"<!DOCTYPE DOC>
<DOC><IMAGE name="img" colorspacename="CMYK" width="128" height="128"><layers>
<layer name="Lines" uuid="lines" opacity="20" compositeop="normal" nodetype="paintlayer" filename="layer2"/>
<layer name="Background" uuid="bg" opacity="20" compositeop="normal" nodetype="paintlayer" filename="layer1"/>
</layers></IMAGE></DOC>"#
                .to_vec(),
        ),
        ("img/layers/layer1", tiled(&[(0, 0, b"bg020")])),
        ("img/layers/layer2", tiled(&[(0, 0, b"ln020")])),
    ]);
    assert!(matches!(
        stage::stage_kra(&working, &committed, &keep(&[LINES])),
        Err(KvcError::StageFailed(_))
    ));
}

// --- through commit_selected ------------------------------------------------------------

/// **The scan trap.** After a partial commit the artwork on disk is *not* what was saved, so it
/// has to keep scanning dirty. Recording the working file's size+mtime in the index would trip
/// `scan_detailed`'s fast path and the unticked layers would silently vanish from the Changes
/// panel — the one failure here that loses work rather than annoying someone.
#[test]
fn the_tree_is_still_dirty_after_a_partial_commit() {
    let dir = tempfile::tempdir().unwrap();
    let path = setup(dir.path(), 10, 10);

    std::fs::write(&path, kra_layered(20, 20)).unwrap();
    let mut r = repo::Repo::open(&path).unwrap();
    commit::commit_selected(&mut r, "just the lines", "t", None, Some(&sel(&[LINES]))).unwrap();

    // Re-open so the check runs against what was actually persisted, not in-memory state.
    let r = repo::Repo::open(&path).unwrap();
    let dirty = scan::scan(&r).unwrap();
    assert_eq!(
        dirty.len(),
        1,
        "the unticked layer is still unsaved, so the artwork must read as modified"
    );
    assert_eq!(dirty[0].1, "M");
}

/// …and it must reach that answer from a `stat`. The index used to record `size`/`mtime` as `0`
/// to force the scanner off its fast path, which made every later scan read and blake3 the whole
/// document — and `kvc status` is on the Krita docker's 1.5s poll, so that was a full read of a
/// possibly-hundreds-of-MB painting, twice a second, forever after one partial commit.
///
/// There's no portable way to assert "didn't read the file", so this pins the index shape the
/// cheap path depends on: real size/mtime (so the fast path can fire at all) plus the `partial`
/// flag (so firing it still yields "M").
#[test]
fn a_partial_commit_leaves_the_index_able_to_answer_from_a_stat() {
    let dir = tempfile::tempdir().unwrap();
    let path = setup(dir.path(), 10, 10);

    std::fs::write(&path, kra_layered(20, 20)).unwrap();
    let mut r = repo::Repo::open(&path).unwrap();
    commit::commit_selected(&mut r, "just the lines", "t", None, Some(&sel(&[LINES]))).unwrap();

    let r = repo::Repo::open(&path).unwrap();
    let tracked = r.index.files.get("art.kra").expect("still tracked");
    assert!(tracked.partial, "the committed version is a layer subset");

    let meta = std::fs::metadata(&path).unwrap();
    let (size, mtime) = repo::size_mtime(&meta);
    assert_eq!(
        (tracked.size, tracked.mtime),
        (size, mtime),
        "the index must describe the file on disk, or the fast path can never fire"
    );
    assert_ne!((tracked.size, tracked.mtime), (0, 0));
}

/// The flag is not sticky: saving the whole artwork afterwards clears it, so the document goes
/// back to being skippable on a size+mtime match.
#[test]
fn a_full_commit_clears_the_partial_flag() {
    let dir = tempfile::tempdir().unwrap();
    let path = setup(dir.path(), 10, 10);

    std::fs::write(&path, kra_layered(20, 20)).unwrap();
    let mut r = repo::Repo::open(&path).unwrap();
    commit::commit_selected(&mut r, "just the lines", "t", None, Some(&sel(&[LINES]))).unwrap();
    assert!(r.index.files["art.kra"].partial);

    commit::commit_snapshot(&mut r, "all of it", "t").unwrap();
    let r = repo::Repo::open(&path).unwrap();
    assert!(!r.index.files["art.kra"].partial);
    assert!(
        scan::scan(&r).unwrap().is_empty(),
        "clean after saving it all"
    );
}

/// The version that lands holds the ticked layer's new pixels and the unticked layer's old ones.
#[test]
fn a_partial_commit_stores_only_the_ticked_layer() {
    let dir = tempfile::tempdir().unwrap();
    let path = setup(dir.path(), 10, 10);
    let committed_first = std::fs::read(&path).unwrap();

    let working = kra_layered(20, 20);
    std::fs::write(&path, &working).unwrap();
    let mut r = repo::Repo::open(&path).unwrap();
    commit::commit_selected(&mut r, "just the lines", "t", None, Some(&sel(&[LINES]))).unwrap();

    assert_eq!(
        entry_at_tip(&r, "img/layers/layer2"),
        entry_of(&working, "img/layers/layer2")
    );
    assert_eq!(
        entry_at_tip(&r, "img/layers/layer1"),
        entry_of(&committed_first, "img/layers/layer1")
    );
}

/// Saving the rest afterwards reproduces the working file — a partial commit defers work, it
/// never destroys it.
#[test]
fn committing_the_remainder_afterwards_catches_up() {
    let dir = tempfile::tempdir().unwrap();
    let path = setup(dir.path(), 10, 10);

    let working = kra_layered(20, 20);
    std::fs::write(&path, &working).unwrap();
    let mut r = repo::Repo::open(&path).unwrap();
    commit::commit_selected(&mut r, "the lines", "t", None, Some(&sel(&[LINES]))).unwrap();
    commit::commit_snapshot(&mut r, "the rest", "t").unwrap();

    assert_eq!(
        entry_at_tip(&r, "img/layers/layer1"),
        entry_of(&working, "img/layers/layer1")
    );
    assert_eq!(
        entry_at_tip(&r, "img/layers/layer2"),
        entry_of(&working, "img/layers/layer2")
    );
    assert!(
        scan::scan(&repo::Repo::open(&path).unwrap())
            .unwrap()
            .is_empty(),
        "everything is saved now"
    );
}

/// Ticking only layers that didn't actually change stores nothing rather than an empty version.
#[test]
fn ticking_nothing_that_changed_is_nothing_to_commit() {
    let dir = tempfile::tempdir().unwrap();
    let path = setup(dir.path(), 10, 10);

    std::fs::write(&path, kra_layered(10, 20)).unwrap(); // only "lines" moved
    let mut r = repo::Repo::open(&path).unwrap();
    assert!(matches!(
        commit::commit_selected(&mut r, "nothing", "t", None, Some(&sel(&[BG]))),
        Err(KvcError::Nothing)
    ));
}

/// A first version has no committed side to revert the unticked layers to, so picking layers has
/// no meaning yet — say so instead of guessing.
#[test]
fn a_partial_first_version_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("art.kra");
    std::fs::write(&path, kra_layered(10, 10)).unwrap();
    repo::Repo::init(&path).unwrap();
    let mut r = repo::Repo::open(&path).unwrap();
    assert!(matches!(
        commit::commit_selected(&mut r, "first", "t", None, Some(&sel(&[LINES]))),
        Err(KvcError::StageFailed(_))
    ));
}

/// `materialize_kra` lifts unchanged entries straight out of the working file, which assumes the
/// working file matches the current manifest — a partial commit breaks that. It self-guards on
/// each entry's recorded crc32+size, so a switch after one still lands on the right bytes.
#[test]
fn a_branch_switch_after_a_partial_commit_lands_correctly() {
    let dir = tempfile::tempdir().unwrap();
    let path = setup(dir.path(), 10, 10);
    let first = std::fs::read(&path).unwrap();

    let mut r = repo::Repo::open(&path).unwrap();
    branch::create_branch(&mut r, "side", None).unwrap();

    std::fs::write(&path, kra_layered(20, 20)).unwrap();
    let mut r = repo::Repo::open(&path).unwrap();
    commit::commit_selected(&mut r, "the lines", "t", None, Some(&sel(&[LINES]))).unwrap();
    // The tree is deliberately still dirty after a partial commit, and a switch refuses that.
    commit::commit_snapshot(&mut r, "the rest", "t").unwrap();

    let mut r = repo::Repo::open(&path).unwrap();
    branch::switch_branch(&mut r, "main").unwrap();
    assert_eq!(
        entry_of(&std::fs::read(&path).unwrap(), "img/layers/layer1"),
        entry_of(&first, "img/layers/layer1"),
        "main never saw either edit"
    );
}

// --- the composited stand-in for a missing mergedimage.png -----------------------------

/// A solid `w`x`h` RGBA PNG.
fn solid(w: u32, h: u32, px: [u8; 4]) -> Vec<u8> {
    let buf: Vec<u8> = std::iter::repeat(px)
        .take((w * h) as usize)
        .flatten()
        .collect();
    raster::rgba_to_png(&buf, w, h).unwrap()
}

fn first_px(png: &[u8]) -> [u8; 4] {
    let (rgba, _, _) = raster::decode_png_rgba(png).unwrap();
    [rgba[0], rgba[1], rgba[2], rgba[3]]
}

/// A partial version has no `mergedimage.png`, so the app composites the stack itself. This is
/// that compositor: opaque source-over, per-layer opacity, and the blend modes it models.
#[test]
fn the_stack_compositor_blends_bottom_to_top() {
    let red = solid(4, 4, [255, 0, 0, 255]);
    let blue = solid(4, 4, [0, 0, 255, 255]);

    // Opaque normal: the top layer wins outright.
    let out = raster::composite_stack(&[
        raster::StackLayer {
            png: &red,
            opacity: 1.0,
            blend: "normal",
        },
        raster::StackLayer {
            png: &blue,
            opacity: 1.0,
            blend: "normal",
        },
    ])
    .unwrap();
    assert_eq!(first_px(&out), [0, 0, 255, 255]);

    // Half-opacity top over opaque bottom: an even mix, alpha still opaque.
    let out = raster::composite_stack(&[
        raster::StackLayer {
            png: &red,
            opacity: 1.0,
            blend: "normal",
        },
        raster::StackLayer {
            png: &blue,
            opacity: 0.5,
            blend: "normal",
        },
    ])
    .unwrap();
    let px = first_px(&out);
    assert_eq!(px[3], 255);
    assert!((px[0] as i32 - 128).abs() <= 1, "got {px:?}");
    assert!((px[2] as i32 - 128).abs() <= 1, "got {px:?}");

    // Multiply: red x blue has no channel in common, so it goes black.
    let out = raster::composite_stack(&[
        raster::StackLayer {
            png: &red,
            opacity: 1.0,
            blend: "normal",
        },
        raster::StackLayer {
            png: &blue,
            opacity: 1.0,
            blend: "multiply",
        },
    ])
    .unwrap();
    assert_eq!(first_px(&out), [0, 0, 0, 255]);

    // A fully transparent layer leaves the backdrop alone.
    let clear = solid(4, 4, [0, 255, 0, 0]);
    let out = raster::composite_stack(&[
        raster::StackLayer {
            png: &red,
            opacity: 1.0,
            blend: "normal",
        },
        raster::StackLayer {
            png: &clear,
            opacity: 1.0,
            blend: "normal",
        },
    ])
    .unwrap();
    assert_eq!(first_px(&out), [255, 0, 0, 255]);

    // Nothing decodable in, nothing out — the caller falls back to showing no composite.
    assert!(raster::composite_stack(&[]).is_none());
}

/// End-to-end: a version saved from a layer subset has no `mergedimage.png`, and the diff for it
/// still produces an `afterImage` — the composited stand-in. Without this the Version Map draws a
/// blank frame for every partial version, since `VersionNode` renders `afterImage` and
/// `commit_diff` fetches no per-layer rasters of its own.
#[test]
fn a_partial_version_still_has_a_composite_for_the_map() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("art.kra");
    std::fs::write(
        &path,
        common::kra_painted([255, 0, 0, 255], [0, 0, 255, 255]),
    )
    .unwrap();
    repo::Repo::init(&path).unwrap();
    let mut r = repo::Repo::open(&path).unwrap();
    commit::commit_snapshot(&mut r, "first", "t").unwrap();

    // Repaint both layers, save only the top one.
    std::fs::write(
        &path,
        common::kra_painted([0, 255, 0, 255], [255, 255, 0, 255]),
    )
    .unwrap();
    let mut r = repo::Repo::open(&path).unwrap();
    let c =
        commit::commit_selected(&mut r, "just the lines", "t", None, Some(&sel(&[LINES]))).unwrap();

    let file = c.files.iter().find(|f| f.path == "art.kra").unwrap();
    // The stored version really doesn't carry one — that's the whole reason the fallback exists.
    let stored = commit::file_at_commit(&r, "art.kra", &c.id).unwrap();
    assert!(entry_of(&stored, "mergedimage.png").is_none());

    let art = krita_vc_lib::commands::committed_art_dto(&r, file, None, false, None).unwrap();
    assert!(
        art.after_image.is_some(),
        "a partial version must still get a composite, or its Version Map node is blank"
    );
}

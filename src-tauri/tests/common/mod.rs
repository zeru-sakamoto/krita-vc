//! `.kra` fixtures shared by the integration test binaries.
//!
//! These live here rather than inline in `engine.rs` because `backup_store_root.rs` has to be a
//! *separate* test binary — it mutates the app-global custom store root, which is process-global
//! state every other test's `store_dir_for` would otherwise resolve through.

// Each test binary uses a different subset.
#![allow(dead_code)]

use std::io::Write;

/// Build a real Krita-style tiled layer block.
pub fn tiled(items: &[(i64, i64, &[u8])]) -> Vec<u8> {
    let mut out = format!(
        "VERSION 2\nTILEWIDTH 64\nTILEHEIGHT 64\nPIXELSIZE 4\nDATA {}\n",
        items.len()
    )
    .into_bytes();
    for (x, y, d) in items {
        out.extend_from_slice(format!("{x},{y},LZF,{}\n", d.len()).as_bytes());
        out.extend_from_slice(d);
    }
    out
}

/// Pack a minimal but valid .kra ZIP (mimetype stored first, like Krita writes it).
pub fn pack_kra(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
    use zip::write::SimpleFileOptions;
    use zip::CompressionMethod;
    let mut out = Vec::new();
    {
        let mut zw = zip::ZipWriter::new(std::io::Cursor::new(&mut out));
        for (name, data) in entries {
            let method = if *name == "mimetype" {
                CompressionMethod::Stored
            } else {
                CompressionMethod::Deflated
            };
            zw.start_file(
                *name,
                SimpleFileOptions::default().compression_method(method),
            )
            .unwrap();
            zw.write_all(data).unwrap();
        }
        zw.finish().unwrap();
    }
    out
}

pub fn maindoc(lines_opacity: i64) -> Vec<u8> {
    format!(
        r#"<!DOCTYPE DOC>
<DOC><IMAGE name="img"><layers>
<layer name="Background" uuid="bg" opacity="255" compositeop="normal" nodetype="paintlayer"/>
<layer name="Lines" uuid="lines" opacity="{lines_opacity}" compositeop="normal" nodetype="paintlayer"/>
</layers></IMAGE></DOC>"#
    )
    .into_bytes()
}

/// A `maindoc.xml` whose two top-level paint layers each own a data file — the shape layer-subset
/// staging needs. [`maindoc`]'s two layers share one data file and carry no `filename`, which is
/// fine for the storage/dedup tests but leaves nothing to pick between. Krita writes the stack
/// top-first, so "Lines" precedes "Background".
pub fn maindoc_layered(bg_opacity: i64, lines_opacity: i64) -> Vec<u8> {
    format!(
        r#"<!DOCTYPE DOC>
<DOC><IMAGE name="img" colorspacename="RGBA" width="128" height="128"><layers>
<layer name="Lines" uuid="lines" opacity="{lines_opacity}" compositeop="normal" nodetype="paintlayer" filename="layer2"/>
<layer name="Background" uuid="bg" opacity="{bg_opacity}" compositeop="normal" nodetype="paintlayer" filename="layer1"/>
</layers></IMAGE></DOC>"#
    )
    .into_bytes()
}

/// One 64x64 tile of a solid RGBA color, encoded the way Krita stores it: planar B,G,R,A planes
/// behind a compression flag byte. [`tiled`]'s header declares `TILEWIDTH/TILEHEIGHT 64` and
/// `PIXELSIZE 4`, so this is what a payload has to look like for `raster::tile_to_rgba` to decode
/// it — the fake `b"bg010"` payloads elsewhere are fine for storage tests but decode to nothing,
/// so anything that actually rasterizes needs this instead.
pub fn solid_tile(rgba: [u8; 4]) -> Vec<u8> {
    const N: usize = 64 * 64;
    let mut planar = Vec::with_capacity(N * 4);
    for ch in [rgba[2], rgba[1], rgba[0], rgba[3]] {
        planar.extend(std::iter::repeat_n(ch, N));
    }
    krita_vc_lib::raster::tile_from_planar(&planar)
}

/// [`kra_layered`]'s shape, but with layer data that really decodes to pixels: `bg` fills the
/// bottom layer, `lines` the top. For tests that rasterize or composite.
pub fn kra_painted(bg: [u8; 4], lines: [u8; 4]) -> Vec<u8> {
    pack_kra(&[
        ("mimetype", b"application/x-krita".to_vec()),
        ("maindoc.xml", maindoc_layered(255, 255)),
        ("img/layers/layer1", tiled(&[(0, 0, &solid_tile(bg))])),
        ("img/layers/layer2", tiled(&[(0, 0, &solid_tile(lines))])),
    ])
}

/// A `.kra` with two independently-editable top-level layers: each parameter drives its own
/// layer's tile payload, so "which layer changed?" is answerable from the bytes alone.
pub fn kra_layered(bg_opacity: i64, lines_opacity: i64) -> Vec<u8> {
    pack_kra(&[
        ("mimetype", b"application/x-krita".to_vec()),
        ("maindoc.xml", maindoc_layered(bg_opacity, lines_opacity)),
        (
            "img/layers/layer1",
            tiled(&[(0, 0, format!("bg{bg_opacity:03}").as_bytes())]),
        ),
        (
            "img/layers/layer2",
            tiled(&[(0, 0, format!("ln{lines_opacity:03}").as_bytes())]),
        ),
    ])
}

/// A tiny but *real* `.kra`. Tracked documents can't be arbitrary bytes the way the old `.gpl`
/// fixtures were — the commit path parses a `.kra` as a zip.
pub fn kra_bytes(lines_opacity: i64) -> Vec<u8> {
    // Shaped like a real document, not a one-entry stub: a mimetype, a maindoc, and a tiled
    // layer. That matters because a `.kra` is stored as one stream per zip entry (and one per
    // tile) — a stub yields a single content object, too few for the dedup, storage-stats and
    // corruption tests to say anything. The tile payloads vary with `lines_opacity` so two
    // revisions genuinely differ and something is left to delta.
    let a = format!("tileA{lines_opacity:03}").into_bytes();
    let b = format!("tileB{lines_opacity:03}").into_bytes();
    pack_kra(&[
        ("mimetype", b"application/x-krita".to_vec()),
        ("maindoc.xml", maindoc(lines_opacity)),
        ("img/layers/layer1", tiled(&[(0, 0, &a), (0, 64, &b)])),
    ])
}

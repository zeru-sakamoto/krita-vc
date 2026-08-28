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

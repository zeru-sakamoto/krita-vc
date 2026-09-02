//! Synthesizing the `.kra` for a **layer-subset commit** — the artist ticked some of the layers
//! that changed, and only those should land in the version.
//!
//! The store versions whole documents, so "save these layers" has to become a real `.kra`. This
//! builds it from the **working** file by reverting every *unticked* top-level layer to the form
//! it has in the committed version. Everything outside the layer stack — canvas size, animation,
//! document settings, embedded palettes — comes from the working file unconditionally: a version
//! is "the document, minus some layers", and reverting `<IMAGE>` attributes and animation blocks
//! is a separate surgery this deliberately doesn't do.
//!
//! **Top-level only.** A group layer is one child of `<layers>`, so it is taken or left whole.
//! That is the grain [`crate::merge`] already speaks, and it is what makes it impossible to emit
//! XML referencing a data file that wasn't copied. Recursing would mean partial groups, mask
//! handling and ancestor forcing — every one able to produce a `.kra` Krita won't open, discovered
//! by the artist in their art, later.
//!
//! Two deliberate differences from [`crate::merge::merge_layers`], which folds a set-aside
//! version's layers *on top of* the working stack:
//!
//! - **uuids are never remapped.** A merge is adding a second copy of a layer, so it must mint a
//!   fresh identity. Staging *substitutes* the same layer, and preserving its uuid is exactly what
//!   makes the next diff recognise it.
//! - **Committed `layerN` filenames are kept wherever they can be**, and renamed only on a real
//!   collision with a layer that survived from the working file. Tile streams are keyed
//!   `kra:{rel}:tile:{image}/layers/{layerN}:{x},{y}` (`kra::tile_key`), so renaming
//!   unconditionally — which `merge.rs` does, correctly, for its own case — would re-store every
//!   reverted layer's tiles under fresh keys and lose dedup against the very history they came
//!   from. Substitution usually collides with nothing at all.
//!
//! The composite is **omitted**: see [`stage_kra`].

use crate::error::{KvcError, Result};
use crate::merge::{
    archive_layer_files, attr, image_node_with, layers_insert_at, layers_node_with, opts,
    read_entry_with, subtree_attr, zip_err,
};
use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Write};
use zip::ZipArchive;

/// Krita's document composite. Dropped from a partial version — see [`stage_kra`].
const COMPOSITE_ENTRY: &str = "mergedimage.png";
/// Krita's small thumbnail, dropped for the same reason as the composite.
const PREVIEW_ENTRY: &str = "preview.png";

fn fail(msg: &str) -> KvcError {
    KvcError::StageFailed(msg.to_string())
}

/// Build the `.kra` for a commit that saves only the layers in `keep`.
///
/// `working` is the file on disk, `committed` the version at the branch tip, and `keep` holds the
/// **top-level** layer ids the artist ticked (see [`layer_key`] for what an id is). Every other
/// top-level layer comes back from `committed`; a layer added in the working file but unticked is
/// dropped, and a layer deleted in the working file but unticked is put back.
///
/// **`mergedimage.png` and `preview.png` are deliberately not written.** They are Krita's renders
/// of the whole stack and we can't redo them, so carrying the working file's copies would ship a
/// preview showing layers this version doesn't contain — visible in the artist's own file manager
/// once the version is restored. Both are regenerable: Krita rewrites them on the next save, and
/// the app composites its own capped preview on demand (`raster::composite_stack`, reached from
/// `commands::composite_data_url` when a manifest has no composite entry).
///
/// Refuses with [`KvcError::StageFailed`] — writing nothing — rather than emit a file Krita can't
/// open: a color-space mismatch between the two versions, or a `maindoc.xml` that is missing,
/// non-UTF-8, unparseable, or has no `<IMAGE>`/`<layers>`.
pub fn stage_kra(working: &[u8], committed: &[u8], keep: &HashSet<String>) -> Result<Vec<u8>> {
    let work_doc = read_entry_with(working, "maindoc.xml", fail)?;
    let comm_doc = read_entry_with(committed, "maindoc.xml", fail)?;
    let work_xml = std::str::from_utf8(&work_doc).map_err(|_| fail("maindoc.xml is not UTF-8"))?;
    let comm_xml = std::str::from_utf8(&comm_doc)
        .map_err(|_| fail("the saved version's maindoc.xml is not UTF-8"))?;

    let opt = roxmltree::ParsingOptions {
        allow_dtd: true,
        ..Default::default()
    };
    let work_tree = roxmltree::Document::parse_with_options(work_xml, opt)
        .map_err(|e| fail(&format!("maindoc: {e}")))?;
    let comm_tree = roxmltree::Document::parse_with_options(comm_xml, opt)
        .map_err(|e| fail(&format!("saved version's maindoc: {e}")))?;

    let work_image = image_node_with(&work_tree, fail)?;
    let comm_image = image_node_with(&comm_tree, fail)?;

    // Same rule as the set-aside merge: copied layer data is in the document's pixel format, so a
    // different color space would produce a file Krita won't open. A canvas *size* difference is
    // fine — the reverted layers may simply sit at an offset, which Krita opens.
    let work_cs = work_image.attribute("colorspacename").unwrap_or("");
    let comm_cs = comm_image.attribute("colorspacename").unwrap_or("");
    if work_cs != comm_cs {
        return Err(fail(&format!(
            "this artwork's color space changed ({} → {}) — save the whole artwork instead",
            or_q(comm_cs),
            or_q(work_cs)
        )));
    }

    let work_name = work_image.attribute("name").unwrap_or("");
    let comm_name = comm_image.attribute("name").unwrap_or("");
    let work_layers = layers_node_with(work_image, fail)?;
    let comm_layers = layers_node_with(comm_image, fail)?;

    let work_top: Vec<roxmltree::Node> = top_layers(work_layers).collect();
    let comm_top: Vec<roxmltree::Node> = top_layers(comm_layers).collect();
    let comm_by_key: HashMap<String, roxmltree::Node> =
        comm_top.iter().map(|n| (layer_key(*n), *n)).collect();
    let work_keys: HashSet<String> = work_top.iter().map(|n| layer_key(*n)).collect();

    // The output stack, in order. Working order is the artist's own stacking order, so it drives.
    let mut pieces: Vec<(String, Piece)> = Vec::new();
    for n in &work_top {
        let key = layer_key(*n);
        if keep.contains(&key) {
            pieces.push((key, Piece::Working(*n)));
        } else if let Some(c) = comm_by_key.get(&key) {
            pieces.push((key, Piece::Committed(*c)));
        }
        // Otherwise: added in the working file and not ticked — it simply isn't in this version.
    }

    // A layer *deleted* in the working file and left unticked means "don't save that deletion", so
    // it has to come back. Its position is gone from the working order, so take it from the
    // committed one: right after whichever committed predecessor is still in the stack. Walking
    // committed order ascending means earlier re-insertions are already placed when later ones
    // look for their predecessor.
    for (ci, cn) in comm_top.iter().enumerate() {
        let key = layer_key(*cn);
        if work_keys.contains(&key) || keep.contains(&key) {
            continue;
        }
        let at = comm_top[..ci]
            .iter()
            .rev()
            .find_map(|p| {
                let pk = layer_key(*p);
                pieces.iter().position(|(k, _)| *k == pk)
            })
            .map_or(0, |i| i + 1);
        pieces.insert(at, (key, Piece::Committed(*cn)));
    }

    // Data files belonging to layers kept verbatim from the working file. Everything else under
    // `layers/` is dropped on the way out, and reverted layers are copied in from `committed`.
    let mut surviving: HashSet<String> = HashSet::new();
    for (_, p) in &pieces {
        if let Piece::Working(n) = p {
            surviving.extend(subtree_attr(*n, "filename"));
        }
    }

    // Names a freshly-minted `layerN` must avoid — both archives in full, not just the maindocs,
    // so an orphaned data file can't be collided with either.
    let mut taken: HashSet<String> = archive_layer_files(working, work_name)?;
    taken.extend(archive_layer_files(committed, comm_name)?);
    let mut next_fn = 1 + taken
        .iter()
        .filter_map(|f| f.strip_prefix("layer").and_then(|n| n.parse::<u64>().ok()))
        .max()
        .unwrap_or(0);

    let mut assigned: HashSet<String> = HashSet::new();
    // (name in the committed archive, name in the output) for every reverted layer's data files.
    let mut renames: Vec<(String, String)> = Vec::new();

    let mut frags: Vec<String> = Vec::with_capacity(pieces.len());
    for (_, piece) in &pieces {
        match piece {
            Piece::Working(n) => frags.push(work_xml[n.range()].to_string()),
            Piece::Committed(n) => {
                let mut frag = comm_xml[n.range()].to_string();
                for old in subtree_attr(*n, "filename") {
                    // Keep the committed name unless it would land on a layer that survived from
                    // the working file (or on a name already handed out) — see the module doc for
                    // why keeping it matters.
                    let new = if surviving.contains(&old) || assigned.contains(&old) {
                        loop {
                            let cand = format!("layer{next_fn}");
                            next_fn += 1;
                            if !taken.contains(&cand) && !assigned.contains(&cand) {
                                break cand;
                            }
                        }
                    } else {
                        old.clone()
                    };
                    if new != old {
                        frag = frag.replace(&attr("filename", &old), &attr("filename", &new));
                    }
                    assigned.insert(new.clone());
                    renames.push((old, new));
                }
                frags.push(frag);
            }
        }
    }

    // Replace the whole run of top-level <layer> children. Krita writes nothing else inside
    // <layers>, so the only thing between the first and last child is whitespace.
    // ponytail: assumes `<layers>` has a closing tag — a self-closed (empty) stack is refused
    // just below rather than special-cased, since an artwork with no layers isn't a thing to stage.
    let lr = work_layers.range();
    if !work_xml[lr.clone()].ends_with("</layers>") {
        return Err(fail("this artwork has no layers to choose from"));
    }
    let insert_at = layers_insert_at(work_xml, work_layers, fail)?;
    let end_at = work_top.last().map_or(insert_at, |n| n.range().end);

    let mut out_xml =
        String::with_capacity(work_xml.len() + frags.iter().map(String::len).sum::<usize>());
    out_xml.push_str(&work_xml[..insert_at]);
    for frag in &frags {
        out_xml.push('\n');
        out_xml.push_str(frag);
    }
    // No separator after the last fragment: `end_at` is the end of the last original `<layer>`
    // tag, so the tail already carries whatever whitespace preceded `</layers>`. Adding one here
    // made a staged document that reverted *everything* differ from the committed one by a single
    // newline — enough to give it a different `maindoc.xml` blob and therefore a different
    // manifest hash, which is what `commit_selected` compares to detect a no-op commit.
    out_xml.push_str(&work_xml[end_at..]);

    repackage(
        working, committed, &out_xml, work_name, comm_name, &surviving, &renames,
    )
}

/// Where each output layer's XML and data files come from.
enum Piece<'a> {
    /// Ticked — kept exactly as the working file has it.
    Working(roxmltree::Node<'a, 'a>),
    /// Unticked — reverted to the committed version.
    Committed(roxmltree::Node<'a, 'a>),
}

/// The `<layer>` element children of a `<layers>` node — the top-level stack, groups included as
/// single units.
fn top_layers<'a>(
    layers: roxmltree::Node<'a, 'a>,
) -> impl Iterator<Item = roxmltree::Node<'a, 'a>> {
    layers
        .children()
        .filter(|n| n.is_element() && n.has_tag_name("layer"))
}

/// The identity a layer is ticked by. Mirrors `commands::layer_id` — the frontend sends back
/// `LayerDto.id`, so keying differently here would silently fail to match a tick on a document
/// whose layers carry no uuid.
fn layer_key(l: roxmltree::Node) -> String {
    match l.attribute("uuid") {
        Some(u) if !u.is_empty() => u.to_string(),
        _ => l.attribute("name").unwrap_or("").to_string(),
    }
}

fn or_q(s: &str) -> &str {
    if s.is_empty() {
        "?"
    } else {
        s
    }
}

/// Rebuild the archive: the working file's entries (minus the composite, the preview, and the data
/// files of layers that didn't survive), with `maindoc.xml` swapped for the staged stack, then the
/// reverted layers' data files copied out of the committed version.
fn repackage(
    working: &[u8],
    committed: &[u8],
    maindoc: &str,
    work_image: &str,
    comm_image: &str,
    surviving: &HashSet<String>,
    renames: &[(String, String)],
) -> Result<Vec<u8>> {
    let mut wz = ZipArchive::new(Cursor::new(working)).map_err(zip_err)?;
    let mut cz = ZipArchive::new(Cursor::new(committed)).map_err(zip_err)?;
    let work_prefix = format!("{work_image}/layers/");
    let comm_prefix = format!("{comm_image}/layers/");

    let mut out = Vec::new();
    {
        let mut zw = zip::ZipWriter::new(Cursor::new(&mut out));

        for i in 0..wz.len() {
            let mut f = wz.by_index(i).map_err(zip_err)?;
            if f.is_dir() {
                continue;
            }
            let name = f.name().to_string();
            if name == COMPOSITE_ENTRY || name == PREVIEW_ENTRY {
                continue;
            }
            // Drop the data files of every layer that was reverted or removed. The `['.', '/']`
            // split is `merge::repackage`'s: it carries `layer2.defaultpixel` and
            // `layer2.shapelayer/content.svg` along with their base filename.
            if let Some(rest) = name.strip_prefix(&work_prefix) {
                let split = rest.find(['.', '/']).unwrap_or(rest.len());
                if !surviving.contains(&rest[..split]) {
                    continue;
                }
            }
            zw.start_file(&name, opts(&name)).map_err(zip_err)?;
            if name == "maindoc.xml" {
                zw.write_all(maindoc.as_bytes())?;
            } else {
                let buf = crate::repo::read_entry_capped(&mut f)?;
                zw.write_all(&buf)?;
            }
        }

        let rename: HashMap<&str, &str> = renames
            .iter()
            .map(|(o, n)| (o.as_str(), n.as_str()))
            .collect();
        for i in 0..cz.len() {
            let mut f = cz.by_index(i).map_err(zip_err)?;
            if f.is_dir() {
                continue;
            }
            let name = f.name().to_string();
            let Some(rest) = name.strip_prefix(&comm_prefix) else {
                continue;
            };
            let split = rest.find(['.', '/']).unwrap_or(rest.len());
            let Some(&new_fn) = rename.get(&rest[..split]) else {
                continue;
            };
            let new_name = format!("{work_prefix}{new_fn}{}", &rest[split..]);
            let buf = crate::repo::read_entry_capped(&mut f)?;
            zw.start_file(&new_name, opts(&new_name)).map_err(zip_err)?;
            zw.write_all(&buf)?;
        }

        zw.finish().map_err(zip_err)?;
    }
    Ok(out)
}

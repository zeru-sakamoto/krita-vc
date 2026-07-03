//! Thin Tauri command wrappers. Heavy I/O and binary diffing run on the blocking pool
//! (`spawn_blocking`) so the webview thread stays responsive; engine errors are flattened
//! to strings for the frontend. DTOs use camelCase to match `src/types.ts`.

use crate::error::{KvcError, Result};
use crate::kra::{self, LayerDiff, LayerNode};
use crate::repo::{Commit, CommittedFile, Repo};
use crate::{commit, scan};
use rayon::prelude::*;
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    pub path: String,
    pub status: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkingChange {
    pub change: FileChange,
    pub staged: bool,
}

/// Run blocking engine work off the UI thread, flattening both join and engine errors.
async fn run<T, F>(f: F) -> std::result::Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn init_repository(path: String) -> std::result::Result<(), String> {
    run(move || Repo::init(Path::new(&path))).await
}

#[tauri::command]
pub async fn is_repository(path: String) -> std::result::Result<bool, String> {
    run(move || Ok(Repo::is_repo(Path::new(&path)))).await
}

/// Validate `.kvc/` and load its state (returns nothing — success means it opened).
#[tauri::command]
pub async fn open_repository(path: String) -> std::result::Result<(), String> {
    run(move || Repo::open(Path::new(&path)).map(|_| ())).await
}

/// Permanently delete a repository folder and everything in it (guarded by `is_repo`).
#[tauri::command]
pub async fn delete_repository(path: String) -> std::result::Result<(), String> {
    run(move || Repo::delete(Path::new(&path))).await
}

#[tauri::command]
pub async fn scan_repository(path: String) -> std::result::Result<Vec<WorkingChange>, String> {
    run(move || {
        let repo = Repo::open_light(Path::new(&path))?;
        Ok(scan::scan(&repo)?
            .into_iter()
            .map(|(path, status)| WorkingChange {
                change: FileChange { path, status },
                staged: false,
            })
            .collect())
    })
    .await
}

#[tauri::command]
pub async fn commit_snapshot(
    path: String,
    message: String,
    author: String,
) -> std::result::Result<Commit, String> {
    run(move || {
        let mut repo = Repo::open(Path::new(&path))?;
        commit::commit_snapshot(&mut repo, &message, &author)
    })
    .await
}

/// Commits reachable from the current branch tip, in stored (topological) order — commits
/// on merged branches appear, commits unique to other branches don't.
#[tauri::command]
pub async fn list_commits(path: String) -> std::result::Result<Vec<Commit>, String> {
    run(move || {
        let repo = Repo::open_light(Path::new(&path))?;
        let reach = match repo.branches.tip() {
            Some(tip) => commit::ancestors(&repo.commits, tip),
            None => return Ok(Vec::new()),
        };
        Ok(repo
            .commits
            .iter()
            .filter(|c| reach.contains(&c.id))
            .cloned()
            .collect())
    })
    .await
}

// --- branches ---------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchDto {
    pub name: String,
    pub tip: Option<String>,
    pub current: bool,
}

fn branch_dtos(repo: &Repo) -> Vec<BranchDto> {
    repo.branches
        .branches
        .iter()
        .map(|(name, tip)| BranchDto {
            name: name.clone(),
            tip: (!tip.is_empty()).then(|| tip.clone()),
            current: *name == repo.branches.current,
        })
        .collect()
}

#[tauri::command]
pub async fn list_branches(path: String) -> std::result::Result<Vec<BranchDto>, String> {
    run(move || {
        let repo = Repo::open_light(Path::new(&path))?;
        Ok(branch_dtos(&repo))
    })
    .await
}

/// Create a branch and switch to it. Without `base` it starts at the current tip (instant —
/// the tree is identical); with a different `base` branch it materializes that branch's tree,
/// which needs the full repo (chains) and a clean working tree.
#[tauri::command]
pub async fn create_branch(
    path: String,
    name: String,
    base: Option<String>,
) -> std::result::Result<Vec<BranchDto>, String> {
    run(move || {
        let mut repo = if base.is_some() {
            Repo::open(Path::new(&path))?
        } else {
            Repo::open_light(Path::new(&path))?
        };
        crate::branch::create_branch(&mut repo, &name, base.as_deref())?;
        Ok(branch_dtos(&repo))
    })
    .await
}

/// Switch the working tree to `name`; rewrites only files that differ between the branches.
#[tauri::command]
pub async fn switch_branch(
    path: String,
    name: String,
) -> std::result::Result<Vec<BranchDto>, String> {
    run(move || {
        let mut repo = Repo::open(Path::new(&path))?;
        crate::branch::switch_branch(&mut repo, &name)?;
        Ok(branch_dtos(&repo))
    })
    .await
}

/// Merge `source` into the current branch (fast-forward or two-parent merge commit).
#[tauri::command]
pub async fn merge_branch(
    path: String,
    source: String,
    author: String,
) -> std::result::Result<Commit, String> {
    run(move || {
        let mut repo = Repo::open(Path::new(&path))?;
        crate::branch::merge_branch(&mut repo, &source, &author)
    })
    .await
}

#[tauri::command]
pub async fn delete_branch(
    path: String,
    name: String,
) -> std::result::Result<Vec<BranchDto>, String> {
    run(move || {
        let mut repo = Repo::open_light(Path::new(&path))?;
        crate::branch::delete_branch(&mut repo, &name)?;
        Ok(branch_dtos(&repo))
    })
    .await
}

/// Report layer metadata changes (opacity/blend/name, added/removed) for a .kra
/// between two commits, by diffing each version's maindoc.xml.
#[tauri::command]
pub async fn layer_diff(
    path: String,
    file: String,
    old_commit: String,
    new_commit: String,
) -> std::result::Result<Vec<LayerDiff>, String> {
    run(move || {
        let repo = Repo::open(Path::new(&path))?;
        // Pull just maindoc.xml out of each side's manifest — rebuilding the whole archive
        // (every tile of every layer) for one small entry dominated this command's cost.
        let maindoc = |commit_id: &str| -> Result<Vec<u8>> {
            let tree = commit::tree_at_commit(&repo.commits, commit_id)
                .ok_or_else(|| KvcError::NoCommit(commit_id.to_string()))?;
            let f = tree
                .get(&file)
                .ok_or_else(|| KvcError::NotTracked(file.clone()))?;
            let hash = f
                .content
                .as_deref()
                .ok_or_else(|| KvcError::NotTracked(format!("{file} (deleted)")))?;
            let manifest = kra::load_manifest(&repo, &file, hash)?;
            kra::entry_bytes(&repo, &file, &manifest, "maindoc.xml")?
                .ok_or_else(|| KvcError::CorruptZip("no maindoc.xml".into()))
        };
        kra::diff_maindoc(&maindoc(&old_commit)?, &maindoc(&new_commit)?)
    })
    .await
}

/// Restore the whole working tree to `commit_id` and record it as a new commit.
#[tauri::command]
pub async fn rollback_to_commit(
    path: String,
    commit_id: String,
    author: String,
) -> std::result::Result<Commit, String> {
    run(move || {
        let mut repo = Repo::open(Path::new(&path))?;
        match commit::rollback_to_commit(&mut repo, &commit_id, &author) {
            Err(crate::error::KvcError::Nothing) => Err(crate::error::KvcError::BadIndex(
                "already at this version".into(),
            )),
            other => other,
        }
    })
    .await
}

/// Undo the last commit, keeping working-tree changes. Returns the new head (or null).
#[tauri::command]
pub async fn undo_last_commit(path: String) -> std::result::Result<Option<Commit>, String> {
    run(move || {
        let mut repo = Repo::open(Path::new(&path))?;
        commit::undo_last_commit(&mut repo)
    })
    .await
}

/// Reconstruct `file` as of `commit_id` and write it back into the working tree.
#[tauri::command]
pub async fn restore_file(
    path: String,
    file: String,
    commit_id: String,
) -> std::result::Result<(), String> {
    run(move || {
        let repo = Repo::open(Path::new(&path))?;
        let bytes = commit::file_at_commit(&repo, &file, &commit_id)?;
        let target: PathBuf = repo.root.join(&file);
        std::fs::write(&target, bytes).map_err(|e| crate::error::io_at(&target, e))?;
        Ok(())
    })
    .await
}

// --- per-commit visual diff ------------------------------------------------------------
// DTOs mirror the frontend `DiffEntry` union in src/types.ts (serde camelCase). Art (.kra)
// files carry real per-layer PNG rasters + a composite; other files get a minimal text entry.

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegionDto {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct LayerDto {
    pub id: String,
    pub name: String,
    pub opacity: i64,
    pub blend_mode: String,
    pub change: String,
    /// Inner SVG `<image>` markup for each state, or null when the layer is absent then.
    pub before: Option<String>,
    pub after: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtDiffDto {
    pub path: String,
    pub status: String,
    pub width: u32,
    pub height: u32,
    pub layers: Vec<LayerDto>,
    pub regions: Vec<RegionDto>,
    /// Composite (mergedimage.png) for each state as `<image>` markup — the reliable composite.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before_image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_image: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextDiffDto {
    path: String,
    status: String,
    lines: Vec<serde_json::Value>,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DiffEntryDto {
    Art(ArtDiffDto),
    Text(TextDiffDto),
}

/// Krita opacity (0..255) → the UI's 0..100 scale.
fn to_percent(op: i64) -> i64 {
    ((op as f64) * 100.0 / 255.0).round() as i64
}

/// Krita compositeop → the UI's BlendMode union (unknown ops fall back to "normal").
fn blend_mode(op: &str) -> String {
    match op {
        "multiply" | "svg:multiply" => "multiply",
        "screen" | "svg:screen" => "screen",
        "overlay" | "svg:overlay" => "overlay",
        "add" | "linear_dodge" => "add",
        _ => "normal",
    }
    .to_string()
}

fn layer_id(l: &LayerNode) -> String {
    if l.uuid.is_empty() {
        l.name.clone()
    } else {
        l.uuid.clone()
    }
}

/// Composite (mergedimage.png) as a capped PNG data URL. The decode/resize/encode runs once per
/// unique composite — the result is disk-cached in `.kvc/cache/` keyed by the entry's content
/// hash, and on a hit `bytes` is never called (no reconstruct at all).
fn composite_data_url(
    repo: &Repo,
    content_hash: Option<String>,
    bytes: impl FnOnce() -> Result<Option<Vec<u8>>>,
) -> Result<Option<String>> {
    let cache_dir = repo.cache_dir();
    let key = content_hash.map(|h| kra::composite_cache_key(&h));
    if let Some(k) = &key {
        if let Some(png) = crate::raster::cache_read(&cache_dir, k) {
            return Ok(Some(crate::raster::png_bytes_to_data_url(&png)));
        }
    }
    let Some(b) = bytes()? else { return Ok(None) };
    let capped = crate::raster::cap_png(&b);
    if let Some(k) = &key {
        crate::raster::cache_write(&cache_dir, k, &capped);
    }
    Ok(Some(crate::raster::png_bytes_to_data_url(&capped)))
}

/// Build the visual diff for one `.kra` file: layer metadata + composite, and (only when
/// `with_rasters`) each layer's before/after PNG. The composite (mergedimage.png) and metadata
/// are cheap; the per-layer rasters are the expensive part, so the default per-commit diff omits
/// them (`with_rasters = false`) and the UI fetches them lazily via `commit_layers`/`working_layers`.
/// `on_layer` (raster path only) is called with each finished layer as rayon completes it —
/// out of order — so the UI can render layers progressively instead of waiting for the slowest.
pub fn art_diff_dto(
    repo: &Repo,
    path: &str,
    status: &str,
    new_src: &kra::KraSource,
    old: Option<&CommittedFile>,
    with_rasters: bool,
    on_layer: Option<&(dyn Fn(LayerDto) + Sync)>,
) -> Result<ArtDiffDto> {
    // Reconstruct + parse the old side's manifest ONCE; every layer/region/composite read below
    // reuses it instead of re-reconstructing (walking the patch chain) per call. The new side
    // is either a committed manifest (loaded once by the caller) or an in-memory working file.
    let old_manifest = match old.and_then(|o| o.content.as_deref()) {
        Some(h) => Some(kra::load_manifest(repo, path, h)?),
        None => None,
    };

    let new_meta = {
        let xml = new_src
            .entry_bytes(repo, path, "maindoc.xml")?
            .ok_or_else(|| KvcError::CorruptZip("no maindoc.xml".into()))?;
        kra::parse_image_meta(&xml)?
    };
    let (w, h) = (new_meta.width, new_meta.height);

    let old_meta = match &old_manifest {
        Some(m) => match kra::entry_bytes(repo, path, m, "maindoc.xml")? {
            Some(xml) => Some(kra::parse_image_meta(&xml)?),
            None => None,
        },
        None => None,
    };
    let new_tiles = new_src.tile_index();
    let old_tiles = old_manifest.as_ref().map(|m| m.tile_index());
    let changed_entries = old_tiles
        .as_ref()
        .map(|ot| kra::changed_entry_paths(ot, &new_tiles))
        .unwrap_or_default();

    // "meet" (not "none"): rasters keep their own aspect ratio, letterboxed inside the document
    // box — a before-side from a version with different canvas dimensions must not stretch.
    let img = |url: String| {
        format!("<image href=\"{url}\" x=\"0\" y=\"0\" width=\"{w}\" height=\"{h}\" preserveAspectRatio=\"xMidYMid meet\"/>")
    };

    // One tile cache for the whole request: before/after sides of a modified layer share most
    // tiles by content hash, so each shared tile reconstructs once.
    let tile_cache = crate::delta::TileCache::new();

    // Rasterize layers in parallel — each layer's decode/blit/PNG-encode is independent and only
    // reads &Repo. Order is preserved by par_iter's indexed collect.
    let mut layers: Vec<LayerDto> = new_meta
        .layers
        .par_iter()
        .map(|nl| -> Result<LayerDto> {
            let entry_path = format!("{}/layers/{}", new_meta.name, nl.filename);
            let ol = old_meta
                .as_ref()
                .and_then(|m| m.layers.iter().find(|o| layer_id(o) == layer_id(nl)));
            let change = if old_meta.is_none() || ol.is_none() {
                "added"
            } else {
                let meta_changed = ol
                    .map(|o| o.opacity != nl.opacity || o.blend != nl.blend || o.name != nl.name)
                    .unwrap_or(false);
                if meta_changed || changed_entries.contains(&entry_path) {
                    "modified"
                } else {
                    "unchanged"
                }
            };
            let (after, before) = if with_rasters {
                let after = new_src
                    .layer_raster(repo, path, &new_meta.name, &nl.filename, w, h, &tile_cache)?
                    .map(img);
                // An unchanged layer's pixels are identical on both sides — reuse the raster
                // instead of decoding + encoding it twice.
                let before = match (change, &old_manifest, &old_meta, ol) {
                    ("added", ..) => None,
                    ("unchanged", ..) => after.clone(),
                    (_, Some(om_manifest), Some(om), Some(o)) => kra::layer_raster(
                        repo,
                        path,
                        om_manifest,
                        &om.name,
                        &o.filename,
                        w,
                        h,
                        &tile_cache,
                    )?
                    .map(img),
                    _ => None,
                };
                (after, before)
            } else {
                (None, None)
            };
            let dto = LayerDto {
                id: layer_id(nl),
                name: nl.name.clone(),
                opacity: to_percent(nl.opacity),
                blend_mode: blend_mode(&nl.blend),
                change: change.into(),
                before,
                after,
            };
            if let Some(cb) = on_layer {
                cb(dto.clone());
            }
            Ok(dto)
        })
        .collect::<Result<Vec<_>>>()?;
    // Layers removed since the parent commit.
    if let (Some(om), Some(om_manifest)) = (&old_meta, &old_manifest) {
        for ol in &om.layers {
            if !new_meta
                .layers
                .iter()
                .any(|nl| layer_id(nl) == layer_id(ol))
            {
                let before = if with_rasters {
                    kra::layer_raster(
                        repo,
                        path,
                        om_manifest,
                        &om.name,
                        &ol.filename,
                        w,
                        h,
                        &tile_cache,
                    )?
                    .map(img)
                } else {
                    None
                };
                let dto = LayerDto {
                    id: layer_id(ol),
                    name: ol.name.clone(),
                    opacity: to_percent(ol.opacity),
                    blend_mode: blend_mode(&ol.blend),
                    change: "removed".into(),
                    before,
                    after: None,
                };
                if let Some(cb) = on_layer {
                    cb(dto.clone());
                }
                layers.push(dto);
            }
        }
    }
    layers.reverse(); // Krita writes top-first; the UI stacks bottom→top.

    let after_image = composite_data_url(repo, new_src.entry_hash("mergedimage.png"), || {
        new_src.entry_bytes(repo, path, "mergedimage.png")
    })?
    .map(&img);
    let before_image = match &old_manifest {
        Some(m) => composite_data_url(repo, m.entry_hash("mergedimage.png"), || {
            kra::entry_bytes(repo, path, m, "mergedimage.png")
        })?
        .map(&img),
        None => None,
    };
    let mut regions = Vec::new();
    if let Some(ot) = &old_tiles {
        if let Some((x, y, rw, rh)) = kra::changed_region(ot, &new_tiles, w, h) {
            regions.push(RegionDto {
                x,
                y,
                w: rw,
                h: rh,
                label: None,
            });
        }
    }

    Ok(ArtDiffDto {
        path: path.to_string(),
        status: status.to_string(),
        width: w.max(0) as u32,
        height: h.max(0) as u32,
        layers,
        regions,
        before_image,
        after_image,
    })
}

/// Minimal text placeholder for a file (non-.kra, deleted, or an .kra we couldn't raster).
fn text_entry(f: &CommittedFile) -> DiffEntryDto {
    DiffEntryDto::Text(TextDiffDto {
        path: f.path.clone(),
        status: f.status.clone(),
        lines: Vec::new(),
    })
}

/// The visual diff for one file: an art diff (metadata + composite, rasters only when
/// `with_rasters`) for a rasterable `.kra`, else a text placeholder. A failed `.kra` raster
/// degrades to a text entry rather than aborting the whole diff, so one unsupported/broken file
/// can't blank the entire panel.
fn diff_entry(
    repo: &Repo,
    f: &CommittedFile,
    old: Option<&CommittedFile>,
    with_rasters: bool,
) -> DiffEntryDto {
    if f.is_kra && f.status != "D" {
        committed_art_dto(repo, f, old, with_rasters, None)
            .map(DiffEntryDto::Art)
            .unwrap_or_else(|_| text_entry(f))
    } else {
        text_entry(f)
    }
}

/// Art diff for a committed `.kra`: load its manifest once, then run the shared builder.
pub fn committed_art_dto(
    repo: &Repo,
    f: &CommittedFile,
    old: Option<&CommittedFile>,
    with_rasters: bool,
    on_layer: Option<&(dyn Fn(LayerDto) + Sync)>,
) -> Result<ArtDiffDto> {
    let hash = f
        .content
        .as_deref()
        .ok_or_else(|| KvcError::NotTracked(format!("{} (no content)", f.path)))?;
    let manifest = kra::load_manifest(repo, &f.path, hash)?;
    art_diff_dto(
        repo,
        &f.path,
        &f.status,
        &kra::KraSource::Committed(&manifest),
        old,
        with_rasters,
        on_layer,
    )
}

/// The visual diff for a commit: one entry per changed file. `.kra` files render as art diffs
/// (composite + layer metadata; the heavy per-layer rasters are fetched lazily via `commit_layers`
/// so the panel appears immediately). Everything else is a minimal text entry.
/// ponytail: real line/palette diffs for non-.kra files are deferred — the focus is .kra fidelity.
#[tauri::command]
pub async fn commit_diff(
    path: String,
    commit_id: String,
) -> std::result::Result<Vec<DiffEntryDto>, String> {
    run(move || {
        let repo = Repo::open(Path::new(&path))?;
        let commit = repo
            .commits
            .iter()
            .find(|c| c.id == commit_id)
            .cloned()
            .ok_or_else(|| KvcError::NoCommit(commit_id.clone()))?;
        let parent_tree = commit
            .parents
            .first()
            .and_then(|p| commit::tree_at_commit(&repo.commits, p))
            .unwrap_or_default();

        Ok(commit
            .files
            .iter()
            .map(|f| diff_entry(&repo, f, parent_tree.get(&f.path), false))
            .collect())
    })
    .await
}

/// The per-layer rasters (before/after) for one `.kra` file in a commit — the expensive part of
/// the diff, loaded on demand after `commit_diff` has already shown the composite + layer list.
/// Each layer is **streamed** through `on_layer` the moment its rasters finish (out of order —
/// the frontend merges by layer id); the command resolving means every layer has been sent.
/// Sends nothing if the file isn't a rasterable `.kra` in that commit.
#[tauri::command]
pub async fn commit_layers(
    path: String,
    commit_id: String,
    file: String,
    on_layer: tauri::ipc::Channel<LayerDto>,
) -> std::result::Result<(), String> {
    run(move || {
        let repo = Repo::open(Path::new(&path))?;
        let commit = repo
            .commits
            .iter()
            .find(|c| c.id == commit_id)
            .cloned()
            .ok_or_else(|| KvcError::NoCommit(commit_id.clone()))?;
        let Some(f) = commit.files.iter().find(|f| f.path == file) else {
            return Ok(());
        };
        if !f.is_kra || f.status == "D" {
            return Ok(());
        }
        let parent_tree = commit
            .parents
            .first()
            .and_then(|p| commit::tree_at_commit(&repo.commits, p))
            .unwrap_or_default();
        committed_art_dto(
            &repo,
            f,
            parent_tree.get(&file),
            true,
            Some(&|dto| {
                let _ = on_layer.send(dto);
            }),
        )?;
        Ok(())
    })
    .await
}

/// Last-committed entry for `file` (the "before" side of a working diff), if any.
/// Head = the current branch tip, not `commits.last()` — after a switch the newest
/// commit in the vec can belong to another branch.
fn last_committed(repo: &Repo, file: &str) -> Option<CommittedFile> {
    repo.branches
        .tip()
        .and_then(|head| commit::tree_at_commit(&repo.commits, head))
        .and_then(|tree| tree.get(file).cloned())
}

/// Working-tree art diff, shared by `working_diff` and `working_layers`. Parses the working
/// `.kra` **in memory** (`parse_working`) — viewing a diff never touches the object store:
/// no bsdiff, no chain reconstructs, no writes. `None` old side (untracked file or empty
/// history) yields an all-"added" diff.
fn working_art_dto(
    repo: &Repo,
    file: &str,
    with_rasters: bool,
    on_layer: Option<&(dyn Fn(LayerDto) + Sync)>,
) -> Result<ArtDiffDto> {
    let abs = repo.root.join(file);
    let bytes = std::fs::read(&abs).map_err(|e| crate::error::io_at(&abs, e))?;
    let working = kra::parse_working(&bytes)?;
    let old = last_committed(repo, file);
    let status = if old.is_some() { "M" } else { "A" };
    art_diff_dto(
        repo,
        file,
        status,
        &kra::KraSource::Working(&working),
        old.as_ref(),
        with_rasters,
        on_layer,
    )
}

/// The visual diff for a single working-tree file vs its last committed version. Read-only:
/// the working `.kra` is diffed straight from memory, nothing is staged into the store.
#[tauri::command]
pub async fn working_diff(
    path: String,
    file: String,
) -> std::result::Result<Vec<DiffEntryDto>, String> {
    run(move || {
        let repo = Repo::open(Path::new(&path))?;
        if !file.to_lowercase().ends_with(".kra") {
            let old = last_committed(&repo, &file);
            return Ok(vec![text_entry(&CommittedFile {
                path: file.clone(),
                status: if old.is_some() {
                    "M".into()
                } else {
                    "A".into()
                },
                content: None,
                is_kra: false,
            })]);
        }
        let entry = working_art_dto(&repo, &file, false, None)
            .map(DiffEntryDto::Art)
            .unwrap_or_else(|_| {
                text_entry(&CommittedFile {
                    path: file.clone(),
                    status: "M".into(),
                    content: None,
                    is_kra: true,
                })
            });
        Ok(vec![entry])
    })
    .await
}

/// The per-layer rasters for a single working-tree `.kra` file (working copy vs its last committed
/// version) — the lazy counterpart to `working_diff`, mirroring `commit_layers` (streamed the
/// same way: one `on_layer` message per finished layer, out of order).
#[tauri::command]
pub async fn working_layers(
    path: String,
    file: String,
    on_layer: tauri::ipc::Channel<LayerDto>,
) -> std::result::Result<(), String> {
    run(move || {
        if !file.to_lowercase().ends_with(".kra") {
            return Ok(());
        }
        let repo = Repo::open(Path::new(&path))?;
        working_art_dto(
            &repo,
            &file,
            true,
            Some(&|dto| {
                let _ = on_layer.send(dto);
            }),
        )?;
        Ok(())
    })
    .await
}

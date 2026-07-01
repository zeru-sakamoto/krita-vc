//! Thin Tauri command wrappers. Heavy I/O and binary diffing run on the blocking pool
//! (`spawn_blocking`) so the webview thread stays responsive; engine errors are flattened
//! to strings for the frontend. DTOs use camelCase to match `src/types.ts`.

use crate::error::{KvcError, Result};
use crate::kra::{self, LayerDiff, LayerNode};
use crate::repo::{Commit, CommittedFile, Repo};
use crate::{commit, scan};
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
        let repo = Repo::open(Path::new(&path))?;
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

#[tauri::command]
pub async fn list_commits(path: String) -> std::result::Result<Vec<Commit>, String> {
    run(move || {
        let repo = Repo::open(Path::new(&path))?;
        Ok(repo.commits.clone())
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
        let old = commit::file_at_commit(&repo, &file, &old_commit)?;
        let new = commit::file_at_commit(&repo, &file, &new_commit)?;
        kra::diff_maindoc(
            &kra::read_entry(&old, "maindoc.xml")?,
            &kra::read_entry(&new, "maindoc.xml")?,
        )
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerDto {
    id: String,
    name: String,
    opacity: i64,
    blend_mode: String,
    change: String,
    /// Inner SVG `<image>` markup for each state, or null when the layer is absent then.
    before: Option<String>,
    after: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtDiffDto {
    path: String,
    status: String,
    width: u32,
    height: u32,
    layers: Vec<LayerDto>,
    regions: Vec<RegionDto>,
    /// Composite (mergedimage.png) for each state as `<image>` markup — the reliable composite.
    #[serde(skip_serializing_if = "Option::is_none")]
    before_image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    after_image: Option<String>,
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

/// Build the visual diff for one `.kra` file: per-layer rasters (before/after) + composite.
fn art_diff(repo: &Repo, f: &CommittedFile, old: Option<&CommittedFile>) -> Result<DiffEntryDto> {
    let path = &f.path;
    let new_manifest = f
        .content
        .as_deref()
        .ok_or_else(|| KvcError::NotTracked(format!("{path} (no content)")))?;
    let new_meta = {
        let xml = kra::entry_bytes(repo, path, new_manifest, "maindoc.xml")?
            .ok_or_else(|| KvcError::CorruptZip("no maindoc.xml".into()))?;
        kra::parse_image_meta(&xml)?
    };
    let (w, h) = (new_meta.width, new_meta.height);

    let old_manifest = old.and_then(|o| o.content.as_deref());
    let old_meta = match old_manifest {
        Some(hh) => match kra::entry_bytes(repo, path, hh, "maindoc.xml")? {
            Some(xml) => Some(kra::parse_image_meta(&xml)?),
            None => None,
        },
        None => None,
    };
    let changed_entries = match old_manifest {
        Some(hh) => kra::changed_entry_paths(repo, path, hh, new_manifest)?,
        None => Default::default(),
    };

    let img = |url: String| {
        format!("<image href=\"{url}\" x=\"0\" y=\"0\" width=\"{w}\" height=\"{h}\" preserveAspectRatio=\"none\"/>")
    };

    let mut layers: Vec<LayerDto> = Vec::new();
    for nl in &new_meta.layers {
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
        let after =
            kra::layer_raster(repo, path, new_manifest, &new_meta.name, &nl.filename, w, h)?
                .map(img);
        let before = match (change, old_manifest, &old_meta, ol) {
            ("added", ..) => None,
            (_, Some(oh), Some(om), Some(o)) => {
                kra::layer_raster(repo, path, oh, &om.name, &o.filename, w, h)?.map(img)
            }
            _ => None,
        };
        layers.push(LayerDto {
            id: layer_id(nl),
            name: nl.name.clone(),
            opacity: to_percent(nl.opacity),
            blend_mode: blend_mode(&nl.blend),
            change: change.into(),
            before,
            after,
        });
    }
    // Layers removed since the parent commit.
    if let (Some(om), Some(oh)) = (&old_meta, old_manifest) {
        for ol in &om.layers {
            if !new_meta
                .layers
                .iter()
                .any(|nl| layer_id(nl) == layer_id(ol))
            {
                let before =
                    kra::layer_raster(repo, path, oh, &om.name, &ol.filename, w, h)?.map(img);
                layers.push(LayerDto {
                    id: layer_id(ol),
                    name: ol.name.clone(),
                    opacity: to_percent(ol.opacity),
                    blend_mode: blend_mode(&ol.blend),
                    change: "removed".into(),
                    before,
                    after: None,
                });
            }
        }
    }
    layers.reverse(); // Krita writes top-first; the UI stacks bottom→top.

    let after_image = kra::entry_data_url(repo, path, new_manifest, "mergedimage.png")?.map(img);
    let before_image = match old_manifest {
        Some(oh) => kra::entry_data_url(repo, path, oh, "mergedimage.png")?.map(img),
        None => None,
    };
    let mut regions = Vec::new();
    if let Some(oh) = old_manifest {
        if let Some((x, y, rw, rh)) = kra::changed_region(repo, path, oh, new_manifest, w, h)? {
            regions.push(RegionDto {
                x,
                y,
                w: rw,
                h: rh,
                label: None,
            });
        }
    }

    Ok(DiffEntryDto::Art(ArtDiffDto {
        path: path.clone(),
        status: f.status.clone(),
        width: w.max(0) as u32,
        height: h.max(0) as u32,
        layers,
        regions,
        before_image,
        after_image,
    }))
}

/// Minimal text placeholder for a file (non-.kra, deleted, or an .kra we couldn't raster).
fn text_entry(f: &CommittedFile) -> DiffEntryDto {
    DiffEntryDto::Text(TextDiffDto {
        path: f.path.clone(),
        status: f.status.clone(),
        lines: Vec::new(),
    })
}

/// The visual diff for one file: an art diff for a rasterable `.kra`, else a text placeholder.
/// A failed `.kra` raster degrades to a text entry rather than aborting the whole diff, so one
/// unsupported/broken file can't blank the entire panel.
fn diff_entry(repo: &Repo, f: &CommittedFile, old: Option<&CommittedFile>) -> DiffEntryDto {
    if f.is_kra && f.status != "D" {
        art_diff(repo, f, old).unwrap_or_else(|_| text_entry(f))
    } else {
        text_entry(f)
    }
}

/// The full visual diff for a commit: one entry per changed file. `.kra` files render as art
/// diffs (per-layer rasters + composite); everything else is a minimal text entry.
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
            .map(|f| diff_entry(&repo, f, parent_tree.get(&f.path)))
            .collect())
    })
    .await
}

/// The visual diff for a single working-tree file vs its last committed version. Stages the
/// working `.kra` into the object store (`commit_kra` — objects only, no commit) to get a
/// manifest hash, then reuses the same art-diff rasterizer. `None` old side (untracked file or
/// empty history) yields an all-"added" diff.
#[tauri::command]
pub async fn working_diff(
    path: String,
    file: String,
) -> std::result::Result<Vec<DiffEntryDto>, String> {
    run(move || {
        let mut repo = Repo::open(Path::new(&path))?;
        let abs = repo.root.join(&file);
        let bytes = std::fs::read(&abs).map_err(|e| crate::error::io_at(&abs, e))?;
        let is_kra = file.to_lowercase().ends_with(".kra");

        // Last-committed version of this file (if any), for the "before" side.
        let old = repo
            .commits
            .last()
            .map(|c| c.id.clone())
            .and_then(|head| commit::tree_at_commit(&repo.commits, &head))
            .and_then(|tree| tree.get(&file).cloned());
        let status = if old.is_some() { "M" } else { "A" };

        let content = if is_kra {
            Some(kra::commit_kra(&mut repo, &file, &bytes)?)
        } else {
            None
        };
        let working = CommittedFile {
            path: file.clone(),
            status: status.into(),
            content,
            is_kra,
        };
        Ok(vec![diff_entry(&repo, &working, old.as_ref())])
    })
    .await
}

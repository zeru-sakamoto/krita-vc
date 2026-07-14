//! Thin Tauri command wrappers. Heavy I/O and binary diffing run on the blocking pool
//! (`spawn_blocking`) so the webview thread stays responsive; engine errors are flattened
//! to strings for the frontend. DTOs use camelCase to match `src/types.ts`.

use crate::error::{KvcError, Result};
use crate::kra::{self, LayerDiff, LayerNode};
use crate::repo::{Commit, CommittedFile, Config, Repo, RepoLock};
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

// --- kvcimg raster delivery ---------------------------------------------------------------
// Roots the `kvcimg` URI scheme is allowed to serve from. Only diff commands register here,
// so the scheme can never be steered at an arbitrary path — and even for registered roots it
// serves nothing but `<root>/.kvc/cache/<hex key>.png`.

static SERVED_REPOS: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> =
    std::sync::OnceLock::new();

fn register_served_repo(path: &str) {
    SERVED_REPOS
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .insert(path.to_string());
}

fn is_served_repo(path: &str) -> bool {
    SERVED_REPOS
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .contains(path)
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 || s.is_empty() {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

/// Serve one cached raster for the `kvcimg` scheme: path shape `/<hex repo root>/<key>.png`,
/// where `key` is a hex content hash. Anything malformed, unregistered, or missing is a 404.
pub fn serve_raster(uri: &tauri::http::Uri) -> tauri::http::Response<Vec<u8>> {
    let not_found = || {
        tauri::http::Response::builder()
            .status(404)
            .body(Vec::new())
            .expect("static 404 response")
    };
    let path = uri.path();
    let mut parts = path.trim_start_matches('/').splitn(2, '/');
    let (Some(root_hex), Some(file)) = (parts.next(), parts.next()) else {
        return not_found();
    };
    let Some(key) = file.strip_suffix(".png") else {
        return not_found();
    };
    if key.is_empty() || key.len() > 64 || !key.bytes().all(|b| b.is_ascii_hexdigit()) {
        return not_found();
    }
    let Some(root) = hex_decode(root_hex).and_then(|b| String::from_utf8(b).ok()) else {
        return not_found();
    };
    if !is_served_repo(&root) {
        return not_found();
    }
    let cache_dir = crate::repo::cache_dir(Path::new(&root));
    // cache_read also refreshes the entry's mtime, protecting served images from pruning.
    let Some(png) = crate::raster::cache_read(&cache_dir, key) else {
        return not_found();
    };
    tauri::http::Response::builder()
        .status(200)
        .header("Content-Type", "image/png")
        // Keys are content-addressed and immutable — let the browser cache do the rest.
        .header("Cache-Control", "public, max-age=31536000, immutable")
        .body(png)
        .unwrap_or_else(|_| not_found())
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
    run(move || {
        let root = Path::new(&path);
        let _lock = RepoLock::acquire(root)?;
        Repo::delete(root)
    })
    .await
}

/// Reclaim storage unreachable from any branch tip (orphans from undo, deleted branches).
/// `dry_run` computes the report without touching anything — the UI shows it before asking
/// the user to confirm the real pass.
#[tauri::command]
pub async fn cleanup_repository(
    path: String,
    dry_run: bool,
) -> std::result::Result<crate::gc::GcReport, String> {
    run(move || {
        let root = Path::new(&path);
        // The real sweep deletes objects; a dry run only reads, so it needn't block writers.
        let _lock = if dry_run {
            None
        } else {
            Some(RepoLock::acquire(root)?)
        };
        let mut repo = Repo::open(root)?;
        crate::gc::collect_garbage(&mut repo, dry_run)
    })
    .await
}

/// Settings knobs a user can see/edit for this repo (cache budget, tile pixel deltas).
#[tauri::command]
pub async fn get_repo_config(path: String) -> std::result::Result<Config, String> {
    run(move || Ok(Repo::open_light(Path::new(&path))?.config)).await
}

#[tauri::command]
pub async fn set_repo_config(
    path: String,
    cache_max_bytes: u64,
    tile_pixel_deltas: bool,
    low_memory_diff: bool,
) -> std::result::Result<(), String> {
    run(move || {
        let root = Path::new(&path);
        let _lock = RepoLock::acquire(root)?;
        let mut repo = Repo::open_light(root)?;
        repo.config.cache_max_bytes = cache_max_bytes;
        repo.config.tile_pixel_deltas = tile_pixel_deltas;
        repo.config.low_memory_diff = low_memory_diff;
        repo.save_config()
    })
    .await
}

/// One row of the storage report: what a full copy of version N would have cost, versus what it
/// actually added to the delta store.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionRow {
    /// 1-based version number (commits are stored oldest-first).
    pub version: u32,
    pub commit_id: String,
    pub message: String,
    pub file_count: u32,
    /// Sum of the original (uncompressed) sizes of every file tracked at this version.
    pub original_bytes: u64,
    /// On-disk bytes this version *added* to the store — the objects it was the first to
    /// introduce (first-reference attribution). 0 for versions whose objects were all already
    /// stored by an earlier version, and for pre-`original_size` history it may under-count.
    pub stored_bytes: u64,
}

/// Storage the delta store saves versus naively keeping a full copy of every file per version.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageStats {
    /// Σ of every version's full-copy cost — the hypothetical "one full copy per version" total.
    pub naive_bytes: u64,
    /// Actual bytes the `.kvc` object + chain store occupies on disk.
    pub actual_bytes: u64,
    /// `naive - actual`, clamped at 0.
    pub saved_bytes: u64,
    pub per_version: Vec<VersionRow>,
}

/// Recursively sum the byte length of every file under `dir` (missing dir -> 0).
fn dir_bytes(dir: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(m) = e.metadata() {
                total += m.len();
            }
        }
    }
    total
}

/// Map every stored object name (`<hash>.full` / `<hash>.<base>.patch`) to its on-disk byte size,
/// across both loose objects and pack entries — the size lookup for per-commit attribution.
/// Mirrors GC's object/pack walk (`gc.rs`).
fn object_size_map(repo: &Repo) -> std::collections::HashMap<String, u64> {
    let mut map = std::collections::HashMap::new();
    let objects = crate::repo::objects_dir(&repo.root);
    let pack_dir = crate::delta::pack_dir(&objects);

    // Loose objects (sharded `<xx>/<name>` + legacy flat), skipping the pack subdir.
    let mut stack = vec![objects.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if p != pack_dir {
                    stack.push(p);
                }
            } else if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                if let Ok(m) = e.metadata() {
                    map.insert(name.to_string(), m.len());
                }
            }
        }
    }
    // Packed objects — payload length per entry (pack header/index overhead is left unattributed).
    if let Ok(rd) = std::fs::read_dir(&pack_dir) {
        for e in rd.flatten() {
            let path = e.path();
            if path.extension().is_none_or(|x| x != "pack") {
                continue;
            }
            if let Some(entries) = crate::delta::read_pack_header(&path) {
                for (name, _off, len) in entries {
                    map.insert(name, len as u64);
                }
            }
        }
    }
    map
}

/// The object name a `(stream key, content hash)` pair resolves to, via its chain.
fn object_of(repo: &Repo, key: &str, hash: &str) -> Option<String> {
    repo.chains
        .chain(key)?
        .iter()
        .find(|v| v.hash == hash)
        .map(|v| v.object_name())
}

/// Attribute each stored object's bytes to the **first** commit (oldest-first) that references it,
/// so a version's `stored_bytes` is exactly what it newly added to the store (objects are
/// content-addressed and shared across versions). Best-effort: a manifest that fails to reconstruct
/// still attributes its own object, just not its tile sub-streams.
fn stored_bytes_by_commit(
    repo: &Repo,
    size_of: &std::collections::HashMap<String, u64>,
) -> std::collections::HashMap<String, u64> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut memo: std::collections::HashMap<String, Vec<u8>> = std::collections::HashMap::new();
    let mut out = std::collections::HashMap::new();
    // repo.commits is append order = oldest-first.
    for c in &repo.commits {
        let mut refs: Vec<String> = Vec::new();
        for f in &c.files {
            let Some(content) = &f.content else { continue };
            if f.is_kra {
                if let Some(o) = object_of(repo, &kra::manifest_stream_key(&f.path), content) {
                    refs.push(o);
                }
                if let Ok(manifest) = kra::load_manifest_memo(repo, &f.path, content, &mut memo) {
                    for (k, h) in kra::referenced_streams(&f.path, &manifest) {
                        if let Some(o) = object_of(repo, &k, &h) {
                            refs.push(o);
                        }
                    }
                }
            } else if let Some(o) = object_of(repo, &format!("file:{}", f.path), content) {
                refs.push(o);
            }
        }
        let mut stored = 0u64;
        for o in refs {
            if seen.insert(o.clone()) {
                stored += size_of.get(&o).copied().unwrap_or(0);
            }
        }
        out.insert(c.id.clone(), stored);
    }
    out
}

/// Pure storage-report computation (no async, testable): per-version original sizes and per-version
/// delta-store cost vs the store's real on-disk footprint. `original_size` and per-version
/// attribution are effectively forward-only — history from before those existed under-counts.
pub fn compute_storage_stats(repo: &Repo) -> StorageStats {
    let size_of = object_size_map(repo);
    let stored_by_commit = stored_bytes_by_commit(repo, &size_of);
    let per_version: Vec<VersionRow> = repo
        .commits
        .iter()
        .enumerate()
        .map(|(i, c)| {
            // ponytail: O(commits × files) tree re-fold per version; fine for hand-scale histories.
            let tree = commit::tree_at_commit(&repo.commits, &c.id).unwrap_or_default();
            let original_bytes = tree.values().map(|f| f.original_size).sum();
            VersionRow {
                version: (i + 1) as u32,
                commit_id: c.id.clone(),
                message: c.message.clone(),
                file_count: tree.len() as u32,
                original_bytes,
                stored_bytes: stored_by_commit.get(&c.id).copied().unwrap_or(0),
            }
        })
        .collect();
    let naive_bytes = per_version.iter().map(|r| r.original_bytes).sum();
    let actual_bytes = dir_bytes(&crate::repo::objects_dir(&repo.root))
        + dir_bytes(&crate::repo::chains_dir(&repo.root));
    StorageStats {
        naive_bytes,
        actual_bytes,
        saved_bytes: naive_bytes.saturating_sub(actual_bytes),
        per_version,
    }
}

/// Per-version original-size breakdown + the delta store's real footprint, for the
/// Performance report.
#[tauri::command]
pub async fn repo_storage_stats(path: String) -> std::result::Result<StorageStats, String> {
    run(move || Ok(compute_storage_stats(&Repo::open(Path::new(&path))?))).await
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
    paths: Option<Vec<String>>,
) -> std::result::Result<Commit, String> {
    run(move || {
        let root = Path::new(&path);
        let _lock = RepoLock::acquire(root)?;
        let mut repo = Repo::open(root)?;
        commit::commit_selected(&mut repo, &message, &author, paths.as_deref())
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
        let root = Path::new(&path);
        let _lock = RepoLock::acquire(root)?;
        let mut repo = if base.is_some() {
            Repo::open(root)?
        } else {
            Repo::open_light(root)?
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
        let root = Path::new(&path);
        let _lock = RepoLock::acquire(root)?;
        let mut repo = Repo::open(root)?;
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
        let root = Path::new(&path);
        let _lock = RepoLock::acquire(root)?;
        let mut repo = Repo::open(root)?;
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
        let root = Path::new(&path);
        let _lock = RepoLock::acquire(root)?;
        let mut repo = Repo::open_light(root)?;
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
        let root = Path::new(&path);
        let _lock = RepoLock::acquire(root)?;
        let mut repo = Repo::open(root)?;
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
        let root = Path::new(&path);
        let _lock = RepoLock::acquire(root)?;
        let mut repo = Repo::open(root)?;
        commit::undo_last_commit(&mut repo)
    })
    .await
}

/// Discard uncommitted working-tree changes, restoring them to the current branch tip's
/// committed content — no new commit. `paths` empty discards every dirty file; otherwise only
/// those relative paths are touched (the frontend passes just the unstaged ones, since staging
/// is a UI-only concept the backend doesn't track).
#[tauri::command]
pub async fn discard_changes(path: String, paths: Vec<String>) -> std::result::Result<(), String> {
    run(move || {
        let root = Path::new(&path);
        let _lock = RepoLock::acquire(root)?;
        let mut repo = Repo::open(root)?;
        let tip = repo
            .branches
            .tip()
            .ok_or_else(|| crate::error::KvcError::NoCommit(String::new()))?
            .to_string();
        let filter = if paths.is_empty() {
            None
        } else {
            Some(paths.as_slice())
        };
        commit::discard_working_changes(&mut repo, &tip, filter)
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
        let root = Path::new(&path);
        let _lock = RepoLock::acquire(root)?;
        let repo = Repo::open(root)?;
        let bytes = commit::file_at_commit(&repo, &file, &commit_id)?;
        let target: PathBuf = crate::repo::safe_join(&repo.root, &file)?;
        std::fs::write(&target, bytes).map_err(|e| crate::error::io_at(&target, e))?;
        Ok(())
    })
    .await
}

// --- per-commit visual diff ------------------------------------------------------------
// DTOs mirror the frontend `DiffEntry` union in src/types.ts (serde camelCase). Art (.kra)
// files carry real per-layer PNG rasters + a composite; other files get a minimal text entry.

#[derive(Serialize, Clone)]
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
pub struct BoundsDto {
    pub x: i64,
    pub y: i64,
    pub w: i64,
    pub h: i64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct LayerDto {
    pub id: String,
    pub name: String,
    pub opacity: i64,
    pub blend_mode: String,
    pub change: String,
    /// `<layer visible>` — false for a Krita-hidden layer.
    pub visible: bool,
    /// Krita nodetype (e.g. "paintlayer", "grouplayer"); the UI maps it to a friendly label.
    pub layer_type: String,
    /// The layer's painted-area bounding box in image pixels (tile-granular), if it has tiles.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bounds: Option<BoundsDto>,
    /// Inner SVG `<image>` markup for each state, or null when the layer is absent then.
    pub before: Option<String>,
    pub after: Option<String>,
    /// This layer's own changed-pixel highlight, diffed from its before/after rasters — the
    /// composite's overlay must not be reused per layer. Only populated for `change == "modified"`
    /// layers (added/removed have no before/after pair); empty otherwise. Mirrors the composite
    /// fields on `ArtDiffDto` but scoped to this layer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff_image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff_outline: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub regions: Vec<RegionDto>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtDiffDto {
    pub path: String,
    pub status: String,
    pub width: u32,
    pub height: u32,
    /// Canvas resolution (DPI); omitted when unknown (0).
    #[serde(skip_serializing_if = "is_zero")]
    pub dpi: f64,
    /// Color space name (e.g. "RGBA") and ICC profile from maindoc.xml; empty when absent.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub color_model: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub color_profile: String,
    pub layers: Vec<LayerDto>,
    pub regions: Vec<RegionDto>,
    /// Composite (mergedimage.png) for each state as `<image>` markup — the reliable composite.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before_image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_image: Option<String>,
    /// Changed-pixel mask as `<image>` markup: transparent except where the composites differ.
    /// Drives the "pixels" highlight; computed off the composite so it ships with the first diff.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff_image: Option<String>,
    /// SVG path data (normalized 0..1) outlining the changed pixels' silhouette — the frontend
    /// scales it to the viewBox and strokes it dashed. Hugs the change, not a bounding box.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff_outline: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextDiffDto {
    path: String,
    status: String,
    lines: Vec<serde_json::Value>,
}

/// One swatch in a palette diff — `#RRGGBB` on each side, `None` where the swatch is absent.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SwatchDto {
    name: String,
    before: Option<String>,
    after: Option<String>,
    change: String,
}

/// Structured color-palette diff (.gpl/.kpl/.aco/.ase) — mirrors `PaletteDiff` in `src/types.ts`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaletteDiffDto {
    path: String,
    status: String,
    columns: u32,
    swatches: Vec<SwatchDto>,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DiffEntryDto {
    Art(ArtDiffDto),
    Text(TextDiffDto),
    Palette(PaletteDiffDto),
}

/// Assemble a palette DTO from the before/after file bytes (either side `None` for add/delete).
/// `None` when neither side parses as a palette — the caller then degrades to a text entry.
fn palette_dto(
    path: &str,
    status: &str,
    old_bytes: Option<&[u8]>,
    new_bytes: Option<&[u8]>,
) -> Option<PaletteDiffDto> {
    let old = old_bytes.and_then(|b| crate::palette::parse(path, b));
    let new = new_bytes.and_then(|b| crate::palette::parse(path, b));
    palette_dto_from(path, status, old.as_ref(), new.as_ref())
}

/// [`palette_dto`] once both sides are already parsed — lets a caller parse each side with its
/// own format (an embedded palette can be `.gpl` on one side and `.kpl` on the other).
fn palette_dto_from(
    path: &str,
    status: &str,
    old: Option<&crate::palette::Palette>,
    new: Option<&crate::palette::Palette>,
) -> Option<PaletteDiffDto> {
    if old.is_none() && new.is_none() {
        return None;
    }
    let d = crate::palette::diff(old, new);
    Some(PaletteDiffDto {
        path: path.to_string(),
        status: status.to_string(),
        columns: d.columns,
        swatches: d
            .swatches
            .into_iter()
            .map(|s| SwatchDto {
                name: s.name,
                before: s.before,
                after: s.after,
                change: s.change.to_string(),
            })
            .collect(),
    })
}

/// The logical identity of an embedded palette entry — its basename with the palette extension
/// and any trailing Krita version segment (`.NNNN`) stripped. Collapses the several files Krita
/// keeps for one palette (e.g. `sun-set.gpl` + `sun-set.0006.kpl`) onto one key.
fn palette_logical_key(entry: &str) -> String {
    let base = entry.rsplit('/').next().unwrap_or(entry).to_lowercase();
    let stem = [".kpl", ".gpl", ".aco", ".ase"]
        .iter()
        .find_map(|e| base.strip_suffix(e))
        .unwrap_or(&base);
    match stem.rsplit_once('.') {
        Some((head, ver)) if !ver.is_empty() && ver.bytes().all(|b| b.is_ascii_digit()) => {
            head.to_string()
        }
        _ => stem.to_string(),
    }
}

/// One representative entry per logical palette in `src`, preferring Krita's native `.kpl` and
/// then the highest version — so format duplicates and stale exports collapse to a single diff.
fn logical_palette_reps(src: &kra::KraSource) -> std::collections::HashMap<String, String> {
    let rank = |n: &str| (n.to_lowercase().ends_with(".kpl"), n.to_string());
    let mut map: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for name in src.palette_entry_names() {
        let key = palette_logical_key(&name);
        match map.get(&key) {
            Some(cur) if rank(cur) >= rank(&name) => {}
            _ => {
                map.insert(key, name);
            }
        }
    }
    map
}

/// Diffs the document palettes embedded inside a `.kra` against the parent version. Krita keeps
/// several files per palette (native `.kpl` plus `.gpl` exports, versioned `.NNNN` copies), so
/// entries are collapsed to one representative per logical palette; unchanged palettes are
/// dropped, and each result is keyed `<kra>::<palette-file>` so the frontend can label/route it.
fn kra_palette_dtos(
    repo: &Repo,
    relpath: &str,
    new_src: &kra::KraSource,
    old_src: Option<&kra::KraSource>,
) -> Vec<PaletteDiffDto> {
    let new_reps = logical_palette_reps(new_src);
    let old_reps = old_src.map(logical_palette_reps).unwrap_or_default();

    let mut keys: Vec<&String> = new_reps.keys().collect();
    keys.extend(old_reps.keys().filter(|k| !new_reps.contains_key(*k)));

    keys.into_iter()
        .filter_map(|key| {
            let new_name = new_reps.get(key);
            let old_name = old_reps.get(key);
            let new_hash = new_name.and_then(|n| new_src.entry_hash(n));
            let old_hash = old_name.and_then(|n| old_src.and_then(|o| o.entry_hash(n)));
            if new_hash.is_some() && new_hash == old_hash {
                return None; // unchanged embedded palette
            }
            // Parse each side with its own entry name so the format dispatch is correct even when
            // the representative flips format across versions.
            let new = new_name.and_then(|n| {
                new_src
                    .entry_bytes(repo, relpath, n)
                    .ok()
                    .flatten()
                    .and_then(|b| crate::palette::parse(n, &b))
            });
            let old = old_name.and_then(|n| {
                old_src.and_then(|o| {
                    o.entry_bytes(repo, relpath, n)
                        .ok()
                        .flatten()
                        .and_then(|b| crate::palette::parse(n, &b))
                })
            });
            let status = match (old.is_some(), new.is_some()) {
                (false, _) => "A",
                (true, false) => "D",
                (true, true) => "M",
            };
            let basename = new_name
                .or(old_name)
                .map(|n| n.rsplit('/').next().unwrap_or(n))
                .unwrap_or(key);
            let dto_path = format!("{relpath}::{basename}");
            let dto = palette_dto_from(&dto_path, status, old.as_ref(), new.as_ref())?;
            // A byte change with no swatch-level change (Krita re-serializing IDs/order on save)
            // isn't worth a panel — only surface palettes whose swatches actually changed.
            dto.swatches
                .iter()
                .any(|s| s.change != "unchanged")
                .then_some(dto)
        })
        .collect()
}

/// Krita opacity (0..255) → the UI's 0..100 scale.
fn to_percent(op: i64) -> i64 {
    ((op as f64) * 100.0 / 255.0).round() as i64
}

/// serde skip predicate for an omit-when-zero DPI.
fn is_zero(v: &f64) -> bool {
    *v == 0.0
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
            return Ok(Some(crate::raster::raster_url(
                &repo.root, &cache_dir, k, &png,
            )));
        }
    }
    let Some(b) = bytes()? else { return Ok(None) };
    let capped = crate::raster::cap_png(&b);
    if let Some(k) = &key {
        crate::raster::cache_write(&cache_dir, k, &capped);
        return Ok(Some(crate::raster::raster_url(
            &repo.root, &cache_dir, k, &capped,
        )));
    }
    Ok(Some(crate::raster::png_bytes_to_data_url(&capped)))
}

/// The changed-pixel highlight for a diff: the accent mask as a capped PNG URL, plus the SVG path
/// (normalized 0..1) that outlines the changed pixels' silhouette. Keyed by both composite content
/// hashes, so a warm cache reads neither composite; on a miss both raw `mergedimage.png` bytes are
/// pulled (each behind its own deferred closure, mirroring `composite_data_url`) and diffed. The
/// outline is rebuilt from the cached mask on a hit (no source re-read, no sibling cache file).
/// `(None, None)` when either side is missing (added/removed file) or can't be decoded.
fn diff_overlay_parts(
    repo: &Repo,
    before_hash: Option<&str>,
    after_hash: Option<&str>,
    before_bytes: impl FnOnce() -> Result<Option<Vec<u8>>>,
    after_bytes: impl FnOnce() -> Result<Option<Vec<u8>>>,
) -> Result<(Option<String>, Option<String>)> {
    let (Some(bh), Some(ah)) = (before_hash, after_hash) else {
        return Ok((None, None));
    };
    let cache_dir = repo.cache_dir();
    let key = kra::diff_cache_key(bh, ah);
    if let Some(png) = crate::raster::cache_read(&cache_dir, &key) {
        let url = crate::raster::raster_url(&repo.root, &cache_dir, &key, &png);
        return Ok((Some(url), crate::raster::outline_from_mask_png(&png)));
    }
    let (Some(before), Some(after)) = (before_bytes()?, after_bytes()?) else {
        return Ok((None, None));
    };
    let Some((mask, outline)) = crate::raster::diff_overlay(&before, &after) else {
        return Ok((None, None));
    };
    crate::raster::cache_write(&cache_dir, &key, &mask);
    let url = crate::raster::raster_url(&repo.root, &cache_dir, &key, &mask);
    Ok((Some(url), outline))
}

/// A single layer's changed-pixel highlight: diff its before/after capped rasters into a mask PNG
/// URL, an outline path, and a region box. Mirrors [`diff_overlay_parts`] but scoped to one layer
/// and keyed by both layer raster cache keys (so it's content-addressed and shared across
/// working/committed views of identical pixels). The pixels are already in hand from building the
/// layer URLs, so no re-decode of source tiles. The region box is **normalized 0..1** — the same
/// convention as the composite's `changed_region` (the frontend's `boxOverlay` scales it to the
/// viewBox), so it must NOT be pre-scaled to pixels or it overflows the canvas. Meant only for
/// modified layers where both sides exist.
fn layer_diff_overlay(
    repo: &Repo,
    before: &kra::LayerRaster,
    after: &kra::LayerRaster,
) -> (Option<String>, Option<String>, Vec<RegionDto>) {
    let region = |bbox: Option<(f64, f64, f64, f64)>| -> Vec<RegionDto> {
        match bbox {
            Some((x, y, w, h)) => vec![RegionDto {
                x,
                y,
                w,
                h,
                label: None,
            }],
            None => Vec::new(),
        }
    };
    let cache_dir = repo.cache_dir();
    let key = kra::diff_cache_key(&before.key, &after.key);
    if let Some(mask) = crate::raster::cache_read(&cache_dir, &key) {
        let url = crate::raster::raster_url(&repo.root, &cache_dir, &key, &mask);
        let outline = crate::raster::outline_from_mask_png(&mask);
        let regions = region(crate::raster::bbox_from_mask_png(&mask));
        return (Some(url), outline, regions);
    }
    let Some((mask, outline, bbox)) = crate::raster::diff_overlay_full(&before.png, &after.png)
    else {
        return (None, None, Vec::new());
    };
    crate::raster::cache_write(&cache_dir, &key, &mask);
    let url = crate::raster::raster_url(&repo.root, &cache_dir, &key, &mask);
    (Some(url), outline, region(bbox))
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
    // Borrow the new side's tile grid once — feeds both the change detection below and each
    // layer's painted-area bounds (kra::layer_bounds) in the loop.
    let new_index = new_src.tile_index_ref();
    // One pass over borrowed tile indexes yields both the changed-layer set and the union
    // region — no per-tile hash clones, no duplicate map builds.
    let (changed_entries, changed_region) = old_manifest
        .as_ref()
        .map(|m| {
            let d = kra::diff_tile_indexes(&m.tile_index_ref(), &new_index, w, h);
            (d.changed_paths, d.region)
        })
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
            // Keep the raster structs (URL + capped PNG + cache key) around: a modified layer's
            // before/after pixels feed its own change highlight below without re-decoding.
            let after_r = if with_rasters {
                new_src.layer_raster(repo, path, &new_meta.name, &nl.filename, w, h, &tile_cache)?
            } else {
                None
            };
            // An unchanged layer's pixels are identical on both sides — no separate before raster
            // (its `before` markup reuses `after`, and no diff is needed).
            let before_r = match (with_rasters, change, &old_manifest, &old_meta, ol) {
                (true, "modified", Some(om_manifest), Some(om), Some(o)) => kra::layer_raster(
                    repo,
                    path,
                    om_manifest,
                    &om.name,
                    &o.filename,
                    w,
                    h,
                    &tile_cache,
                )?,
                _ => None,
            };
            let after = after_r.as_ref().map(|r| img(r.url.clone()));
            let before = match change {
                "added" => None,
                "unchanged" => after.clone(),
                _ => before_r.as_ref().map(|r| img(r.url.clone())),
            };
            // Per-layer change highlight — only modified layers with both sides present. Added/
            // removed layers get none (no pair to diff; the row label conveys the change).
            let (diff_image, diff_outline, regions) = match (&before_r, &after_r) {
                (Some(b), Some(a)) if change == "modified" => {
                    let (mask_url, outline, regions) = layer_diff_overlay(repo, b, a);
                    (mask_url.map(&img), outline, regions)
                }
                _ => (None, None, Vec::new()),
            };
            let dto = LayerDto {
                id: layer_id(nl),
                name: nl.name.clone(),
                opacity: to_percent(nl.opacity),
                blend_mode: blend_mode(&nl.blend),
                change: change.into(),
                visible: nl.visible,
                layer_type: nl.kind.clone(),
                bounds: kra::layer_bounds(&new_index, &entry_path, w, h)
                    .map(|(x, y, bw, bh)| BoundsDto { x, y, w: bw, h: bh }),
                before,
                after,
                diff_image,
                diff_outline,
                regions,
            };
            if let Some(cb) = on_layer {
                cb(dto.clone());
            }
            Ok(dto)
        })
        .collect::<Result<Vec<_>>>()?;
    // Layers removed since the parent commit.
    if let (Some(om), Some(om_manifest)) = (&old_meta, &old_manifest) {
        let old_index = om_manifest.tile_index_ref();
        for ol in &om.layers {
            if !new_meta
                .layers
                .iter()
                .any(|nl| layer_id(nl) == layer_id(ol))
            {
                let entry_path = format!("{}/layers/{}", om.name, ol.filename);
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
                    .map(|r| img(r.url))
                } else {
                    None
                };
                let dto = LayerDto {
                    id: layer_id(ol),
                    name: ol.name.clone(),
                    opacity: to_percent(ol.opacity),
                    blend_mode: blend_mode(&ol.blend),
                    change: "removed".into(),
                    visible: ol.visible,
                    layer_type: ol.kind.clone(),
                    bounds: kra::layer_bounds(&old_index, &entry_path, w, h)
                        .map(|(x, y, bw, bh)| BoundsDto { x, y, w: bw, h: bh }),
                    before,
                    after: None,
                    diff_image: None,
                    diff_outline: None,
                    regions: Vec::new(),
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
    // Changed-pixel highlight: diff the before/after composites. Rides on this first diff (no
    // dependence on the per-layer raster stream), so the highlight appears with the composite.
    let after_hash = new_src.entry_hash("mergedimage.png");
    let before_hash = old_manifest
        .as_ref()
        .and_then(|m| m.entry_hash("mergedimage.png"));
    let (mask_url, diff_outline) = diff_overlay_parts(
        repo,
        before_hash.as_deref(),
        after_hash.as_deref(),
        || match &old_manifest {
            Some(m) => kra::entry_bytes(repo, path, m, "mergedimage.png"),
            None => Ok(None),
        },
        || new_src.entry_bytes(repo, path, "mergedimage.png"),
    )?;
    let diff_image = mask_url.map(&img);
    let mut regions = Vec::new();
    if let Some((x, y, rw, rh)) = changed_region {
        regions.push(RegionDto {
            x,
            y,
            w: rw,
            h: rh,
            label: None,
        });
    }

    Ok(ArtDiffDto {
        path: path.to_string(),
        status: status.to_string(),
        width: w.max(0) as u32,
        height: h.max(0) as u32,
        dpi: new_meta.dpi,
        color_model: new_meta.color_model.clone(),
        color_profile: new_meta.color_profile.clone(),
        layers,
        regions,
        before_image,
        after_image,
        diff_image,
        diff_outline,
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
) -> Vec<DiffEntryDto> {
    if f.is_kra && f.status != "D" {
        // Load the manifest once and run both the art diff and the embedded-palette diff off it,
        // so the `.kra` can emit its Art entry plus one Palette entry per changed document palette.
        let Some(manifest) = f
            .content
            .as_deref()
            .and_then(|h| kra::load_manifest(repo, &f.path, h).ok())
        else {
            return vec![text_entry(f)];
        };
        let new_src = kra::KraSource::Committed(&manifest);
        let Ok(art) = art_diff_dto(repo, &f.path, &f.status, &new_src, old, with_rasters, None)
        else {
            return vec![text_entry(f)];
        };
        let old_manifest = old
            .and_then(|o| o.content.as_deref())
            .and_then(|h| kra::load_manifest(repo, &f.path, h).ok());
        let old_src = old_manifest.as_ref().map(kra::KraSource::Committed);
        let mut out = vec![DiffEntryDto::Art(art)];
        out.extend(
            kra_palette_dtos(repo, &f.path, &new_src, old_src.as_ref())
                .into_iter()
                .map(DiffEntryDto::Palette),
        );
        return out;
    }
    if crate::palette::is_palette(&f.path) {
        let recon =
            |h: Option<&str>| h.and_then(|h| repo.reconstruct(&format!("file:{}", f.path), h).ok());
        let new_bytes = if f.status == "D" {
            None
        } else {
            recon(f.content.as_deref())
        };
        let old_bytes = recon(old.and_then(|o| o.content.as_deref()));
        if let Some(dto) = palette_dto(
            &f.path,
            &f.status,
            old_bytes.as_deref(),
            new_bytes.as_deref(),
        ) {
            return vec![DiffEntryDto::Palette(dto)];
        }
    }
    vec![text_entry(f)]
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
        // Register only after the path is confirmed to be a real repo, so a failed open never
        // adds a root to the kvcimg scheme's allowlist.
        register_served_repo(&path);
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
            .flat_map(|f| diff_entry(&repo, f, parent_tree.get(&f.path), false))
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
        // Register only after the path is confirmed to be a real repo, so a failed open never
        // adds a root to the kvcimg scheme's allowlist.
        register_served_repo(&path);
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
        // Off the UI's critical path (all layers already streamed) and rate-limited inside.
        crate::raster::cache_prune_throttled(&repo.cache_dir(), repo.config.cache_max_bytes);
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
    let abs = crate::repo::safe_join(&repo.root, file)?;
    let bytes = std::fs::read(&abs).map_err(|e| crate::error::io_at(&abs, e))?;
    let working = kra::parse_working(&bytes, repo.config.low_memory_diff)?;
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
        // Register only after the path is confirmed to be a real repo, so a failed open never
        // adds a root to the kvcimg scheme's allowlist.
        register_served_repo(&path);
        if !file.to_lowercase().ends_with(".kra") {
            let old = last_committed(&repo, &file);
            let status = if old.is_some() { "M" } else { "A" };
            if crate::palette::is_palette(&file) {
                let abs = crate::repo::safe_join(&repo.root, &file)?;
                let new_bytes = std::fs::read(&abs).ok();
                let old_bytes = old
                    .as_ref()
                    .and_then(|o| o.content.as_deref())
                    .and_then(|h| repo.reconstruct(&format!("file:{}", file), h).ok());
                if let Some(dto) =
                    palette_dto(&file, status, old_bytes.as_deref(), new_bytes.as_deref())
                {
                    return Ok(vec![DiffEntryDto::Palette(dto)]);
                }
            }
            return Ok(vec![text_entry(&CommittedFile {
                path: file.clone(),
                status: status.into(),
                content: None,
                is_kra: false,
                file_hash: None,
                original_size: 0,
            })]);
        }
        // Parse the working `.kra` once, then run the art diff and the embedded-palette diff off
        // the same source (mirrors `diff_entry`'s committed path). `working_art_dto` stays for the
        // raster-streaming `working_layers`.
        let art_text = || {
            text_entry(&CommittedFile {
                path: file.clone(),
                status: "M".into(),
                content: None,
                is_kra: true,
                file_hash: None,
                original_size: 0,
            })
        };
        let abs = crate::repo::safe_join(&repo.root, &file)?;
        let Ok(bytes) = std::fs::read(&abs) else {
            return Ok(vec![art_text()]);
        };
        let Ok(working) = kra::parse_working(&bytes, repo.config.low_memory_diff) else {
            return Ok(vec![art_text()]);
        };
        let new_src = kra::KraSource::Working(&working);
        let old = last_committed(&repo, &file);
        let status = if old.is_some() { "M" } else { "A" };
        let Ok(art) = art_diff_dto(&repo, &file, status, &new_src, old.as_ref(), false, None)
        else {
            return Ok(vec![art_text()]);
        };
        let old_manifest = old
            .as_ref()
            .and_then(|o| o.content.as_deref())
            .and_then(|h| kra::load_manifest(&repo, &file, h).ok());
        let old_src = old_manifest.as_ref().map(kra::KraSource::Committed);
        let mut out = vec![DiffEntryDto::Art(art)];
        out.extend(
            kra_palette_dtos(&repo, &file, &new_src, old_src.as_ref())
                .into_iter()
                .map(DiffEntryDto::Palette),
        );
        Ok(out)
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
        // Register only after the path is confirmed to be a real repo, so a failed open never
        // adds a root to the kvcimg scheme's allowlist.
        register_served_repo(&path);
        working_art_dto(
            &repo,
            &file,
            true,
            Some(&|dto| {
                let _ = on_layer.send(dto);
            }),
        )?;
        // Off the UI's critical path (all layers already streamed) and rate-limited inside.
        crate::raster::cache_prune_throttled(&repo.cache_dir(), repo.config.cache_max_bytes);
        Ok(())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::palette_logical_key;

    #[test]
    fn palette_logical_key_collapses_format_and_version_duplicates() {
        // The real files Krita packs for one palette all fold to a single key.
        assert_eq!(
            palette_logical_key("test-file/palettes/sun-set.gpl"),
            "sun-set"
        );
        assert_eq!(
            palette_logical_key("test-file/palettes/sun-set.0006.kpl"),
            "sun-set"
        );
        // A different palette keeps its own identity.
        assert_eq!(
            palette_logical_key("test-file/palettes/godscrown.gpl"),
            "godscrown"
        );
        // A dotted name without a numeric version segment is preserved.
        assert_eq!(palette_logical_key("palettes/My.Palette.kpl"), "my.palette");
    }
}

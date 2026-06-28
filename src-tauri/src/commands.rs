//! Thin Tauri command wrappers. Heavy I/O and binary diffing run on the blocking pool
//! (`spawn_blocking`) so the webview thread stays responsive; engine errors are flattened
//! to strings for the frontend. DTOs use camelCase to match `src/types.ts`.

use crate::error::Result;
use crate::kra::{self, LayerDiff};
use crate::repo::{Commit, Repo};
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
        kra::diff_maindoc(&kra::read_entry(&old, "maindoc.xml")?, &kra::read_entry(&new, "maindoc.xml")?)
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

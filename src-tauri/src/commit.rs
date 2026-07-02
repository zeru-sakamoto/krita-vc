//! Commit orchestration and file restoration. `commit_snapshot` scans the working tree,
//! routes each change through the .kra engine or the generic blob store, records a commit,
//! and flushes state. `file_at_commit` rebuilds a file's exact bytes from any commit.

use crate::error::{io_at, KvcError, Result};
use crate::repo::{hash_bytes, Commit, CommittedFile, Repo, TrackedFile};
use crate::{kra, scan};
use std::collections::BTreeMap;

/// Commit every working-tree change. Returns the new commit (or `Nothing` if clean).
pub fn commit_snapshot(repo: &mut Repo, message: &str, author: &str) -> Result<Commit> {
    let changes = scan::scan(repo)?;
    if changes.is_empty() {
        return Err(KvcError::Nothing);
    }

    let mut files = Vec::new();
    for (rel, status) in changes {
        if status == "D" {
            repo.index.files.remove(&rel);
            files.push(CommittedFile {
                path: rel,
                status: "D".into(),
                content: None,
                is_kra: false,
            });
            continue;
        }

        let abs = repo.root.join(&rel);
        let bytes = std::fs::read(&abs).map_err(|e| io_at(&abs, e))?;
        let (size, mtime) = std::fs::metadata(&abs)
            .map(|m| crate::repo::size_mtime(&m))
            .unwrap_or((0, 0));
        let is_kra = rel.to_lowercase().ends_with(".kra");

        let content = if is_kra {
            kra::commit_kra(repo, &rel, &bytes)?
        } else {
            repo.store_stream(&format!("file:{rel}"), &bytes)?
        };

        repo.index.files.insert(
            rel.clone(),
            TrackedFile {
                hash: hash_bytes(&bytes),
                is_kra,
                size,
                mtime,
            },
        );

        files.push(CommittedFile {
            path: rel,
            status: if status == "U" {
                "A".into()
            } else {
                "M".into()
            },
            content: Some(content),
            is_kra,
        });
    }

    let timestamp = crate::repo::now_iso();
    let parents: Vec<String> = repo
        .commits
        .last()
        .map(|c| vec![c.id.clone()])
        .unwrap_or_default();
    let id = commit_id(&timestamp, message, &files);

    let commit = Commit {
        id: id.clone(),
        hash: id,
        message: message.to_string(),
        author: author.to_string(),
        timestamp,
        parents,
        files,
    };
    repo.commits.push(commit.clone());
    repo.save()?;
    Ok(commit)
}

/// Reconstruct the exact bytes of `relpath` as recorded in commit `commit_id`.
pub fn file_at_commit(repo: &Repo, relpath: &str, commit_id: &str) -> Result<Vec<u8>> {
    let commit = repo
        .commits
        .iter()
        .find(|c| c.id == commit_id)
        .ok_or_else(|| KvcError::NoCommit(commit_id.to_string()))?;
    let file = commit
        .files
        .iter()
        .find(|f| f.path == relpath)
        .ok_or_else(|| KvcError::NotTracked(relpath.to_string()))?;
    let content = file
        .content
        .as_ref()
        .ok_or_else(|| KvcError::NotTracked(format!("{relpath} (deleted in this commit)")))?;

    if file.is_kra {
        kra::reconstruct_kra(repo, relpath, content)
    } else {
        repo.reconstruct(&format!("file:{relpath}"), content)
    }
}

/// Effective tree state (path -> its committed entry) after applying `commits[..=idx]`.
/// Commits store only their *changed* files, so the full tree is the fold of every commit
/// up to `idx`, last-writer-wins per path, dropping paths whose latest entry is a deletion.
fn tree_at(commits: &[Commit], idx: usize) -> BTreeMap<String, CommittedFile> {
    let mut tree = BTreeMap::new();
    for c in &commits[..=idx] {
        for f in &c.files {
            if f.status == "D" {
                tree.remove(&f.path);
            } else {
                tree.insert(f.path.clone(), f.clone());
            }
        }
    }
    tree
}

/// Effective tree state (path -> committed entry) as of `commit_id`, or `None` if unknown.
pub fn tree_at_commit(
    commits: &[Commit],
    commit_id: &str,
) -> Option<BTreeMap<String, CommittedFile>> {
    let idx = commits.iter().position(|c| c.id == commit_id)?;
    Some(tree_at(commits, idx))
}

/// Reconstruct a committed file's exact bytes from its stored entry (kra manifest or blob).
fn bytes_of(repo: &Repo, f: &CommittedFile) -> Result<Vec<u8>> {
    let content = f
        .content
        .as_deref()
        .ok_or_else(|| KvcError::NotTracked(format!("{} (no content)", f.path)))?;
    if f.is_kra {
        kra::reconstruct_kra(repo, &f.path, content)
    } else {
        repo.reconstruct(&format!("file:{}", f.path), content)
    }
}

/// Return the working tree to its state at `commit_id`, then record that as a **new** commit
/// (non-destructive; history stays linear and reversible). Reuses `commit_snapshot` to capture
/// the result. Errors `NoCommit` if the id is unknown, or `Nothing` if the tree already matches.
pub fn rollback_to_commit(repo: &mut Repo, commit_id: &str, author: &str) -> Result<Commit> {
    let idx = repo
        .commits
        .iter()
        .position(|c| c.id == commit_id)
        .ok_or_else(|| KvcError::NoCommit(commit_id.to_string()))?;
    let target = tree_at(&repo.commits, idx);

    // Materialize every file from the target state into the working tree.
    for f in target.values() {
        let bytes = bytes_of(repo, f)?;
        let abs = repo.root.join(&f.path);
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent).map_err(|e| io_at(parent, e))?;
        }
        std::fs::write(&abs, &bytes).map_err(|e| io_at(&abs, e))?;
    }
    // Remove currently-tracked files that didn't exist at the target commit.
    for path in repo.index.files.keys().cloned().collect::<Vec<_>>() {
        if !target.contains_key(&path) {
            let abs = repo.root.join(&path);
            if abs.exists() {
                std::fs::remove_file(&abs).map_err(|e| io_at(&abs, e))?;
            }
        }
    }

    let message = format!("Restored to Version {}", idx + 1);
    commit_snapshot(repo, &message, author)
}

/// Undo the most recent commit, **keeping working-tree files as-is** (soft reset): the undone
/// edits reappear as uncommitted changes on the next scan. Only the index is rewound, for the
/// paths the popped commit touched. Returns the new head commit (or `None` if the log is empty).
///
/// ponytail: objects/chain versions from the popped commit are left in place — they're
/// content-addressed, so they're harmless orphans and dedup on any future re-commit.
pub fn undo_last_commit(repo: &mut Repo) -> Result<Option<Commit>> {
    let last = match repo.commits.pop() {
        Some(c) => c,
        None => return Ok(None),
    };

    // For each path the undone commit touched, find its most-recent surviving entry.
    let mut restores: Vec<CommittedFile> = Vec::new();
    let mut removes: Vec<String> = Vec::new();
    for f in &last.files {
        let prior = repo
            .commits
            .iter()
            .rev()
            .flat_map(|c| c.files.iter())
            .find(|pf| pf.path == f.path);
        match prior {
            Some(pf) if pf.status != "D" && pf.content.is_some() => restores.push(pf.clone()),
            _ => removes.push(f.path.clone()),
        }
    }

    for path in removes {
        repo.index.files.remove(&path);
    }
    for pf in restores {
        let bytes = bytes_of(repo, &pf)?;
        // size/mtime left 0: the working-tree file is untouched by a soft undo, so its real mtime
        // is unknown here — 0 never matches, forcing the next scan to re-hash (correct, conservative).
        repo.index.files.insert(
            pf.path.clone(),
            TrackedFile {
                hash: hash_bytes(&bytes),
                is_kra: pf.is_kra,
                size: 0,
                mtime: 0,
            },
        );
    }
    repo.save()?;
    Ok(repo.commits.last().cloned())
}

fn commit_id(timestamp: &str, message: &str, files: &[CommittedFile]) -> String {
    let mut seed = format!("{timestamp}\n{message}\n");
    for f in files {
        seed.push_str(&format!(
            "{}:{}\n",
            f.path,
            f.content.as_deref().unwrap_or("-")
        ));
    }
    hash_bytes(seed.as_bytes())[..12].to_string()
}

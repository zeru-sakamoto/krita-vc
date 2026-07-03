//! Commit orchestration and file restoration. `commit_snapshot` scans the working tree,
//! routes each change through the .kra engine or the generic blob store, records a commit,
//! and flushes state. `file_at_commit` rebuilds a file's exact bytes from any commit.

use crate::error::{io_at, KvcError, Result};
use crate::repo::{hash_bytes, Commit, CommittedFile, Repo, TrackedFile};
use crate::{kra, scan};
use std::collections::{BTreeMap, HashMap, HashSet};

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
        .branches
        .tip()
        .map(|t| vec![t.to_string()])
        .unwrap_or_default();
    let id = commit_id(&timestamp, message, &parents, &files);

    let commit = Commit {
        id: id.clone(),
        hash: id.clone(),
        message: message.to_string(),
        author: author.to_string(),
        timestamp,
        parents,
        branch: repo.branches.current.clone(),
        files,
    };
    repo.commits.push(commit.clone());
    repo.branches.set_tip(&id);
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

/// Effective tree state (path -> committed entry) as of `commit_id`, or `None` if unknown.
///
/// Commits store only their *changed* files, and each commit's `files` is by invariant the
/// exact diff against its **first parent** (merge commits record the full merged-result diff
/// vs their first parent). So the tree is a fold along the first-parent chain, root -> commit;
/// second parents exist only for graph drawing and reachability.
pub fn tree_at_commit(
    commits: &[Commit],
    commit_id: &str,
) -> Option<BTreeMap<String, CommittedFile>> {
    let by_id: HashMap<&str, &Commit> = commits.iter().map(|c| (c.id.as_str(), c)).collect();
    let mut chain: Vec<&Commit> = Vec::new();
    let mut cur = *by_id.get(commit_id)?;
    loop {
        chain.push(cur);
        match cur.parents.first() {
            Some(p) => cur = by_id.get(p.as_str())?,
            None => break,
        }
    }
    let mut tree = BTreeMap::new();
    for c in chain.iter().rev() {
        for f in &c.files {
            if f.status == "D" {
                tree.remove(&f.path);
            } else {
                tree.insert(f.path.clone(), f.clone());
            }
        }
    }
    Some(tree)
}

/// Every commit id reachable from `tip` (inclusive) over the full parent set.
pub fn ancestors(commits: &[Commit], tip: &str) -> HashSet<String> {
    let by_id: HashMap<&str, &Commit> = commits.iter().map(|c| (c.id.as_str(), c)).collect();
    let mut seen = HashSet::new();
    let mut queue = vec![tip.to_string()];
    while let Some(id) = queue.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        if let Some(c) = by_id.get(id.as_str()) {
            queue.extend(c.parents.iter().cloned());
        }
    }
    seen
}

/// Tree of the current branch tip (empty for a branch with no commits).
pub fn current_tree(repo: &Repo) -> BTreeMap<String, CommittedFile> {
    repo.branches
        .tip()
        .and_then(|t| tree_at_commit(&repo.commits, t))
        .unwrap_or_default()
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

/// Make the working tree **and index** match `target`, rewriting only files whose committed
/// `content` hash differs from `current` — the fast-switch path: unchanged files are never
/// read, reconstructed, or rewritten (their index entries carry over untouched, so size/mtime
/// stay valid for the scanner).
pub fn materialize_tree(
    repo: &mut Repo,
    current: &BTreeMap<String, CommittedFile>,
    target: &BTreeMap<String, CommittedFile>,
) -> Result<()> {
    for (path, f) in target {
        if current.get(path).map(|c| &c.content) == Some(&f.content) {
            continue;
        }
        let bytes = bytes_of(repo, f)?;
        let abs = repo.root.join(path);
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent).map_err(|e| io_at(parent, e))?;
        }
        std::fs::write(&abs, &bytes).map_err(|e| io_at(&abs, e))?;
        let (size, mtime) = std::fs::metadata(&abs)
            .map(|m| crate::repo::size_mtime(&m))
            .unwrap_or((0, 0));
        repo.index.files.insert(
            path.clone(),
            TrackedFile {
                hash: hash_bytes(&bytes),
                is_kra: f.is_kra,
                size,
                mtime,
            },
        );
    }
    for path in current.keys() {
        if !target.contains_key(path) {
            let abs = repo.root.join(path);
            if abs.exists() {
                std::fs::remove_file(&abs).map_err(|e| io_at(&abs, e))?;
            }
            repo.index.files.remove(path);
        }
    }
    Ok(())
}

/// Return the working tree to its state at `commit_id`, then record that as a **new** commit
/// on the current branch (non-destructive and reversible). Reuses `commit_snapshot` to capture
/// the result. Errors `NoCommit` if the id is unknown, or `Nothing` if the tree already matches.
pub fn rollback_to_commit(repo: &mut Repo, commit_id: &str, author: &str) -> Result<Commit> {
    let target = tree_at_commit(&repo.commits, commit_id)
        .ok_or_else(|| KvcError::NoCommit(commit_id.to_string()))?;
    let current = current_tree(repo);

    // Materialize only what differs; the index is left alone so commit_snapshot's scan
    // picks the rewritten files up as changes.
    for (path, f) in &target {
        if current.get(path).map(|c| &c.content) == Some(&f.content) {
            continue;
        }
        let bytes = bytes_of(repo, f)?;
        let abs = repo.root.join(path);
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

    let message = format!("Restored to {}", version_label(repo, commit_id));
    commit_snapshot(repo, &message, author)
}

/// "Version N" where N is the target's 1-based position among commits reachable from the
/// current branch tip (matches the frontend's per-branch version numbering), falling back
/// to the short id for unreachable commits.
fn version_label(repo: &Repo, commit_id: &str) -> String {
    if let Some(tip) = repo.branches.tip() {
        let reach = ancestors(&repo.commits, tip);
        let pos = repo
            .commits
            .iter()
            .filter(|c| reach.contains(&c.id))
            .position(|c| c.id == commit_id);
        if let Some(pos) = pos {
            return format!("Version {}", pos + 1);
        }
    }
    format!("version {commit_id}")
}

/// Undo the most recent commit, **keeping working-tree files as-is** (soft reset): the undone
/// edits reappear as uncommitted changes on the next scan. Only the index is rewound, for the
/// paths the popped commit touched. Returns the new head commit (or `None` if the log is empty).
///
/// ponytail: objects/chain versions from the popped commit are left in place — they're
/// content-addressed, so they're harmless orphans and dedup on any future re-commit.
pub fn undo_last_commit(repo: &mut Repo) -> Result<Option<Commit>> {
    let tip_id = match repo.branches.tip() {
        Some(t) => t.to_string(),
        None => return Ok(None),
    };
    // The tip may sit mid-vec after a branch switch — locate and remove it by id, but only
    // when nothing else depends on it: no child commit anywhere, no other branch tip on it.
    let pos = repo
        .commits
        .iter()
        .position(|c| c.id == tip_id)
        .ok_or_else(|| KvcError::NoCommit(tip_id.clone()))?;
    if repo.commits.iter().any(|c| c.parents.contains(&tip_id)) {
        return Err(KvcError::CannotUndo(
            "later versions build on this one".into(),
        ));
    }
    if repo
        .branches
        .branches
        .iter()
        .any(|(name, tip)| *name != repo.branches.current && *tip == tip_id)
    {
        return Err(KvcError::CannotUndo(
            "another branch points at this version".into(),
        ));
    }
    let last = repo.commits.remove(pos);
    repo.branches
        .set_tip(last.parents.first().map(String::as_str).unwrap_or(""));

    // For each path the undone commit touched, rewind the index to the new tip's tree.
    let tip_tree = current_tree(repo);
    let mut restores: Vec<CommittedFile> = Vec::new();
    let mut removes: Vec<String> = Vec::new();
    for f in &last.files {
        match tip_tree.get(&f.path) {
            Some(pf) if pf.content.is_some() => restores.push(pf.clone()),
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
    let new_tip = repo
        .branches
        .tip()
        .and_then(|t| repo.commits.iter().find(|c| c.id == t))
        .cloned();
    Ok(new_tip)
}

pub(crate) fn commit_id(
    timestamp: &str,
    message: &str,
    parents: &[String],
    files: &[CommittedFile],
) -> String {
    let mut seed = format!("{timestamp}\n{message}\n{}\n", parents.join(","));
    for f in files {
        seed.push_str(&format!(
            "{}:{}\n",
            f.path,
            f.content.as_deref().unwrap_or("-")
        ));
    }
    hash_bytes(seed.as_bytes())[..12].to_string()
}

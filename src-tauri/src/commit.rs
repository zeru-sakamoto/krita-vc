//! Commit orchestration and file restoration. `commit_snapshot` scans the working tree,
//! routes each change through the .kra engine or the generic blob store, records a commit,
//! and flushes state. `file_at_commit` rebuilds a file's exact bytes from any commit.

use crate::error::{io_at, KvcError, Result};
use crate::repo::{hash_bytes, safe_join, Commit, CommittedFile, Repo, TrackedFile};
use crate::{kra, scan};
use std::collections::{BTreeMap, HashMap, HashSet};

/// Commit every working-tree change. Returns the new commit (or `Nothing` if clean).
pub fn commit_snapshot(repo: &mut Repo, message: &str, author: &str) -> Result<Commit> {
    commit_selected(repo, message, author, None)
}

/// Commit working-tree changes, optionally restricted to `only` (e.g. the frontend's "staged"
/// paths) — files outside that set are left dirty, uncommitted. `None` commits everything,
/// matching [`commit_snapshot`]. Errors `Nothing` if the selected set has no changes.
pub fn commit_selected(
    repo: &mut Repo,
    message: &str,
    author: &str,
    only: Option<&[String]>,
) -> Result<Commit> {
    // `keep_bytes`: changed files hand their just-read buffers straight to the commit below
    // (budgeted — see `scan::RETAIN_BUDGET`), so a big .kra isn't read twice per commit.
    let mut changes = scan::scan_detailed(repo, true)?;
    if let Some(only) = only {
        changes.retain(|c| only.iter().any(|p| p == &c.rel));
    }
    if changes.is_empty() {
        return Err(KvcError::Nothing);
    }

    // Tree at the current tip, so each .kra commit can hand its previous manifest to
    // `commit_kra` and skip re-inflating unchanged zip entries.
    let prev_tree = current_tree(repo);

    let mut files = Vec::new();
    for change in changes {
        let scan::ScanChange {
            rel,
            status,
            hash,
            size,
            mtime,
            bytes,
        } = change;
        if status == "D" {
            repo.index.files.remove(&rel);
            files.push(CommittedFile {
                path: rel,
                status: "D".into(),
                content: None,
                is_kra: false,
                file_hash: None,
            });
            continue;
        }

        // Reuse the scan's buffer when it kept one; only over-budget files pay a re-read.
        let abs = repo.root.join(&rel);
        let bytes = match bytes {
            Some(b) => b,
            None => std::fs::read(&abs).map_err(|e| io_at(&abs, e))?,
        };
        let is_kra = rel.to_lowercase().ends_with(".kra");

        let content = if is_kra {
            let prev = prev_tree
                .get(&rel)
                .filter(|f| f.is_kra)
                .and_then(|f| f.content.as_deref())
                .and_then(|h| kra::load_manifest(repo, &rel, h).ok());
            kra::commit_kra(repo, &rel, &bytes, prev.as_ref())?
        } else {
            repo.store_stream(&format!("file:{rel}"), &bytes)?
        };
        drop(bytes);

        repo.index.files.insert(
            rel.clone(),
            TrackedFile {
                hash: hash.clone(),
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
            file_hash: Some(hash),
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
        restored_from: None,
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

/// Reconstruct a committed file's exact bytes from its stored entry (kra manifest or blob),
/// plus the blake3 of those bytes for the index. A generic blob's stream hash *is* blake3 of
/// its exact bytes (write-time verified), so only a rebuilt `.kra` pays a hash pass.
fn bytes_of(repo: &Repo, f: &CommittedFile) -> Result<(Vec<u8>, String)> {
    let content = f
        .content
        .as_deref()
        .ok_or_else(|| KvcError::NotTracked(format!("{} (no content)", f.path)))?;
    if f.is_kra {
        let bytes = kra::reconstruct_kra(repo, &f.path, content)?;
        let hash = hash_bytes(&bytes);
        Ok((bytes, hash))
    } else {
        let bytes = repo.reconstruct(&format!("file:{}", f.path), content)?;
        Ok((bytes, content.to_string()))
    }
}

/// Reconstruct `target`'s bytes for one file, incrementally when possible: for a `.kra` whose
/// current committed version is on disk (materialize runs only on a clean tree), unchanged
/// entries/tiles are lifted straight from the working file instead of replayed from the object
/// store ([`kra::materialize_kra`]). Any failure falls back to the full store rebuild.
fn restore_bytes(
    repo: &Repo,
    target: &CommittedFile,
    current: Option<&CommittedFile>,
) -> Result<(Vec<u8>, String)> {
    if target.is_kra {
        if let (Some(th), Some(ch)) = (
            target.content.as_deref(),
            current
                .filter(|c| c.is_kra)
                .and_then(|c| c.content.as_deref()),
        ) {
            let working_path = safe_join(&repo.root, &target.path)?;
            if let Ok(working) = std::fs::read(&working_path) {
                if let Ok(bytes) = kra::materialize_kra(repo, &target.path, th, ch, &working) {
                    let hash = hash_bytes(&bytes);
                    return Ok((bytes, hash));
                }
            }
        }
    }
    bytes_of(repo, target)
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
        let (bytes, hash) = restore_bytes(repo, f, current.get(path))?;
        let abs = safe_join(&repo.root, path)?;
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
                hash,
                is_kra: f.is_kra,
                size,
                mtime,
            },
        );
    }
    for path in current.keys() {
        if !target.contains_key(path) {
            let abs = safe_join(&repo.root, path)?;
            if abs.exists() {
                std::fs::remove_file(&abs).map_err(|e| io_at(&abs, e))?;
            }
            repo.index.files.remove(path);
        }
    }
    Ok(())
}

/// Return the working tree to its state at `commit_id`, then record that as a **new** commit
/// on the current branch (non-destructive and reversible). Errors `NoCommit` if the id is
/// unknown, or `Nothing` if the tree already matches.
///
/// The commit is synthesized directly from the target-vs-current tree diff: every restored
/// file's content hash is already recorded in the target tree, so re-scanning and re-running
/// the whole `.kra` decompose (`commit_snapshot`) just to rediscover those hashes doubled the
/// rollback cost. The diff vs the current tip is exactly the first-parent invariant a commit's
/// `files` must satisfy, and zero new objects are needed — everything is already stored.
///
/// If `commit_id` is the current tip, there's nothing new to record — delegates to
/// [`discard_to_tip`] to discard uncommitted changes instead.
pub fn rollback_to_commit(repo: &mut Repo, commit_id: &str, author: &str) -> Result<Commit> {
    if repo.branches.tip() == Some(commit_id) {
        return discard_to_tip(repo, commit_id);
    }
    let target = tree_at_commit(&repo.commits, commit_id)
        .ok_or_else(|| KvcError::NoCommit(commit_id.to_string()))?;
    let current = current_tree(repo);

    let mut files: Vec<CommittedFile> = Vec::new();

    // Materialize and record only what differs, updating the index as we go (fresh size/mtime
    // keep the scanner's fast path valid, exactly like `materialize_tree`).
    for (path, f) in &target {
        if current.get(path).map(|c| &c.content) == Some(&f.content) {
            continue;
        }
        let (bytes, hash) = restore_bytes(repo, f, current.get(path))?;
        let abs = safe_join(&repo.root, path)?;
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
                hash: hash.clone(),
                is_kra: f.is_kra,
                size,
                mtime,
            },
        );
        files.push(CommittedFile {
            path: path.clone(),
            status: if current.contains_key(path) {
                "M".into()
            } else {
                "A".into()
            },
            content: f.content.clone(),
            is_kra: f.is_kra,
            file_hash: Some(hash),
        });
    }
    // Remove currently-tracked files that didn't exist at the target commit.
    for path in repo.index.files.keys().cloned().collect::<Vec<_>>() {
        if !target.contains_key(&path) {
            let abs = safe_join(&repo.root, &path)?;
            if abs.exists() {
                std::fs::remove_file(&abs).map_err(|e| io_at(&abs, e))?;
            }
            repo.index.files.remove(&path);
        }
    }
    // Deletions recorded against the first-parent tree only (the fold invariant).
    for (path, cf) in &current {
        if !target.contains_key(path) {
            files.push(CommittedFile {
                path: path.clone(),
                status: "D".into(),
                content: None,
                is_kra: cf.is_kra,
                file_hash: None,
            });
        }
    }

    if files.is_empty() {
        return Err(KvcError::Nothing);
    }

    let message = format!("Restored to {}", version_label(repo, commit_id));
    let timestamp = crate::repo::now_iso();
    let parents: Vec<String> = repo
        .branches
        .tip()
        .map(|t| vec![t.to_string()])
        .unwrap_or_default();
    let id = self::commit_id(&timestamp, &message, &parents, &files);
    let commit = Commit {
        id: id.clone(),
        hash: id.clone(),
        message,
        author: author.to_string(),
        timestamp,
        parents,
        branch: repo.branches.current.clone(),
        files,
        restored_from: Some(commit_id.to_string()),
    };
    repo.commits.push(commit.clone());
    repo.branches.set_tip(&id);
    repo.save()?;
    Ok(commit)
}

/// Discard uncommitted changes, restoring the working tree (and index) to `tip_id` — no new
/// commit. When `paths` is `Some`, only those relative paths are touched (e.g. so the frontend
/// can discard everything except files it's marked "staged" — staging has no backend concept of
/// its own); `None` discards every dirty file. Errors `Nothing` if nothing in scope is dirty.
///
/// Uses the real on-disk scan ([`scan::scan_detailed`]), unlike `current_tree` above which is
/// derived from committed history and would trivially equal `tip_id`'s own tree. Rewrites go
/// through [`bytes_of`] (full store rebuild), not the incremental [`restore_bytes`] path — that
/// path trusts the on-disk file as a diff base, which doesn't hold for the dirty file being
/// discarded.
pub fn discard_working_changes(
    repo: &mut Repo,
    tip_id: &str,
    paths: Option<&[String]>,
) -> Result<()> {
    let target = tree_at_commit(&repo.commits, tip_id)
        .ok_or_else(|| KvcError::NoCommit(tip_id.to_string()))?;
    let dirty = scan::scan_detailed(repo, false)?;
    let selected: Vec<&scan::ScanChange> = match paths {
        Some(only) => dirty
            .iter()
            .filter(|c| only.iter().any(|p| p == &c.rel))
            .collect(),
        None => dirty.iter().collect(),
    };
    if selected.is_empty() {
        return Err(KvcError::Nothing);
    }

    for change in selected {
        let abs = safe_join(&repo.root, &change.rel)?;
        if change.status == "U" {
            // Never committed — discarding it means it goes away.
            if abs.exists() {
                std::fs::remove_file(&abs).map_err(|e| io_at(&abs, e))?;
            }
            continue;
        }
        // "M" or "D": rewrite from the committed store.
        let f = target
            .get(&change.rel)
            .ok_or_else(|| KvcError::NotTracked(change.rel.clone()))?;
        let (bytes, hash) = bytes_of(repo, f)?;
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent).map_err(|e| io_at(parent, e))?;
        }
        std::fs::write(&abs, &bytes).map_err(|e| io_at(&abs, e))?;
        let (size, mtime) = std::fs::metadata(&abs)
            .map(|m| crate::repo::size_mtime(&m))
            .unwrap_or((0, 0));
        repo.index.files.insert(
            change.rel.clone(),
            TrackedFile {
                hash,
                is_kra: f.is_kra,
                size,
                mtime,
            },
        );
    }
    repo.save()
}

/// Discard uncommitted changes and reset the working tree to `tip_id` — no new commit.
/// Errors `Nothing` if the tree is already clean. Delegates to [`discard_working_changes`];
/// returns the (unchanged) tip commit, matching [`rollback_to_commit`]'s return shape.
fn discard_to_tip(repo: &mut Repo, tip_id: &str) -> Result<Commit> {
    discard_working_changes(repo, tip_id, None)?;
    repo.commits
        .iter()
        .rev()
        .find(|c| c.id == tip_id)
        .cloned()
        .ok_or_else(|| KvcError::NoCommit(tip_id.to_string()))
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
    repo.note_commits_truncated(); // the popped commit must leave the log on the next save
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
        // The hash the index needs is blake3 of the file as it sat on disk at the new tip:
        // recorded in `file_hash` since that field existed; a non-kra's `content` is already
        // that hash; only old .kra records pay the full reconstruct-to-hash fallback.
        let hash = if let Some(h) = &pf.file_hash {
            h.clone()
        } else if !pf.is_kra {
            pf.content.clone().expect("restores keep content")
        } else {
            bytes_of(repo, &pf)?.1
        };
        // size/mtime left 0: the working-tree file is untouched by a soft undo, so its real mtime
        // is unknown here — 0 never matches, forcing the next scan to re-hash (correct, conservative).
        repo.index.files.insert(
            pf.path.clone(),
            TrackedFile {
                hash,
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

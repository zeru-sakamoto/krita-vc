//! Local branch operations: create, switch, merge, delete. Branches are just named tips
//! over the shared commit DAG (`.kvc/branches.json`); delta chains are keyed by file path,
//! so content is deduplicated across branches automatically.

use crate::commit::{ancestors, current_tree, materialize_tree, tree_at_commit};
use crate::error::{KvcError, Result};
use crate::repo::{Commit, CommittedFile, Repo};
use crate::scan;
use std::collections::{BTreeMap, BTreeSet};

/// Create `name` at the current tip and switch to it. The tree is identical, so this is
/// O(1) — no file I/O beyond saving `.kvc/` state.
pub fn create_branch(repo: &mut Repo, name: &str) -> Result<()> {
    let name = validate_name(name)?;
    if repo.branches.branches.contains_key(&name) {
        return Err(KvcError::BranchExists(name));
    }
    let tip = repo.branches.tip().unwrap_or("").to_string();
    repo.branches.branches.insert(name.clone(), tip);
    repo.branches.current = name;
    repo.save_branches()
}

/// Switch the working tree to `name`. Refuses on a dirty tree (a clean scan also proves
/// there are no untracked files, so materialization cannot clobber unsaved work). Only
/// files whose committed content differs between the two branch trees are rewritten.
pub fn switch_branch(repo: &mut Repo, name: &str) -> Result<()> {
    if !repo.branches.branches.contains_key(name) {
        return Err(KvcError::NoBranch(name.to_string()));
    }
    if name == repo.branches.current {
        return Ok(());
    }
    ensure_clean(repo)?;

    let current = current_tree(repo);
    let target = repo
        .branches
        .tip_of(name)
        .and_then(|t| tree_at_commit(&repo.commits, t))
        .unwrap_or_default();
    materialize_tree(repo, &current, &target)?;
    repo.branches.current = name.to_string();
    repo.save()
}

/// Merge `source` into the current branch. Fast-forwards when possible; otherwise builds a
/// two-parent merge commit via a per-file three-way against the merge base. Art files can't
/// be content-merged, so when both sides changed the same file the **source** version wins
/// and the entry is flagged `"C"` for the UI to surface.
///
/// The merge commit's `files` records the merged result's diff vs the **first parent**
/// (the current branch) — the invariant `tree_at_commit` folds by.
pub fn merge_branch(repo: &mut Repo, source: &str, author: &str) -> Result<Commit> {
    if source == repo.branches.current {
        return Err(KvcError::NothingToMerge(
            "a branch cannot be merged into itself".into(),
        ));
    }
    let source_tip = repo
        .branches
        .tip_of(source)
        .ok_or_else(|| KvcError::NoBranch(source.to_string()))?
        .to_string();
    ensure_clean(repo)?;

    let current_tip = repo.branches.tip().map(str::to_string);
    let current_ancestors = current_tip
        .as_deref()
        .map(|t| ancestors(&repo.commits, t))
        .unwrap_or_default();
    if current_ancestors.contains(&source_tip) {
        return Err(KvcError::NothingToMerge(format!(
            "{source} is already part of {}",
            repo.branches.current
        )));
    }

    let cur_tree = current_tree(repo);
    let src_tree = tree_at_commit(&repo.commits, &source_tip).unwrap_or_default();

    // Fast-forward: the current branch hasn't moved since `source` split off.
    let ff = match &current_tip {
        None => true,
        Some(tip) => ancestors(&repo.commits, &source_tip).contains(tip),
    };
    if ff {
        materialize_tree(repo, &cur_tree, &src_tree)?;
        repo.branches.set_tip(&source_tip);
        repo.save()?;
        let tip = repo
            .commits
            .iter()
            .find(|c| c.id == source_tip)
            .cloned()
            .expect("source tip commit exists");
        return Ok(tip);
    }

    // Three-way: base = first common ancestor found walking source's history newest-first.
    // With criss-cross merges (multiple LCAs) the worst case is an extra "C" flag — never
    // data loss, since the source content is always kept.
    let src_ancestors = ancestors(&repo.commits, &source_tip);
    let base_tree = repo
        .commits
        .iter()
        .rev()
        .find(|c| src_ancestors.contains(&c.id) && current_ancestors.contains(&c.id))
        .and_then(|c| tree_at_commit(&repo.commits, &c.id))
        .unwrap_or_default();

    let content =
        |t: &BTreeMap<String, CommittedFile>, p: &str| t.get(p).map(|f| f.content.clone());
    let mut paths: BTreeSet<String> = BTreeSet::new();
    paths.extend(base_tree.keys().cloned());
    paths.extend(cur_tree.keys().cloned());
    paths.extend(src_tree.keys().cloned());

    let mut files: Vec<CommittedFile> = Vec::new();
    for p in &paths {
        let (base, cur, src) = (
            content(&base_tree, p),
            content(&cur_tree, p),
            content(&src_tree, p),
        );
        if src == cur || src == base {
            continue; // identical, or only the current branch changed it — first parent already has it
        }
        // From here the source side changed the file vs base.
        let conflicted = cur != base; // ...and so did the current branch: collision
        match (&src_tree.get(p.as_str()), &cur_tree.get(p.as_str())) {
            (Some(sf), cur_entry) => files.push(CommittedFile {
                path: p.clone(),
                status: if conflicted {
                    "C".into()
                } else if cur_entry.is_some() {
                    "M".into()
                } else {
                    "A".into()
                },
                content: sf.content.clone(),
                is_kra: sf.is_kra,
            }),
            (None, Some(cf)) => {
                if conflicted {
                    // Source deleted, current modified: keep the current content (a no-op
                    // entry vs the first-parent tree, so the fold invariant holds) but flag it.
                    files.push(CommittedFile {
                        path: p.clone(),
                        status: "C".into(),
                        content: cf.content.clone(),
                        is_kra: cf.is_kra,
                    });
                } else {
                    files.push(CommittedFile {
                        path: p.clone(),
                        status: "D".into(),
                        content: None,
                        is_kra: cf.is_kra,
                    });
                }
            }
            (None, None) => {} // deleted on both sides
        }
    }

    let timestamp = crate::repo::now_iso();
    let mut parents = Vec::new();
    if let Some(tip) = current_tip {
        parents.push(tip);
    }
    parents.push(source_tip);
    let message = format!("Merge {source} into {}", repo.branches.current);
    let id = crate::commit::commit_id(&timestamp, &message, &parents, &files);
    let commit = Commit {
        id: id.clone(),
        hash: id.clone(),
        message,
        author: author.to_string(),
        timestamp,
        parents,
        branch: repo.branches.current.clone(),
        files,
    };

    // Materialize the merged result: apply the commit's own diff onto the current tree.
    let mut target = cur_tree.clone();
    for f in &commit.files {
        if f.status == "D" {
            target.remove(&f.path);
        } else {
            target.insert(f.path.clone(), f.clone());
        }
    }
    materialize_tree(repo, &cur_tree, &target)?;

    repo.commits.push(commit.clone());
    repo.branches.set_tip(&id);
    repo.save()?;
    Ok(commit)
}

/// Remove the branch label. Its commits stay in history (harmless unreachable data,
/// content-addressed objects tolerate orphans; no vacuum).
pub fn delete_branch(repo: &mut Repo, name: &str) -> Result<()> {
    if name == repo.branches.current {
        return Err(KvcError::DeleteCurrent);
    }
    if repo.branches.branches.remove(name).is_none() {
        return Err(KvcError::NoBranch(name.to_string()));
    }
    repo.save_branches()
}

fn ensure_clean(repo: &Repo) -> Result<()> {
    if scan::scan(repo)?.is_empty() {
        Ok(())
    } else {
        Err(KvcError::DirtyTree)
    }
}

fn validate_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() || name.len() > 60 {
        return Err(KvcError::BadBranchName("use 1-60 characters".into()));
    }
    if name.chars().any(|c| {
        c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
    }) {
        return Err(KvcError::BadBranchName(
            "avoid slashes, quotes, and punctuation like : * ? < > |".into(),
        ));
    }
    Ok(name.to_string())
}

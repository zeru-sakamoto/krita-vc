//! Setting work aside: park the working tree's changes off to the side of history, revert the
//! files on disk, and bring them back later.
//!
//! Storage is borrowed wholesale from the commit path. A stash's content goes through the same
//! relpath-keyed streams a commit uses (`kra:{rel}:*` / `file:{rel}`) via
//! [`commit::store_change`], so a stashed `.kra` dedups its tiles against committed history and
//! costs close to nothing. The record itself lives in `.kvc/stashes.json`, not `commits.log` —
//! see [`Stash`] for why — and [`crate::gc`] roots it so the content isn't swept.
//!
//! Two orderings in here are load-bearing against a crash; both are commented at the point of
//! the write. The rule behind them: never let the working tree lose its only copy of the work.

use crate::commit;
use crate::error::{io_at, KvcError, Result};
use crate::repo::{hash_bytes, now_iso, safe_join, Repo, Stash};
use crate::scan;

/// Every stash on the shelf, oldest first — so `last()` is the latest.
pub fn list(repo: &Repo) -> &[Stash] {
    &repo.stashes.stashes
}

/// Set the working tree's changes aside: store their content, record the stash, then revert the
/// files on disk to their committed state (a set-aside `.kra` that was never committed simply
/// goes away — its content is safe in the stash).
///
/// `only` restricts the stash to those relative paths, leaving the rest of the tree dirty — the
/// same mechanism `commit_selected` uses to honour the frontend's UI-only "staged" flag.
///
/// Errors `Nothing` if nothing in scope is dirty, or `NoCommit` on a repo with no commits at all
/// (there's no committed state to revert to; the UI gates on this the way undo does).
pub fn create(
    repo: &mut Repo,
    label: &str,
    author: &str,
    only: Option<&[String]>,
) -> Result<Stash> {
    let tip = repo
        .branches
        .tip()
        .ok_or_else(|| KvcError::NoCommit(String::new()))?
        .to_string();

    // `keep_bytes`: hand the scan's buffers straight to the store, as commit does.
    let mut changes = scan::scan_detailed(repo, true)?;
    if let Some(only) = only {
        changes.retain(|c| only.iter().any(|p| p == &c.rel));
    }
    if changes.is_empty() {
        return Err(KvcError::Nothing);
    }

    let prev_tree = commit::current_tree(repo);
    let paths: Vec<String> = changes.iter().map(|c| c.rel.clone()).collect();

    let mut files = Vec::new();
    for change in changes {
        // The index entry is deliberately dropped: a stash commits nothing, so the "committed
        // head" must stay where it is. Recording the stashed hash here would make the revert
        // below scan the file as clean and skip it — see `commit::store_change`.
        let (record, _index_entry) = commit::store_change(repo, change, &prev_tree)?;
        files.push(record);
    }

    let timestamp = now_iso();
    let stash = Stash {
        id: stash_id(&timestamp, label, repo.stashes.stashes.len(), &files),
        label: label.to_string(),
        author: author.to_string(),
        timestamp,
        branch: repo.branches.current.clone(),
        files,
    };
    repo.stashes.stashes.push(stash.clone());

    // Order matters, and this is the dangerous one. `discard_working_changes` erases the work
    // from disk *before* its own save, so the stash must already be durable when it runs —
    // otherwise a crash mid-revert leaves the files reverted with no record of what was in them
    // and the user's work is gone. Saving first inverts the failure mode to a harmless one: the
    // stash exists and the files are merely still dirty.
    repo.save()?;
    commit::discard_working_changes(repo, &tip, Some(&paths))?;
    Ok(stash)
}

/// Bring a stash back: write its files onto the working tree and take it off the shelf.
///
/// All-or-nothing. If anything the stash touches has been changed since, this refuses with
/// [`KvcError::StashConflict`] naming the files and leaves the stash intact — overwriting would
/// destroy work with no way back.
pub fn pop(repo: &mut Repo, id: &str) -> Result<Stash> {
    let idx = repo
        .stashes
        .stashes
        .iter()
        .position(|s| s.id == id)
        .ok_or_else(|| KvcError::NoStash(id.to_string()))?;
    let stash = repo.stashes.stashes[idx].clone();

    let dirty = scan::scan_detailed(repo, false)?;
    let mut clash: Vec<&str> = stash
        .files
        .iter()
        .filter(|f| dirty.iter().any(|d| d.rel == f.path))
        .map(|f| f.path.as_str())
        .collect();
    if !clash.is_empty() {
        clash.sort();
        return Err(KvcError::StashConflict(clash.join(", ")));
    }

    for f in &stash.files {
        let abs = safe_join(&repo.root, &f.path)?;
        if f.status == "D" {
            // The stash recorded this file as deleted — bringing it back deletes it again.
            if abs.exists() {
                std::fs::remove_file(&abs).map_err(|e| io_at(&abs, e))?;
            }
            continue;
        }
        // Full store rebuild, not the incremental `restore_bytes` path — that one trusts the
        // on-disk file as a diff base, which isn't what's sitting there.
        let (bytes, _) = commit::bytes_of(repo, f)?;
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent).map_err(|e| io_at(parent, e))?;
        }
        std::fs::write(&abs, &bytes).map_err(|e| io_at(&abs, e))?;
    }

    // The index is left alone on purpose: it still holds the committed head, which is exactly
    // what makes the scanner report these files as changed again now they're back.
    //
    // Order matters here too, the other way round: the files land before the record goes away.
    // A crash between the two leaves the stash on the shelf with the work already restored —
    // the next pop just reports a conflict, which is recoverable. Dropping the record first
    // would risk losing the only reference to the content.
    repo.stashes.stashes.remove(idx);
    repo.save_stashes()?;
    Ok(stash)
}

/// Take a stash off the shelf without restoring it. The content stays in the object store until
/// [`crate::gc`] runs — nothing else roots it, so a cleanup reclaims it.
pub fn drop_one(repo: &mut Repo, id: &str) -> Result<()> {
    let idx = repo
        .stashes
        .stashes
        .iter()
        .position(|s| s.id == id)
        .ok_or_else(|| KvcError::NoStash(id.to_string()))?;
    repo.stashes.stashes.remove(idx);
    repo.save_stashes()
}

/// Empty the shelf. Returns how many stashes were removed.
pub fn drop_all(repo: &mut Repo) -> Result<usize> {
    let n = repo.stashes.stashes.len();
    repo.stashes.stashes.clear();
    repo.save_stashes()?;
    Ok(n)
}

/// Mirror of `commit::commit_id`. `now_iso` is second-granularity and the label may be empty, so
/// the shelf position joins the seed to keep ids distinct.
fn stash_id(
    timestamp: &str,
    label: &str,
    position: usize,
    files: &[crate::repo::CommittedFile],
) -> String {
    let mut seed = format!("{timestamp}\n{label}\n{position}\n");
    for f in files {
        seed.push_str(&format!(
            "{}:{}\n",
            f.path,
            f.content.as_deref().unwrap_or("-")
        ));
    }
    hash_bytes(seed.as_bytes())[..12].to_string()
}

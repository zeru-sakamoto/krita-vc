//! Mark-and-sweep garbage collection. Nothing in the engine ever deletes stored data on its
//! own (`undo` orphans a commit's objects, `delete_branch` strands whole histories — both by
//! design, content-addressed orphans are harmless), so a long-lived repository only grows.
//! This module reclaims it, explicitly, as a user-triggered "clean up storage" action.
//!
//! Safety model: everything reachable from any branch tip stays, patch **bases are closed
//! over** (a patch is useless without the chain back to its full snapshot), and state files
//! (chains, commits) are rewritten **before** any object is deleted — a crash mid-sweep
//! leaves only re-collectable orphans, never a reference to missing data.

use crate::commit::ancestors;
use crate::error::{io_at, Result};
use crate::kra;
use crate::repo::{Chains, Repo};
use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GcReport {
    pub dry_run: bool,
    /// Commits dropped from the log (unreachable from every branch tip).
    pub commits_removed: usize,
    /// Chain versions dropped across all streams.
    pub versions_removed: usize,
    /// Loose object files + whole pack files quarantined (rewritten packs count once).
    pub objects_deleted: usize,
    /// Bytes that left the live store this run — quarantined to `.kvc/trash/`, not yet
    /// permanently gone (see `trash_bytes_pruned`).
    pub bytes_reclaimed: u64,
    /// Raster-cache bytes freed (regenerable previews: over-budget entries pruned, plus a
    /// full wipe when the cache's filter version is stale).
    pub cache_bytes_reclaimed: u64,
    /// Bytes permanently freed by aging quarantined trash out past its retention window.
    pub trash_bytes_pruned: u64,
}

/// Retention window for quarantined GC victims: long enough to notice and recover from a bad
/// cleanup by hand, short enough that trash doesn't grow unbounded between cleanup runs.
const TRASH_MAX_AGE_DAYS: u64 = 14;

/// Rewrite a partially-dead pack only when more than a quarter of it is dead — rewriting a
/// pack rereads and rewrites every survivor, so reclaiming a few KB from a big pack costs
/// far more IO than it frees. Kept dead bytes are excluded from the report (it must state
/// what the run actually frees).
fn worth_rewriting(dead_bytes: u64, total: u64) -> bool {
    dead_bytes > 0 && dead_bytes * 4 > total
}

/// Consolidation targets: at least this many packs, each under this size, merge into one
/// (every pack header is parsed on index load — many small packs from many mid-size commits
/// accumulate parse cost and directory churn).
const CONSOLIDATE_MIN_PACKS: usize = 8;
const CONSOLIDATE_MAX_PACK_BYTES: u64 = 4 << 20;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrite_gate_over_quarter_dead() {
        assert!(!super::worth_rewriting(0, 100));
        assert!(!super::worth_rewriting(25, 100), "25% dead: keep the pack");
        assert!(super::worth_rewriting(26, 100), ">25% dead: rewrite");
    }

    #[test]
    fn quarantine_preserves_relative_path_and_moves_file() {
        let dir = tempfile::tempdir().unwrap();
        let objects = dir.path().join("objects");
        let trash = dir.path().join("trash/run1");
        std::fs::create_dir_all(objects.join("ab")).unwrap();
        let victim = objects.join("ab").join("hash.full");
        std::fs::write(&victim, b"data").unwrap();

        quarantine(&trash, &objects, &victim).unwrap();

        assert!(!victim.exists(), "the original must be gone, not copied");
        assert_eq!(
            std::fs::read(trash.join("ab").join("hash.full")).unwrap(),
            b"data"
        );
    }

    #[test]
    fn prune_trash_leaves_dirs_within_cutoff() {
        let dir = tempfile::tempdir().unwrap();
        let kvc = dir.path().join(".kvc");
        let run = kvc.join("trash").join("run1");
        std::fs::create_dir_all(&run).unwrap();
        std::fs::write(run.join("f"), b"12345").unwrap();

        // A cutoff before this run's mtime: nothing is old enough to prune yet.
        let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        assert_eq!(prune_trash(&kvc, cutoff), 0);
        assert!(run.exists(), "trash within retention survives cleanup");
    }

    #[test]
    fn prune_trash_removes_dirs_older_than_cutoff() {
        let dir = tempfile::tempdir().unwrap();
        let kvc = dir.path().join(".kvc");
        let run = kvc.join("trash").join("run1");
        std::fs::create_dir_all(&run).unwrap();
        std::fs::write(run.join("f"), b"12345").unwrap();

        // A cutoff after this run's mtime: it's expired.
        let cutoff = std::time::SystemTime::now() + std::time::Duration::from_secs(1);
        assert_eq!(prune_trash(&kvc, cutoff), 5, "reports the bytes it freed");
        assert!(!run.exists(), "expired trash run is permanently pruned");
    }
}

/// What the reachability walk found: live commits and the `(stream key, content hash)` pairs
/// they reference, with patch bases closed over.
pub(crate) struct Marks {
    /// Commit ids reachable from some branch tip.
    pub reachable: HashSet<String>,
    /// Every live stream version, patch bases included.
    pub marked: HashSet<(String, String)>,
    /// Manifests that wouldn't load — only ever non-empty in `tolerant` mode.
    pub problems: Vec<(String, String)>,
    /// Every chain, handed back rather than re-exported: `export_all` re-reads and decodes every
    /// shard on disk, and both callers need it right after the walk.
    pub all: Chains,
}

/// The mark half of mark-and-sweep, shared with [`crate::check`]. `tolerant` is the whole
/// difference between the two callers: GC must fail hard on a manifest it can't load (sweeping
/// on a partial mark would delete live data), while `check` exists precisely to *report* that
/// and keep walking.
pub(crate) fn mark_live(repo: &Repo, tolerant: bool) -> Result<Marks> {
    // --- mark: reachable commits --------------------------------------------------------
    let mut reachable: HashSet<String> = HashSet::new();
    for tip in repo.branches.branches.values().filter(|t| !t.is_empty()) {
        reachable.extend(ancestors(&repo.commits, tip));
    }

    // --- mark: live (stream key, content hash) pairs -------------------------------------
    // One reconstruct memo across all manifest loads: a long-lived `.kra` has many manifest
    // versions on the reachable chain, and each shares a patch-chain prefix with the next —
    // memoizing keeps the whole marking pass linear instead of quadratic in history length.
    let mut manifest_memo: std::collections::HashMap<String, Vec<u8>> =
        std::collections::HashMap::new();
    let mut live: HashSet<(String, String)> = HashSet::new();
    let mut problems: Vec<(String, String)> = Vec::new();
    // Stashes are roots too, not just branch tips: a stash's content is referenced by nothing in
    // `commits.log`, so without this the sweep below would delete the chains and objects behind
    // every set-aside — the work would be unrecoverable. Their files are `CommittedFile`s stored
    // through the same streams, so the same walk marks them.
    let rooted = repo
        .commits
        .iter()
        .filter(|c| reachable.contains(&c.id))
        .flat_map(|c| &c.files)
        .chain(repo.stashes.stashes.iter().flat_map(|s| &s.files));
    for f in rooted {
        let Some(content) = &f.content else { continue };
        if f.is_kra {
            live.insert((kra::manifest_stream_key(&f.path), content.clone()));
            match kra::load_manifest_memo(repo, &f.path, content, &mut manifest_memo) {
                Ok(manifest) => live.extend(kra::referenced_streams(&f.path, &manifest)),
                Err(e) if tolerant => problems.push((f.path.clone(), e.to_string())),
                Err(e) => return Err(e),
            }
        } else {
            live.insert((format!("file:{}", f.path), content.clone()));
        }
    }

    // --- close over patch bases ----------------------------------------------------------
    let all = repo.chains.export_all();
    let mut marked: HashSet<(String, String)> = HashSet::new();
    let mut queue: Vec<(String, String)> = live.into_iter().collect();
    while let Some((key, hash)) = queue.pop() {
        if !marked.insert((key.clone(), hash.clone())) {
            continue;
        }
        let base = all
            .0
            .get(&key)
            .and_then(|chain| chain.iter().find(|v| v.hash == hash))
            .and_then(|v| v.base.clone());
        if let Some(base) = base {
            queue.push((key, base));
        }
    }

    Ok(Marks {
        reachable,
        marked,
        problems,
        all,
    })
}

/// Collect everything unreachable from the current branch tips. With `dry_run` the report is
/// computed but nothing is written or deleted.
pub fn collect_garbage(repo: &mut Repo, dry_run: bool) -> Result<GcReport> {
    let Marks {
        reachable,
        marked,
        all,
        ..
    } = mark_live(repo, false)?;

    // --- sweep plan: chains + live object names ------------------------------------------
    let mut new_chains = Chains::default();
    let mut live_objects: HashSet<String> = HashSet::new();
    let mut versions_removed = 0usize;
    for (key, chain) in &all.0 {
        let kept: Vec<_> = chain
            .iter()
            .filter(|v| marked.contains(&(key.clone(), v.hash.clone())))
            .cloned()
            .collect();
        versions_removed += chain.len() - kept.len();
        for v in &kept {
            live_objects.insert(v.object_name());
        }
        if !kept.is_empty() {
            new_chains.0.insert(key.clone(), kept);
        }
    }
    let commits_removed = repo
        .commits
        .iter()
        .filter(|c| !reachable.contains(&c.id))
        .count();

    // --- sweep plan: dead loose objects and dead pack entries ----------------------------
    let objects = repo.objects_dir();
    let mut dead_loose: Vec<(PathBuf, u64)> = Vec::new();
    let mut stack = vec![objects.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if p != crate::delta::pack_dir(&objects) {
                    stack.push(p);
                }
            } else if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                if !live_objects.contains(name) {
                    let len = e.metadata().map(|m| m.len()).unwrap_or(0);
                    dead_loose.push((p, len));
                }
            }
        }
    }

    // pack path -> (its entries, dead payload bytes)
    struct PackPlan {
        path: PathBuf,
        entries: Vec<(String, u64, u32)>,
        live: usize,
        dead_bytes: u64,
        total: u64,
    }
    let mut pack_plans: Vec<PackPlan> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(crate::delta::pack_dir(&objects)) {
        for e in rd.flatten() {
            let path = e.path();
            if path.extension().is_none_or(|x| x != "pack") {
                continue;
            }
            let Some(entries) = crate::delta::read_pack_header(&path) else {
                continue;
            };
            let live = entries
                .iter()
                .filter(|(n, ..)| live_objects.contains(n))
                .count();
            let dead_bytes: u64 = entries
                .iter()
                .filter(|(n, ..)| !live_objects.contains(n))
                .map(|(_, _, len)| *len as u64)
                .sum();
            let total = e.metadata().map(|m| m.len()).unwrap_or(0);
            pack_plans.push(PackPlan {
                path,
                entries,
                live,
                dead_bytes,
                total,
            });
        }
    }

    // --- sweep plan: stale temp files (crash leftovers from atomic writes) ---------------
    let stale_tmp = stale_tmp_files(repo);

    let mut report = GcReport {
        dry_run,
        commits_removed,
        versions_removed,
        objects_deleted: dead_loose.len(),
        bytes_reclaimed: dead_loose.iter().map(|(_, len)| len).sum(),
        cache_bytes_reclaimed: 0,
        trash_bytes_pruned: 0,
    };
    for p in &pack_plans {
        if p.live == 0 {
            report.objects_deleted += 1;
            report.bytes_reclaimed += p.total;
        } else if worth_rewriting(p.dead_bytes, p.total) {
            report.bytes_reclaimed += p.dead_bytes;
        }
    }
    report.bytes_reclaimed += stale_tmp.iter().map(|(_, len)| len).sum::<u64>();

    // --- raster cache: stale filter version wipes everything, else prune to budget -------
    let cache_dir = repo.cache_dir();
    let cache_total = crate::raster::cache_total_bytes(&cache_dir);
    report.cache_bytes_reclaimed = if crate::raster::cache_filter_stale(&cache_dir) {
        cache_total
    } else {
        cache_total.saturating_sub(repo.config.cache_max_bytes)
    };

    if dry_run {
        return Ok(report);
    }

    // --- write state FIRST (crash between = harmless re-collectable orphans) -------------
    repo.commits.retain(|c| reachable.contains(&c.id));
    repo.note_commits_truncated(); // dropped commits must leave the log
    repo.chains.rewrite_all(&repo.store.clone(), new_chains)?;
    repo.save()?;

    // --- quarantine loose (moved to .kvc/trash/, not deleted outright — see gap #5) --------
    let kvc = repo.store.clone();
    let trash_dir = trash_run_dir(&kvc);
    for (path, _) in &dead_loose {
        let _ = quarantine(&trash_dir, &objects, path);
    }

    // --- quarantine / rewrite packs ---------------------------------------------------------
    for p in &pack_plans {
        if p.live == 0 {
            let _ = quarantine(&trash_dir, &objects, &p.path);
        } else if worth_rewriting(p.dead_bytes, p.total) {
            // Rewrite with survivors only; write the new pack (or loose files for a small
            // remainder) before quarantining the old one, so a crash never loses live objects.
            let survivors: Vec<(String, Vec<u8>)> = p
                .entries
                .iter()
                .filter(|(n, ..)| live_objects.contains(n))
                .map(|(n, off, len)| {
                    crate::delta::read_exact_at(&p.path, *off, *len as usize)
                        .map(|bytes| (n.clone(), bytes))
                })
                .collect::<Result<_>>()?;
            if survivors.len() >= crate::delta::PACK_MIN_OBJECTS {
                let refs: Vec<&(String, Vec<u8>)> = survivors.iter().collect();
                repo.packs.write_pack(&objects, &refs)?;
            } else {
                for (name, bytes) in &survivors {
                    crate::delta::write_loose(&objects, name, bytes)?;
                }
            }
            let _ = quarantine(&trash_dir, &objects, &p.path);
        }
    }
    repo.packs.invalidate();

    // --- consolidate small surviving packs into one (fragmentation, not reclamation) -------
    consolidate_small_packs(repo)?;

    // --- stale temp files: crash leftovers, not reachable-once data, no recovery value -----
    for (path, _) in &stale_tmp {
        let _ = std::fs::remove_file(path);
    }

    // --- raster cache ------------------------------------------------------------------------
    let mut cache_freed = crate::raster::cache_sync_filter_version(&cache_dir);
    cache_freed += crate::raster::cache_prune(&cache_dir, repo.config.cache_max_bytes);
    report.cache_bytes_reclaimed = cache_freed;

    // --- age out quarantined trash past its retention window --------------------------------
    let cutoff =
        std::time::SystemTime::now() - std::time::Duration::from_secs(TRASH_MAX_AGE_DAYS * 86_400);
    report.trash_bytes_pruned = prune_trash(&kvc, cutoff);

    let tip = repo.branches.tip().map(str::to_string);
    let _ = crate::ops_log::append(
        repo,
        "cleanup",
        tip.as_deref(),
        tip.as_deref(),
        Some(format!(
            "reclaimed {} bytes ({} objects, {} versions, {} commits)",
            report.bytes_reclaimed,
            report.objects_deleted,
            report.versions_removed,
            commits_removed
        )),
    );

    Ok(report)
}

/// Where this cleanup run's quarantined objects/packs land — `.kvc/trash/<now>/`. Only
/// referenced lazily by [`quarantine`], which creates it (and its subdirectories) on first use,
/// so a cleanup with nothing to reclaim never creates an empty trash run.
fn trash_run_dir(kvc: &Path) -> PathBuf {
    kvc.join("trash").join(crate::repo::now_iso_filesafe())
}

/// Move `victim` (an absolute path under `objects_root`) into `trash_dir`, preserving its path
/// relative to `objects_root` (`objects/ab/<hash>.full` -> `trash/<ts>/ab/<hash>.full`) so two
/// quarantined objects never collide on name. `rename` is the same cost class as the
/// `remove_file` it replaces — same-volume, metadata-only.
fn quarantine(trash_dir: &Path, objects_root: &Path, victim: &Path) -> Result<()> {
    let rel = victim.strip_prefix(objects_root).unwrap_or(victim);
    let dest = trash_dir.join(rel);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| io_at(parent, e))?;
    }
    std::fs::rename(victim, &dest).map_err(|e| io_at(victim, e))?;
    Ok(())
}

/// Sum of all file sizes under `dir`, recursively — used to report bytes freed by pruning.
fn dir_size(dir: &Path) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                total += e.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
    }
    total
}

/// Permanently remove trash run-dirs older than `cutoff`. Only called on a real (non-dry-run)
/// cleanup, after the sweep — a dry run must never touch trash. Takes an explicit cutoff instant
/// (rather than computing "now minus the retention window" internally) so the aging logic is
/// testable without faking a directory's mtime — the caller just moves the cutoff instead.
fn prune_trash(kvc: &Path, cutoff: std::time::SystemTime) -> u64 {
    let trash = kvc.join("trash");
    let Ok(rd) = std::fs::read_dir(&trash) else {
        return 0;
    };
    let mut freed = 0u64;
    for e in rd.flatten() {
        let p = e.path();
        if !p.is_dir() {
            continue;
        }
        let expired = e
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .is_some_and(|modified| modified < cutoff);
        if expired {
            freed += dir_size(&p);
            let _ = std::fs::remove_dir_all(&p);
        }
    }
    freed
}

/// `*.tmp` leftovers of `write_atomic`/`write_pack` interrupted by a crash — never cleaned by
/// anything else. Only files older than an hour qualify (paranoia margin; the app is
/// single-process, so anything old is definitively dead).
fn stale_tmp_files(repo: &Repo) -> Vec<(PathBuf, u64)> {
    let kvc = repo.store.clone();
    let mut out = Vec::new();
    for dir in [
        kvc.clone(),
        crate::repo::chains_dir(&repo.store),
        crate::delta::pack_dir(&repo.objects_dir()),
    ] {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_none_or(|x| x != "tmp") {
                continue;
            }
            let Ok(m) = e.metadata() else { continue };
            let old = m
                .modified()
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age > std::time::Duration::from_secs(3600));
            if old {
                out.push((p, m.len()));
            }
        }
    }
    out
}

/// Merge many small live packs into one. Purely a fragmentation fix (no bytes reclaimed):
/// every pack header is parsed when the index loads, so dozens of small packs from mid-size
/// commits add up. Write-before-delete, then invalidate the in-memory index.
fn consolidate_small_packs(repo: &mut Repo) -> Result<()> {
    let objects = repo.objects_dir();
    let pack_dir = crate::delta::pack_dir(&objects);
    let mut small: Vec<(PathBuf, Vec<(String, u64, u32)>)> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&pack_dir) {
        for e in rd.flatten() {
            let path = e.path();
            if path.extension().is_none_or(|x| x != "pack") {
                continue;
            }
            if e.metadata().map(|m| m.len()).unwrap_or(u64::MAX) > CONSOLIDATE_MAX_PACK_BYTES {
                continue;
            }
            if let Some(entries) = crate::delta::read_pack_header(&path) {
                small.push((path, entries));
            }
        }
    }
    if small.len() < CONSOLIDATE_MIN_PACKS {
        return Ok(());
    }
    let mut merged: Vec<(String, Vec<u8>)> = Vec::new();
    let mut seen = HashSet::new();
    for (path, entries) in &small {
        for (name, off, len) in entries {
            if seen.insert(name.clone()) {
                merged.push((
                    name.clone(),
                    crate::delta::read_exact_at(path, *off, *len as usize)?,
                ));
            }
        }
    }
    let refs: Vec<&(String, Vec<u8>)> = merged.iter().collect();
    repo.packs.write_pack(&objects, &refs)?;
    for (path, _) in &small {
        let _ = std::fs::remove_file(path);
    }
    repo.packs.invalidate();
    Ok(())
}

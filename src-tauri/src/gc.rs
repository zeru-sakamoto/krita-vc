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
use crate::error::Result;
use crate::kra;
use crate::repo::{kvc_dir, Chains, Repo};
use serde::Serialize;
use std::collections::HashSet;
use std::path::PathBuf;

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GcReport {
    pub dry_run: bool,
    /// Commits dropped from the log (unreachable from every branch tip).
    pub commits_removed: usize,
    /// Chain versions dropped across all streams.
    pub versions_removed: usize,
    /// Loose object files + whole pack files deleted (rewritten packs count once).
    pub objects_deleted: usize,
    pub bytes_reclaimed: u64,
    /// Raster-cache bytes freed (regenerable previews: over-budget entries pruned, plus a
    /// full wipe when the cache's filter version is stale).
    pub cache_bytes_reclaimed: u64,
}

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
    #[test]
    fn rewrite_gate_over_quarter_dead() {
        assert!(!super::worth_rewriting(0, 100));
        assert!(!super::worth_rewriting(25, 100), "25% dead: keep the pack");
        assert!(super::worth_rewriting(26, 100), ">25% dead: rewrite");
    }
}

/// Collect everything unreachable from the current branch tips. With `dry_run` the report is
/// computed but nothing is written or deleted.
pub fn collect_garbage(repo: &mut Repo, dry_run: bool) -> Result<GcReport> {
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
    for c in repo.commits.iter().filter(|c| reachable.contains(&c.id)) {
        for f in &c.files {
            let Some(content) = &f.content else { continue };
            if f.is_kra {
                live.insert((kra::manifest_stream_key(&f.path), content.clone()));
                let manifest = kra::load_manifest_memo(repo, &f.path, content, &mut manifest_memo)?;
                live.extend(kra::referenced_streams(&f.path, &manifest));
            } else {
                live.insert((format!("file:{}", f.path), content.clone()));
            }
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
    repo.chains.rewrite_all(&kvc_dir(&repo.root), new_chains)?;
    repo.save()?;

    // --- delete loose ---------------------------------------------------------------------
    for (path, _) in &dead_loose {
        let _ = std::fs::remove_file(path);
    }

    // --- delete / rewrite packs ------------------------------------------------------------
    for p in &pack_plans {
        if p.live == 0 {
            let _ = std::fs::remove_file(&p.path);
        } else if worth_rewriting(p.dead_bytes, p.total) {
            // Rewrite with survivors only; write the new pack (or loose files for a small
            // remainder) before deleting the old one, so a crash never loses live objects.
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
            let _ = std::fs::remove_file(&p.path);
        }
    }
    repo.packs.invalidate();

    // --- consolidate small surviving packs into one (fragmentation, not reclamation) -------
    consolidate_small_packs(repo)?;

    // --- stale temp files -------------------------------------------------------------------
    for (path, _) in &stale_tmp {
        let _ = std::fs::remove_file(path);
    }

    // --- raster cache ------------------------------------------------------------------------
    let mut cache_freed = crate::raster::cache_sync_filter_version(&cache_dir);
    cache_freed += crate::raster::cache_prune(&cache_dir, repo.config.cache_max_bytes);
    report.cache_bytes_reclaimed = cache_freed;

    Ok(report)
}

/// `*.tmp` leftovers of `write_atomic`/`write_pack` interrupted by a crash — never cleaned by
/// anything else. Only files older than an hour qualify (paranoia margin; the app is
/// single-process, so anything old is definitively dead).
fn stale_tmp_files(repo: &Repo) -> Vec<(PathBuf, u64)> {
    let kvc = kvc_dir(&repo.root);
    let mut out = Vec::new();
    for dir in [
        kvc.clone(),
        crate::repo::chains_dir(&repo.root),
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

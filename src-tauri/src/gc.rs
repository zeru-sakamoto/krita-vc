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
    let mut live: HashSet<(String, String)> = HashSet::new();
    for c in repo.commits.iter().filter(|c| reachable.contains(&c.id)) {
        for f in &c.files {
            let Some(content) = &f.content else { continue };
            if f.is_kra {
                live.insert((kra::manifest_stream_key(&f.path), content.clone()));
                let manifest = kra::load_manifest(repo, &f.path, content)?;
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
            live_objects.insert(v.object.clone());
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

    let mut report = GcReport {
        dry_run,
        commits_removed,
        versions_removed,
        objects_deleted: dead_loose.len(),
        bytes_reclaimed: dead_loose.iter().map(|(_, len)| len).sum(),
    };
    for p in &pack_plans {
        if p.live == 0 {
            report.objects_deleted += 1;
            report.bytes_reclaimed += p.total;
        } else if p.dead_bytes > 0 {
            report.bytes_reclaimed += p.dead_bytes;
        }
    }

    if dry_run {
        return Ok(report);
    }

    // --- write state FIRST (crash between = harmless re-collectable orphans) -------------
    repo.commits.retain(|c| reachable.contains(&c.id));
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
        } else if p.dead_bytes > 0 {
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

    Ok(report)
}

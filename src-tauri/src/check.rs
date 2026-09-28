//! Read-only repository check — the answer to "is my history intact?".
//!
//! There is no remote to re-fetch from, so the only honest answer to a corrupt store is to find
//! out early. This is a detection pass, not a repair one: it walks the same reachability graph
//! [`crate::gc`] does (via the shared [`crate::gc::mark_live`]) and reports what's broken instead
//! of deleting what isn't reachable. Takes no lock, writes nothing.
//!
//! An opt-in **scrub** (`scrub: true`) additionally re-hashes every live version's content —
//! IO over the whole store, so it's never run automatically, only on explicit request (Settings
//! → Storage → "Check for problems…" or `kvc check --scrub true`). It reuses `Repo::reconstruct_
//! cached` + `Repo::verify_reads` exactly as the restore path does — no new hashing logic.

use crate::error::{KvcError, Result};
use crate::repo::{Commit, Repo};
use serde::Serialize;

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Problem {
    /// `missingObject` | `brokenChain` | `danglingTip` | `missingParent` | `badLogLine` |
    /// `badChains` | `badPack` | `corruptContent`
    pub kind: String,
    pub detail: String,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CheckReport {
    pub commits_checked: usize,
    /// Live stream versions whose backing object was looked for.
    pub objects_checked: usize,
    /// Whether `scrub` was requested for this run.
    pub scrub_performed: bool,
    /// Live versions re-hashed (only non-zero when `scrub_performed`).
    pub versions_scrubbed: usize,
    pub problems: Vec<Problem>,
}

impl CheckReport {
    pub fn ok(&self) -> bool {
        self.problems.is_empty()
    }
}

fn problem(kind: &str, detail: String) -> Problem {
    Problem {
        kind: kind.to_string(),
        detail,
    }
}

pub fn check_repository(repo: &mut Repo, scrub: bool) -> Result<CheckReport> {
    let mut problems: Vec<Problem> = Vec::new();

    // --- branch tips point at commits that exist -----------------------------------------
    let ids: std::collections::HashSet<&str> = repo.commits.iter().map(|c| c.id.as_str()).collect();
    for (name, tip) in &repo.branches.branches {
        if !tip.is_empty() && !ids.contains(tip.as_str()) {
            problems.push(problem(
                "danglingTip",
                format!("branch '{name}' points at unknown version {tip}"),
            ));
        }
    }
    if !repo.branches.branches.contains_key(&repo.branches.current) {
        problems.push(problem(
            "danglingTip",
            format!("current branch '{}' does not exist", repo.branches.current),
        ));
    }

    // --- every version's parents are in the log ---------------------------------------------
    // A log can decode cleanly and still be missing a version later ones build on: the state a
    // damaged line, or an interrupted undo, leaves once the next commit lands on top. Everything
    // older than the gap is then unreachable, and nothing else here would say so.
    for c in &repo.commits {
        for parent in c.parents.iter().filter(|p| !ids.contains(p.as_str())) {
            problems.push(problem(
                "missingParent",
                format!(
                    "version {} builds on {parent}, which the history doesn't have",
                    c.id
                ),
            ));
        }
    }

    // --- every commit-log line decodes ----------------------------------------------------
    // The store opens with every line that does (a torn last line is dropped as a crash
    // leftover); name the ones that don't.
    let log = repo.store.join("commits.log");
    if log.is_file() {
        let bytes = std::fs::read(&log).map_err(|e| crate::error::io_at(&log, e))?;
        for (i, line) in bytes.split(|&b| b == b'\n').enumerate() {
            if line.is_empty() {
                continue;
            }
            if serde_json::from_slice::<Commit>(line).is_err() {
                problems.push(problem(
                    "badLogLine",
                    format!("commits.log line {} is unreadable", i + 1),
                ));
            }
        }
    }

    // --- reachability: every live version has a chain entry and a stored object -----------
    let marks = crate::gc::mark_live(repo, true)?;
    for (path, err) in &marks.problems {
        problems.push(problem("brokenChain", format!("{path}: {err}")));
    }

    // Scrubbing needs `verify_reads` on for the duration — restore it afterward regardless of
    // what it was (it's always `false` on a freshly-opened `Repo`, but don't assume that).
    let prior_verify_reads = repo.verify_reads;
    repo.verify_reads = scrub;
    let mut scrub_memo = crate::delta::ReconstructMemo::default();
    // Each stream's versions in chain order, so a patch's base is the version rebuilt just before
    // it and the memo — a few entries, not the whole history — keeps hitting.
    let chain_pos = |key: &str, hash: &str| {
        marks
            .all
            .0
            .get(key)
            .and_then(|chain| chain.iter().position(|v| v.hash == hash))
    };
    let mut in_order: Vec<(&String, Option<usize>, &String)> = marks
        .marked
        .iter()
        .map(|(key, hash)| (key, chain_pos(key, hash), hash))
        .collect();
    in_order.sort();
    let mut objects_checked = 0usize;
    let mut versions_scrubbed = 0usize;
    for (key, _, hash) in in_order {
        let version = marks
            .all
            .0
            .get(key)
            .and_then(|chain| chain.iter().find(|v| &v.hash == hash));
        let Some(v) = version else {
            problems.push(problem(
                "brokenChain",
                format!("no stored version for {key}@{hash}"),
            ));
            continue;
        };
        objects_checked += 1;
        if !repo.object_exists(&v.object_name()) {
            problems.push(problem(
                "missingObject",
                format!("{} (needed by {key})", v.object_name()),
            ));
            continue;
        }
        if scrub {
            match repo.reconstruct_cached(key, hash, &mut scrub_memo) {
                Ok(_) => versions_scrubbed += 1,
                Err(KvcError::Corrupt(detail)) => {
                    problems.push(problem("corruptContent", detail));
                }
                Err(e) => {
                    problems.push(problem("corruptContent", format!("{key}@{hash}: {e}")));
                }
            }
        }
    }
    repo.verify_reads = prior_verify_reads;

    // --- chain shards decode ----------------------------------------------------------------
    // An unreadable shard reads as empty everywhere else, which shows up as a `brokenChain` per
    // version. Name the file, and keep naming one a save set aside (`Repo::save` moves it to
    // `.corrupt-<time>` rather than write over it) until someone repairs or removes it.
    if let Ok(rd) = std::fs::read_dir(crate::repo::chains_dir(&repo.store)) {
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if name.contains(".bin.corrupt-") {
                problems.push(problem(
                    "badChains",
                    format!("{name} couldn't be decoded and was kept aside; the versions it recorded can't be rebuilt from it"),
                ));
            } else if p.extension().is_some_and(|x| x == "bin")
                && crate::repo::read_chains_file(&p).is_none()
            {
                problems.push(problem("badChains", format!("{name} is unreadable")));
            }
        }
    }

    // --- packs parse ----------------------------------------------------------------------
    // An unparseable pack is skipped everywhere else in the engine, which turns real corruption
    // into a confusing `MissingObject` for each of its contents. Name it directly.
    let pack_dir = crate::delta::pack_dir(&repo.objects_dir());
    if let Ok(rd) = std::fs::read_dir(&pack_dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_none_or(|x| x != "pack") {
                continue;
            }
            if crate::delta::read_pack_header(&p).is_none() {
                problems.push(problem(
                    "badPack",
                    format!(
                        "{} is unreadable",
                        p.file_name().unwrap_or_default().to_string_lossy()
                    ),
                ));
            }
        }
    }

    problems.sort_by(|a, b| (&a.kind, &a.detail).cmp(&(&b.kind, &b.detail)));
    Ok(CheckReport {
        commits_checked: marks.reachable.len(),
        objects_checked,
        scrub_performed: scrub,
        versions_scrubbed,
        problems,
    })
}

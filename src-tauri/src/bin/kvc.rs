//! `kvc` — headless companion CLI over the same `.kvc` engine the Tauri app uses.
//! No Tauri dependency: built for out-of-process callers (the Krita plugin) that need
//! to commit/scan/branch without going through the desktop app's IPC.
//!
//! Usage: kvc <status|commit|branches|switch|create-branch|discard|stash|stash-pop|stash-list>
//!            --repo <path> [flags...]
//! Every subcommand prints one JSON object to stdout on success, or
//! `{"error": "..."}` to stderr with a non-zero exit code on failure.

use krita_vc_lib::branch;
use krita_vc_lib::commands::stash_dtos;
use krita_vc_lib::commit;
use krita_vc_lib::repo::{Repo, RepoLock};
use krita_vc_lib::scan;
use krita_vc_lib::stash;
use serde_json::json;
use std::collections::HashMap;
use std::path::Path;
use std::process::ExitCode;

/// Hand-rolled `--flag value` parser — a handful of subcommands, not worth a dependency.
// Flat arg parse, adopt clap only if subcommands grow.
fn parse_flags(args: &[String]) -> HashMap<String, String> {
    let mut flags = HashMap::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if let Some(name) = arg.strip_prefix("--") {
            if let Some(value) = it.next() {
                flags.insert(name.to_string(), value.clone());
            }
        }
    }
    flags
}

fn require<'a>(flags: &'a HashMap<String, String>, name: &str) -> Result<&'a str, String> {
    flags
        .get(name)
        .map(String::as_str)
        .ok_or_else(|| format!("missing required --{name}"))
}

/// The optional `--paths` subset (commit / discard / stash), as a JSON array of repo-relative
/// paths. A JSON array rather than a comma list because `parse_flags` is a map (a repeated flag
/// would overwrite) and paths may legitimately contain commas.
fn paths_of(flags: &HashMap<String, String>) -> Result<Option<Vec<String>>, String> {
    match flags.get("paths") {
        None => Ok(None),
        Some(raw) => serde_json::from_str(raw)
            .map(Some)
            .map_err(|e| format!("bad --paths (expected a JSON array of relative paths): {e}")),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // The Krita plugin's "Locate kvc…" picker identifies this binary by the literal
    // "usage: kvc" prefix below. Widen the command list freely; don't touch that prefix.
    let Some(cmd) = args.first() else {
        return fail(
            "usage: kvc <status|commit|branches|switch|create-branch|discard|stash|stash-pop|stash-list> --repo <path> [...]",
        );
    };
    let flags = parse_flags(&args[1..]);

    // The plugin parses our stdout/stderr as JSON, so a panic must not escape as a plain-text
    // Rust backtrace. Silence the default hook (it prints to stderr) and turn any unwinding
    // panic — an engine `unwrap`/index on a corrupt store, say — into the same {"error":...}
    // contract every other failure already uses.
    std::panic::set_hook(Box::new(|_| {}));
    let dispatch = std::panic::AssertUnwindSafe(|| match cmd.as_str() {
        "status" => run_status(&flags),
        "commit" => run_commit(&flags),
        "branches" => run_branches(&flags),
        "switch" => run_switch(&flags),
        "create-branch" => run_create_branch(&flags),
        "discard" => run_discard(&flags),
        "stash" => run_stash(&flags),
        "stash-pop" => run_stash_pop(&flags),
        "stash-list" => run_stash_list(&flags),
        other => Err(format!("unknown command: {other}")),
    });
    let result = std::panic::catch_unwind(dispatch).unwrap_or_else(|payload| {
        let detail = payload
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown panic".to_string());
        Err(format!("internal error in kvc: {detail}"))
    });

    match result {
        Ok(value) => {
            println!("{value}");
            ExitCode::SUCCESS
        }
        Err(msg) => fail(&msg),
    }
}

fn fail(msg: &str) -> ExitCode {
    eprintln!("{}", json!({ "error": msg }));
    ExitCode::FAILURE
}

fn run_status(flags: &HashMap<String, String>) -> Result<String, String> {
    let repo_path = require(flags, "repo")?;
    let repo = Repo::open_light(Path::new(repo_path)).map_err(|e| e.to_string())?;
    let changes = scan::scan_detailed(&repo, false).map_err(|e| e.to_string())?;
    let changes: Vec<_> = changes
        .iter()
        .map(|c| json!({ "path": c.rel, "status": c.status }))
        .collect();
    // Stash count rides along so the plugin's 1.5s poll needn't spawn a third process —
    // open_light already read the shelf.
    Ok(json!({
        "branch": repo.branches.current,
        "changes": changes,
        "stashes": repo.stashes.stashes.len(),
    })
    .to_string())
}

fn run_commit(flags: &HashMap<String, String>) -> Result<String, String> {
    let repo_path = require(flags, "repo")?;
    let message = require(flags, "message")?;
    let author = require(flags, "author")?;
    let only = paths_of(flags)?;
    let root = Path::new(repo_path);
    let _lock = RepoLock::acquire(root, "committing").map_err(|e| e.to_string())?;
    let mut repo = Repo::open(root).map_err(|e| e.to_string())?;
    // `only: None` is exactly commit_snapshot — it's a one-line delegate to this.
    let c = commit::commit_selected(&mut repo, message, author, only.as_deref())
        .map_err(|e| e.to_string())?;
    Ok(json!({ "id": c.id, "message": c.message, "timestamp": c.timestamp }).to_string())
}

fn run_branches(flags: &HashMap<String, String>) -> Result<String, String> {
    let repo_path = require(flags, "repo")?;
    let repo = Repo::open_light(Path::new(repo_path)).map_err(|e| e.to_string())?;
    let branches: Vec<_> = repo
        .branches
        .branches
        .iter()
        .map(|(name, tip)| {
            json!({
                "name": name,
                "tip": (!tip.is_empty()).then(|| tip.clone()),
                "current": *name == repo.branches.current,
            })
        })
        .collect();
    Ok(json!({ "current": repo.branches.current, "branches": branches }).to_string())
}

fn run_switch(flags: &HashMap<String, String>) -> Result<String, String> {
    let repo_path = require(flags, "repo")?;
    let name = require(flags, "branch")?;
    let root = Path::new(repo_path);
    let _lock = RepoLock::acquire(root, "switching branches").map_err(|e| e.to_string())?;
    let mut repo = Repo::open(root).map_err(|e| e.to_string())?;
    branch::switch_branch(&mut repo, name).map_err(|e| e.to_string())?;
    Ok(json!({ "ok": true, "current": repo.branches.current }).to_string())
}

fn run_create_branch(flags: &HashMap<String, String>) -> Result<String, String> {
    let repo_path = require(flags, "repo")?;
    let name = require(flags, "name")?;
    let base = flags.get("base").map(String::as_str);
    let root = Path::new(repo_path);
    let _lock = RepoLock::acquire(root, "creating a branch").map_err(|e| e.to_string())?;
    // Mirrors the Tauri command: a plain new-branch-at-tip only needs the light open;
    // basing on another branch materializes that branch's tree, which needs chains.
    let mut repo = if base.is_some() {
        Repo::open(root).map_err(|e| e.to_string())?
    } else {
        Repo::open_light(root).map_err(|e| e.to_string())?
    };
    branch::create_branch(&mut repo, name, base).map_err(|e| e.to_string())?;
    Ok(json!({ "ok": true, "current": repo.branches.current }).to_string())
}

/// Revert working-tree files to the current tip — no new commit. `--paths` omitted discards
/// every dirty file.
fn run_discard(flags: &HashMap<String, String>) -> Result<String, String> {
    let repo_path = require(flags, "repo")?;
    let only = paths_of(flags)?;
    let root = Path::new(repo_path);
    let _lock = RepoLock::acquire(root, "discarding changes").map_err(|e| e.to_string())?;
    let mut repo = Repo::open(root).map_err(|e| e.to_string())?;
    // discard_working_changes takes the tip rather than looking it up, same as the Tauri command.
    let tip = repo
        .branches
        .tip()
        .ok_or("nothing committed yet — no version to revert to")?
        .to_string();
    commit::discard_working_changes(&mut repo, &tip, only.as_deref()).map_err(|e| e.to_string())?;
    Ok(json!({ "ok": true }).to_string())
}

/// Set working-tree changes aside and revert those files. Needs the full open — storing the
/// stashed content writes streams, which a light repo forbids.
fn run_stash(flags: &HashMap<String, String>) -> Result<String, String> {
    let repo_path = require(flags, "repo")?;
    let author = require(flags, "author")?;
    let label = flags.get("label").map(String::as_str).unwrap_or("");
    let only = paths_of(flags)?;
    let root = Path::new(repo_path);
    let _lock = RepoLock::acquire(root, "setting work aside").map_err(|e| e.to_string())?;
    let mut repo = Repo::open(root).map_err(|e| e.to_string())?;
    let s = stash::create(&mut repo, label, author, only.as_deref()).map_err(|e| e.to_string())?;
    Ok(json!({ "id": s.id, "label": s.label, "files": s.files.len() }).to_string())
}

fn run_stash_pop(flags: &HashMap<String, String>) -> Result<String, String> {
    let repo_path = require(flags, "repo")?;
    let id = require(flags, "id")?;
    let root = Path::new(repo_path);
    let _lock =
        RepoLock::acquire(root, "bringing back set-aside work").map_err(|e| e.to_string())?;
    let mut repo = Repo::open(root).map_err(|e| e.to_string())?;
    let s = stash::pop(&mut repo, id).map_err(|e| e.to_string())?;
    Ok(json!({ "ok": true, "id": s.id }).to_string())
}

fn run_stash_list(flags: &HashMap<String, String>) -> Result<String, String> {
    let repo_path = require(flags, "repo")?;
    let repo = Repo::open_light(Path::new(repo_path)).map_err(|e| e.to_string())?;
    // Reuses the desktop's DTO rather than reading repo.stashes directly: it reverses the
    // engine's oldest-first storage to newest-first, which "bring back the latest" relies on.
    Ok(json!({ "stashes": stash_dtos(&repo) }).to_string())
}

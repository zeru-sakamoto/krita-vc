//! `kvc` — headless companion CLI over the same `.kvc` engine the Tauri app uses.
//! No Tauri dependency: built for out-of-process callers (the Krita plugin) that need
//! to commit/scan/branch without going through the desktop app's IPC.
//!
//! Usage: kvc <status|commit|branches|switch|create-branch> --repo <path> [flags...]
//! Every subcommand prints one JSON object to stdout on success, or
//! `{"error": "..."}` to stderr with a non-zero exit code on failure.

use krita_vc_lib::branch;
use krita_vc_lib::commit;
use krita_vc_lib::repo::{Repo, RepoLock};
use krita_vc_lib::scan;
use serde_json::json;
use std::collections::HashMap;
use std::path::Path;
use std::process::ExitCode;

/// Hand-rolled `--flag value` parser — five subcommands, not worth a dependency.
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

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first() else {
        return fail(
            "usage: kvc <status|commit|branches|switch|create-branch> --repo <path> [...]",
        );
    };
    let flags = parse_flags(&args[1..]);

    let result = match cmd.as_str() {
        "status" => run_status(&flags),
        "commit" => run_commit(&flags),
        "branches" => run_branches(&flags),
        "switch" => run_switch(&flags),
        "create-branch" => run_create_branch(&flags),
        other => Err(format!("unknown command: {other}")),
    };

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
    Ok(json!({ "branch": repo.branches.current, "changes": changes }).to_string())
}

fn run_commit(flags: &HashMap<String, String>) -> Result<String, String> {
    let repo_path = require(flags, "repo")?;
    let message = require(flags, "message")?;
    let author = require(flags, "author")?;
    let root = Path::new(repo_path);
    let _lock = RepoLock::acquire(root).map_err(|e| e.to_string())?;
    let mut repo = Repo::open(root).map_err(|e| e.to_string())?;
    let c = commit::commit_snapshot(&mut repo, message, author).map_err(|e| e.to_string())?;
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
    let _lock = RepoLock::acquire(root).map_err(|e| e.to_string())?;
    let mut repo = Repo::open(root).map_err(|e| e.to_string())?;
    branch::switch_branch(&mut repo, name).map_err(|e| e.to_string())?;
    Ok(json!({ "ok": true, "current": repo.branches.current }).to_string())
}

fn run_create_branch(flags: &HashMap<String, String>) -> Result<String, String> {
    let repo_path = require(flags, "repo")?;
    let name = require(flags, "name")?;
    let base = flags.get("base").map(String::as_str);
    let root = Path::new(repo_path);
    let _lock = RepoLock::acquire(root).map_err(|e| e.to_string())?;
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

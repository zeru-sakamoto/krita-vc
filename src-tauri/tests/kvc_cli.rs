//! Integration test for the `kvc` companion CLI (`src/bin/kvc.rs`) — the binary the Krita
//! plugin shells out to. Spawns the real compiled binary against a temp repo so the test
//! covers the same path the plugin exercises (arg parsing, JSON output, the lock file),
//! not just the engine functions it wraps.

use serde_json::Value;
use std::path::Path;
use std::process::Command;

fn kvc(repo: &Path, args: &[&str]) -> (bool, Value) {
    let out = Command::new(env!("CARGO_BIN_EXE_kvc"))
        .args(args)
        .arg("--repo")
        .arg(repo)
        .output()
        .expect("failed to run kvc binary");
    let bytes = if out.status.success() {
        &out.stdout
    } else {
        &out.stderr
    };
    let json: Value = serde_json::from_slice(bytes)
        .unwrap_or_else(|e| panic!("non-JSON output ({e}): {}", String::from_utf8_lossy(bytes)));
    (out.status.success(), json)
}

#[test]
fn status_commit_roundtrip_and_lock() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    krita_vc_lib::repo::Repo::init(root).unwrap();

    // Clean repo, no changes yet.
    let (ok, status) = kvc(root, &["status"]);
    assert!(ok);
    assert_eq!(status["branch"], "main");
    assert_eq!(status["changes"].as_array().unwrap().len(), 0);

    std::fs::write(root.join("hello.txt"), b"hello world").unwrap();

    let (ok, status) = kvc(root, &["status"]);
    assert!(ok);
    let changes = status["changes"].as_array().unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0]["path"], "hello.txt");
    assert_eq!(changes[0]["status"], "U");

    // Commit lands, and the working tree is clean afterward.
    let (ok, commit) = kvc(
        root,
        &["commit", "--message", "first version", "--author", "Zeru"],
    );
    assert!(ok);
    let id = commit["id"].as_str().unwrap().to_string();
    assert!(!id.is_empty());
    assert_eq!(commit["message"], "first version");

    let (ok, status) = kvc(root, &["status"]);
    assert!(ok);
    assert_eq!(status["changes"].as_array().unwrap().len(), 0);

    // The commit is visible to the plain engine too — same store, no divergence.
    let repo = krita_vc_lib::repo::Repo::open(root).unwrap();
    assert!(repo.commits.iter().any(|c| c.id == id));

    // Normal commit releases the lock — no stale file left behind.
    assert!(!root.join(".kvc/kvc.lock").exists());

    // A held lock blocks a concurrent commit with a clear error (checked before the tree
    // is even scanned, so the working tree stays clean for the branch ops below).
    std::fs::write(root.join(".kvc/kvc.lock"), b"").unwrap();
    let (ok, err) = kvc(
        root,
        &["commit", "--message", "blocked", "--author", "Zeru"],
    );
    assert!(!ok);
    assert!(err["error"].as_str().unwrap().contains("busy"));
    std::fs::remove_file(root.join(".kvc/kvc.lock")).unwrap();

    // Branch create/switch round-trip through the same lock-guarded path.
    let (ok, res) = kvc(root, &["create-branch", "--name", "feature"]);
    assert!(ok);
    assert_eq!(res["current"], "feature");

    let (ok, res) = kvc(root, &["switch", "--branch", "main"]);
    assert!(ok);
    assert_eq!(res["current"], "main");

    let (ok, branches) = kvc(root, &["branches"]);
    assert!(ok);
    assert_eq!(branches["current"], "main");
    let names: Vec<&str> = branches["branches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"main") && names.contains(&"feature"));
}

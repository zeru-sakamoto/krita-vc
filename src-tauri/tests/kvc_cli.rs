//! Integration test for the `kvc` companion CLI (`src/bin/kvc.rs`) — the binary the Krita
//! plugin shells out to. Spawns the real compiled binary against a temp document so the test
//! covers the same path the plugin exercises (arg parsing, JSON output, the lock file),
//! not just the engine functions it wraps.
//!
//! `--repo` takes the path to a `.kra` document: one document is one history.

use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

mod common;

/// A minimal but real `.kra` — the commit path parses one as a zip, so a document can't be
/// arbitrary bytes.
fn kra_bytes(tag: &str) -> Vec<u8> {
    let maindoc = format!(
        r#"<!DOCTYPE DOC>
<DOC><IMAGE name="img"><layers>
<layer name="{tag}" uuid="l1" opacity="255" compositeop="normal" nodetype="paintlayer"/>
</layers></IMAGE></DOC>"#
    );
    let mut out = Vec::new();
    {
        let mut zw = zip::ZipWriter::new(std::io::Cursor::new(&mut out));
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        zw.start_file("maindoc.xml", opts).unwrap();
        zw.write_all(maindoc.as_bytes()).unwrap();
        zw.finish().unwrap();
    }
    out
}

/// Create and track one document in `root`, returning its path.
fn init_doc(root: &Path) -> PathBuf {
    let doc = root.join("art.kra");
    std::fs::write(&doc, kra_bytes("start")).unwrap();
    krita_vc_lib::repo::Repo::init(&doc).unwrap();
    doc
}

/// A loose object holding document *content*. A `.kra`'s manifest is an object too, and losing
/// it is reported as a broken chain rather than a missing object.
fn content_object(doc: &Path) -> PathBuf {
    let r = krita_vc_lib::repo::Repo::open(doc).unwrap();
    let store = krita_vc_lib::repo::store_dir_for(doc);
    for (key, versions) in &r.chains.export_all().0 {
        if key.ends_with(":manifest") {
            continue;
        }
        for v in versions {
            let p = store
                .join("objects")
                .join(&v.hash[..2])
                .join(v.object_name());
            if p.is_file() {
                return p;
            }
        }
    }
    panic!("no content object in {store:?}")
}

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
    let doc = init_doc(root);

    // Freshly tracked: the document is there but not yet committed.
    let (ok, status) = kvc(&doc, &["status"]);
    assert!(ok);
    assert_eq!(status["branch"], "main");
    assert_eq!(status["document"], "art.kra");

    std::fs::write(&doc, kra_bytes("hello-world")).unwrap();

    let (ok, status) = kvc(&doc, &["status"]);
    assert!(ok);
    let changes = status["changes"].as_array().unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0]["path"], "art.kra");
    assert_eq!(changes[0]["status"], "U");

    // Commit lands, and the working tree is clean afterward.
    let (ok, commit) = kvc(
        &doc,
        &["commit", "--message", "first version", "--author", "Zeru"],
    );
    assert!(ok);
    let id = commit["id"].as_str().unwrap().to_string();
    assert!(!id.is_empty());
    assert_eq!(commit["message"], "first version");

    let (ok, status) = kvc(&doc, &["status"]);
    assert!(ok);
    assert_eq!(status["changes"].as_array().unwrap().len(), 0);

    // The commit is visible to the plain engine too — same store, no divergence.
    let repo = krita_vc_lib::repo::Repo::open(&doc).unwrap();
    assert!(repo.commits.iter().any(|c| c.id == id));

    // Normal commit releases the OS lock — the marker file itself may persist, but a fresh
    // acquire succeeds immediately since nothing actually holds it anymore.
    assert!(krita_vc_lib::repo::RepoLock::acquire(&doc, "testing").is_ok());

    // A genuinely held lock (this test process holding the real OS lock, not just a leftover
    // file) blocks a concurrent commit from the kvc subprocess with a clear error — checked
    // before the tree is even scanned, so the working tree stays clean for the branch ops
    // below.
    let held = krita_vc_lib::repo::RepoLock::acquire(&doc, "testing").unwrap();
    let (ok, err) = kvc(
        &doc,
        &["commit", "--message", "blocked", "--author", "Zeru"],
    );
    assert!(!ok);
    assert!(err["error"].as_str().unwrap().contains("busy"));
    drop(held);

    // Releasing it frees the lock immediately for the next writer.
    assert!(krita_vc_lib::repo::RepoLock::acquire(&doc, "testing").is_ok());

    // Branch create/switch round-trip through the same lock-guarded path.
    let (ok, res) = kvc(&doc, &["create-branch", "--name", "feature"]);
    assert!(ok);
    assert_eq!(res["current"], "feature");

    let (ok, res) = kvc(&doc, &["switch", "--branch", "main"]);
    assert!(ok);
    assert_eq!(res["current"], "main");

    let (ok, branches) = kvc(&doc, &["branches"]);
    assert!(ok);
    assert_eq!(branches["current"], "main");
    let names: Vec<&str> = branches["branches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"main") && names.contains(&"feature"));

    // `status` carries the same branch list, which is what lets the plugin's 1.5s poll spawn
    // one process instead of two. Both come from `branch_list`, so this pins them together —
    // if they ever drift the docker's branch menu silently goes wrong.
    let (ok, status) = kvc(&doc, &["status"]);
    assert!(ok);
    assert_eq!(status["branch"], branches["current"]);
    assert_eq!(status["branches"], branches["branches"]);
}

/// The staging + set-aside surface the Krita docker drives. Field names and the `--paths`
/// encoding are a contract with `krita-plugin/kritavc/kvc_client.py`, so assert the shapes.
/// `--paths` now only ever names the one document, but the encoding is unchanged.
#[test]
fn staged_commit_stash_and_discard() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let doc = init_doc(root);

    std::fs::write(&doc, kra_bytes("one")).unwrap();
    let (ok, _) = kvc(&doc, &["commit", "--message", "base", "--author", "Zeru"]);
    assert!(ok);

    // A `--paths` selection naming the document commits it.
    std::fs::write(&doc, kra_bytes("one-edited")).unwrap();
    let (ok, _) = kvc(
        &doc,
        &[
            "commit",
            "--message",
            "just the document",
            "--author",
            "Zeru",
            "--paths",
            r#"["art.kra"]"#,
        ],
    );
    assert!(ok);

    let (ok, status) = kvc(&doc, &["status"]);
    assert!(ok);
    assert_eq!(status["changes"].as_array().unwrap().len(), 0);
    // The docker labels its "bring back" actions off this, rather than a third process spawn.
    assert_eq!(status["stashes"], 0);

    // Set the work aside: it reverts on disk and the shelf grows.
    std::fs::write(&doc, kra_bytes("wip")).unwrap();
    let (ok, stashed) = kvc(&doc, &["stash", "--author", "Zeru", "--label", "wip"]);
    assert!(ok);
    let stash_id = stashed["id"].as_str().unwrap().to_string();
    assert_eq!(stashed["files"], 1);

    let (ok, status) = kvc(&doc, &["status"]);
    assert!(ok);
    assert_eq!(status["changes"].as_array().unwrap().len(), 0);
    assert_eq!(status["stashes"], 1);

    // stash-list is newest-first — "bring back latest" takes row 0.
    let (ok, listed) = kvc(&doc, &["stash-list"]);
    assert!(ok);
    let stashes = listed["stashes"].as_array().unwrap();
    assert_eq!(stashes.len(), 1);
    assert_eq!(stashes[0]["id"], stash_id.as_str());
    assert_eq!(stashes[0]["label"], "wip");
    assert_eq!(stashes[0]["changes"][0]["path"], "art.kra");

    let (ok, popped) = kvc(&doc, &["stash-pop", "--id", &stash_id]);
    assert!(ok);
    assert_eq!(popped["ok"], true);

    let (ok, status) = kvc(&doc, &["status"]);
    assert!(ok);
    assert_eq!(status["stashes"], 0);
    assert_eq!(status["changes"].as_array().unwrap().len(), 1);

    // Discard puts it back to the committed content, with no new version recorded.
    let before = krita_vc_lib::repo::Repo::open(&doc).unwrap().commits.len();
    let (ok, res) = kvc(&doc, &["discard", "--paths", r#"["art.kra"]"#]);
    assert!(ok);
    assert_eq!(res["ok"], true);
    let (ok, status) = kvc(&doc, &["status"]);
    assert!(ok);
    assert_eq!(status["changes"].as_array().unwrap().len(), 0);
    assert_eq!(
        krita_vc_lib::repo::Repo::open(&doc).unwrap().commits.len(),
        before
    );
}

/// `--layers` saves only the named top-level layers, the same JSON-array encoding as `--paths`.
/// Nothing calls it today — the Krita docker has no layer picker — but the CLI stays a complete
/// surface over the engine, so the flag is pinned here rather than left to rot untested.
#[test]
fn commit_with_a_layer_subset() {
    let dir = tempfile::tempdir().unwrap();
    let doc = dir.path().join("art.kra");
    std::fs::write(&doc, common::kra_layered(10, 10)).unwrap();
    krita_vc_lib::repo::Repo::init(&doc).unwrap();

    let (ok, _) = kvc(&doc, &["commit", "--message", "base", "--author", "Zeru"]);
    assert!(ok);

    // Edit both layers, then save only "lines".
    std::fs::write(&doc, common::kra_layered(20, 20)).unwrap();
    let (ok, committed) = kvc(
        &doc,
        &[
            "commit",
            "--message",
            "just the lines",
            "--author",
            "Zeru",
            "--layers",
            r#"["lines"]"#,
        ],
    );
    assert!(ok);
    assert!(committed["id"].as_str().is_some());

    // The unticked layer is still unsaved, so the document stays dirty — the same guarantee
    // `tests/staging.rs` pins on the engine side, checked here through the real binary.
    let (ok, status) = kvc(&doc, &["status"]);
    assert!(ok);
    assert_eq!(status["changes"].as_array().unwrap().len(), 1);

    // A malformed value is rejected by the flag parser, not swallowed.
    let (ok, err) = kvc(
        &doc,
        &[
            "commit",
            "--message",
            "m",
            "--author",
            "Z",
            "--layers",
            "lines",
        ],
    );
    assert!(!ok);
    assert!(err["error"].as_str().unwrap().contains("--layers"));
}

/// A repo with problems is still a *successful* check run — `{"error":…}` means the check
/// itself failed, which the plugin distinguishes.
#[test]
fn check_reports_findings_without_failing() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let doc = init_doc(root);
    std::fs::write(&doc, kra_bytes("hello-world")).unwrap();
    let (ok, _) = kvc(&doc, &["commit", "--message", "first", "--author", "Zeru"]);
    assert!(ok);

    let (ok, report) = kvc(&doc, &["check"]);
    assert!(ok);
    assert_eq!(report["ok"], true);
    assert_eq!(report["problems"].as_array().unwrap().len(), 0);
    assert!(report["objectsChecked"].as_u64().unwrap() > 0);

    std::fs::remove_file(content_object(&doc)).unwrap();

    let (ok, report) = kvc(&doc, &["check"]);
    assert!(ok, "a repo with problems is still a successful check run");
    assert_eq!(report["ok"], false);
    assert_eq!(report["problems"][0]["kind"], "missingObject");
}

/// `--scrub true` re-hashes stored content and catches what presence-only `check` can't: a
/// valid-but-wrong object. Default (no `--scrub`) stays silent about the same tamper.
#[test]
fn check_scrub_flag_reports_corruption() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let doc = init_doc(root);
    std::fs::write(&doc, kra_bytes("hello-world")).unwrap();
    let (ok, _) = kvc(&doc, &["commit", "--message", "first", "--author", "Zeru"]);
    assert!(ok);

    let obj = content_object(&doc);
    std::fs::write(&obj, zstd::encode_all(&b"tampered"[..], 1).unwrap()).unwrap();

    let (ok, report) = kvc(&doc, &["check"]);
    assert!(ok);
    assert_eq!(
        report["ok"], true,
        "default check must not notice a content-level tamper"
    );
    assert_eq!(report["scrubPerformed"], false);

    let (ok, report) = kvc(&doc, &["check", "--scrub", "true"]);
    assert!(ok, "a repo with problems is still a successful check run");
    assert_eq!(report["ok"], false);
    assert_eq!(report["scrubPerformed"], true);
    assert_eq!(report["problems"][0]["kind"], "corruptContent");
}

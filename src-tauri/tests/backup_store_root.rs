//! Restoring a backup when the machine keeps history somewhere else.
//!
//! This is its own test **binary** on purpose. The custom store root is app-global process state
//! (`repo::custom_store_root`, cached in a `RwLock` behind a file in the user's app-data dir), so
//! a test that sets it would change where every concurrently-running test in the same binary
//! resolves its store. Cargo runs each integration test file as its own process, and this file
//! holds exactly **one** `#[test]` so nothing races it even here.
//!
//! It also redirects the app-data dir to a tempdir before touching anything, so running the
//! suite never reads or clobbers the developer's real "where version history is kept" setting.

use krita_vc_lib::{commit, repo};

mod common;
use common::kra_bytes;

/// Point `repo::app_data_dir()` at a scratch folder. Must run before any code reads the store
/// root, which is why it is the first thing the single test does.
fn redirect_app_data(dir: &std::path::Path) {
    if cfg!(windows) {
        std::env::set_var("LOCALAPPDATA", dir);
    } else {
        std::env::set_var("XDG_DATA_HOME", dir);
    }
}

fn seed(root: &std::path::Path, name: &str, opacity: i64) -> std::path::PathBuf {
    let path = root.join(name);
    std::fs::write(&path, kra_bytes(100)).unwrap();
    repo::Repo::init(&path).unwrap();
    let mut r = repo::Repo::open(&path).unwrap();
    std::fs::write(&path, kra_bytes(opacity)).unwrap();
    commit::commit_snapshot(&mut r, "c1", "t").unwrap();
    path
}

fn import_all(zip_path: &std::path::Path, dest: &std::path::Path) -> Vec<repo::ImportResult> {
    let manifest = repo::Repo::read_backup_manifest(zip_path).unwrap();
    let items: Vec<repo::ImportItem> = manifest
        .entries
        .iter()
        .map(|e| repo::ImportItem {
            dir: e.dir.clone(),
            dest_dir: dest.to_string_lossy().into_owned(),
        })
        .collect();
    repo::Repo::import_zip(zip_path, &items).unwrap()
}

/// Both directions of the rule that makes a backup portable: **import re-derives where history
/// goes on the importing machine instead of copying the path baked into the archive.**
///
/// Without it, restoring onto a machine with a custom store root drops a `.kvc/` beside the
/// artwork that `store_dir_for` then never looks at — so the app reports the artwork as
/// untracked and offers to start tracking it, minting an empty store beside a perfectly intact
/// history. That was the pre-existing failure this whole feature exists to close.
#[test]
fn import_follows_this_machines_store_root_in_both_directions() {
    let app_data = tempfile::tempdir().unwrap();
    redirect_app_data(app_data.path());
    assert!(
        repo::custom_store_root().is_none(),
        "the redirected app-data dir must start with no store root configured"
    );

    // --- back up with the default layout (history in `.kvc/` beside the artwork) ------------
    let src = tempfile::tempdir().unwrap();
    let doc = seed(src.path(), "art.kra", 1);
    assert!(src.path().join(".kvc").is_dir());
    let out = tempfile::tempdir().unwrap();
    let zip_path = out.path().join("backup.zip");
    repo::Repo::export_zip_multi(&[doc], &zip_path).unwrap();

    // --- restore onto a machine that keeps history under a custom root ----------------------
    let root_store = tempfile::tempdir().unwrap();
    repo::set_custom_store_root(Some(root_store.path())).unwrap();
    assert_eq!(
        repo::custom_store_root().as_deref(),
        Some(root_store.path())
    );

    let dest = tempfile::tempdir().unwrap();
    let results = import_all(&zip_path, dest.path());
    assert!(results[0].error.is_none(), "{:?}", results[0]);
    assert!(results[0].problems.is_empty(), "{:?}", results[0]);

    let restored = dest.path().join("art.kra");
    let store = std::path::Path::new(&results[0].store);
    assert!(restored.is_file(), "the artwork lands in the chosen folder");
    assert!(
        store.starts_with(root_store.path()),
        "history must land under the configured store root, got {store:?}"
    );
    assert_eq!(store, repo::store_dir_for(&restored));
    assert!(
        !dest.path().join(".kvc").exists(),
        "no hidden container may be created beside the artwork when a store root is set"
    );
    assert!(repo::Repo::is_repo(&restored));
    let r = repo::Repo::open(&restored).unwrap();
    assert_eq!(r.commits.len(), 1);
    assert_eq!(r.branches.current, "main");

    // The restored store is live, not just readable.
    std::fs::write(&restored, kra_bytes(5)).unwrap();
    let mut r = repo::Repo::open(&restored).unwrap();
    commit::commit_snapshot(&mut r, "c2", "t").unwrap();
    assert_eq!(repo::Repo::open(&restored).unwrap().commits.len(), 2);

    // --- and back the other way: an archive made *under* a custom root ----------------------
    // Export rebases the store to `.kvc/<slug>/` regardless of where it lived, so clearing the
    // root and importing again must put history back beside the artwork.
    let zip2 = out.path().join("backup2.zip");
    repo::Repo::export_zip_multi(std::slice::from_ref(&restored), &zip2).unwrap();
    repo::set_custom_store_root(None).unwrap();
    assert!(repo::custom_store_root().is_none());

    let dest2 = tempfile::tempdir().unwrap();
    let results = import_all(&zip2, dest2.path());
    assert!(results[0].error.is_none(), "{:?}", results[0]);
    let restored2 = dest2.path().join("art.kra");
    assert!(
        dest2.path().join(".kvc").is_dir(),
        "with no store root, history goes back beside the artwork"
    );
    assert_eq!(
        std::path::Path::new(&results[0].store),
        repo::store_dir_for(&restored2)
    );
    assert_eq!(repo::Repo::open(&restored2).unwrap().commits.len(), 2);
}

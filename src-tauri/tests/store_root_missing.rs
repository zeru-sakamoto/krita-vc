//! A custom store root that isn't there anymore.
//!
//! Its own test **binary** with one `#[test]`, for the reason `backup_store_root.rs` gives: the
//! store root is process-global state, so setting it would move every concurrently running test's
//! store. The app-data dir is redirected first so the developer's real setting is never touched.

use krita_vc_lib::{error::KvcError, repo};

mod common;
use common::kra_bytes;

fn redirect_app_data(dir: &std::path::Path) {
    if cfg!(windows) {
        std::env::set_var("LOCALAPPDATA", dir);
    } else {
        std::env::set_var("XDG_DATA_HOME", dir);
    }
}

/// A store root that was moved or renamed must read as unreachable, never be recreated: a fresh
/// folder there means a new, empty history beside the real one — the case `StoreUnreachable`
/// exists to prevent. Tracking an artwork and restoring one are the two ways a store gets made.
#[test]
fn a_missing_store_root_is_reported_not_recreated() {
    let app_data = tempfile::tempdir().unwrap();
    redirect_app_data(app_data.path());

    // A backup made with the default layout, to restore from below.
    let src = tempfile::tempdir().unwrap();
    let doc = src.path().join("art.kra");
    std::fs::write(&doc, kra_bytes(1)).unwrap();
    repo::Repo::init(&doc).unwrap();
    let out = tempfile::tempdir().unwrap();
    let zip_path = out.path().join("backup.zip");
    repo::Repo::export_zip_multi(&[doc], &zip_path).unwrap();

    let gone = app_data.path().join("history-drive-that-was-renamed");
    repo::set_custom_store_root(Some(&gone)).unwrap();

    let art = tempfile::tempdir().unwrap();
    let fresh = art.path().join("new.kra");
    std::fs::write(&fresh, kra_bytes(2)).unwrap();
    assert!(matches!(
        repo::Repo::init(&fresh),
        Err(KvcError::StoreUnreachable(_))
    ));

    let manifest = repo::Repo::read_backup_manifest(&zip_path).unwrap();
    let items = vec![repo::ImportItem {
        dir: manifest.entries[0].dir.clone(),
        dest_dir: art.path().to_string_lossy().into_owned(),
    }];
    let results = repo::Repo::import_zip(&zip_path, &items).unwrap();
    assert!(results[0].error.is_some(), "{:?}", results[0]);

    assert!(
        !gone.exists(),
        "the missing store root must not be recreated"
    );
}

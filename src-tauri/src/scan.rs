//! Working-tree scanner: classify each file against the committed index as
//! untracked (`U`), modified (`M`), or deleted (`D`). Krita lock/autosave files
//! (`*.kra~`) and the `.kvc/` directory are ignored.

use crate::error::{io_at, KvcError, Result};
use crate::repo::{hash_bytes, Repo};
use std::collections::HashSet;
use std::io;
use std::path::Path;
use walkdir::WalkDir;

/// One working-tree change with everything the scan already computed for it, so the commit
/// path can reuse the hash/size/mtime instead of re-reading and re-hashing the file (a second
/// full read + blake3 pass over a big `.kra` was pure duplication).
pub struct ScanChange {
    pub rel: String,
    /// `U` untracked, `M` modified, `D` deleted.
    pub status: String,
    /// blake3 of the file bytes as scanned (empty for deletions).
    pub hash: String,
    /// Size + mtime taken **before** the scan read its bytes, so a mid-scan edit can only make
    /// them stale in the safe direction (mismatch -> the next scan re-hashes).
    pub size: u64,
    pub mtime: u64,
}

/// Returns `(relativePath, status)` pairs for everything that differs from the index.
/// A tracked file whose size+mtime still match the index is assumed unchanged and skipped
/// without reading/hashing it — the win for big `.kra` files. Anything else is hashed and
/// compared against the committed hash (so a size-preserving edit or an mtime touch is still
/// classified correctly).
pub fn scan(repo: &Repo) -> Result<Vec<(String, String)>> {
    Ok(scan_detailed(repo)?
        .into_iter()
        .map(|c| (c.rel, c.status))
        .collect())
}

/// [`scan`] with the per-file hash/size/mtime kept, for [`crate::commit::commit_snapshot`].
pub fn scan_detailed(repo: &Repo) -> Result<Vec<ScanChange>> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    let walker = WalkDir::new(&repo.root)
        .into_iter()
        .filter_entry(|e| e.file_name() != crate::repo::KVC_DIR);

    for entry in walker {
        let entry = entry.map_err(walk_err)?;
        if !entry.file_type().is_file() {
            continue;
        }
        if entry.file_name().to_string_lossy().ends_with(".kra~") {
            continue;
        }

        let rel = rel_path(&repo.root, entry.path());
        seen.insert(rel.clone());

        // Fast path: a tracked file with matching size+mtime is unchanged — don't read it.
        let tracked = repo.index.files.get(&rel);
        let (size, mtime) = entry
            .metadata()
            .map(|m| crate::repo::size_mtime(&m))
            .unwrap_or((0, 0));
        if let Some(tf) = tracked {
            if size == tf.size && mtime == tf.mtime && (size, mtime) != (0, 0) {
                continue;
            }
        }

        let bytes = std::fs::read(entry.path()).map_err(|e| io_at(entry.path(), e))?;
        let hash = hash_bytes(&bytes);
        let change = |status: &str| ScanChange {
            rel: rel.clone(),
            status: status.into(),
            hash: hash.clone(),
            size,
            mtime,
        };
        match tracked {
            None => out.push(change("U")),
            Some(tf) if tf.hash != hash => out.push(change("M")),
            Some(_) => {}
        }
    }

    for path in repo.index.files.keys() {
        if !seen.contains(path) {
            out.push(ScanChange {
                rel: path.clone(),
                status: "D".into(),
                hash: String::new(),
                size: 0,
                mtime: 0,
            });
        }
    }
    Ok(out)
}

/// Repo-relative path with forward slashes.
fn rel_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn walk_err(e: walkdir::Error) -> KvcError {
    match e.into_io_error() {
        Some(io) => KvcError::Io(io),
        None => KvcError::Io(io::Error::new(
            io::ErrorKind::Other,
            "directory walk failed",
        )),
    }
}

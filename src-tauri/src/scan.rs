//! Working-tree scanner: classify each file against the committed index as
//! untracked (`U`), modified (`M`), or deleted (`D`). Krita lock/autosave files
//! (`*.kra~`) and the `.kvc/` directory are ignored.

use crate::error::{io_at, KvcError, Result};
use crate::repo::{hash_bytes, Repo};
use std::collections::HashSet;
use std::io;
use std::path::Path;
use walkdir::WalkDir;

/// Returns `(relativePath, status)` pairs for everything that differs from the index.
/// A tracked file whose size+mtime still match the index is assumed unchanged and skipped
/// without reading/hashing it — the win for big `.kra` files. Anything else is hashed and
/// compared against the committed hash (so a size-preserving edit or an mtime touch is still
/// classified correctly).
pub fn scan(repo: &Repo) -> Result<Vec<(String, String)>> {
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
        if let Some(tf) = tracked {
            if let Ok(meta) = entry.metadata() {
                let (size, mtime) = crate::repo::size_mtime(&meta);
                if size == tf.size && mtime == tf.mtime && (size, mtime) != (0, 0) {
                    continue;
                }
            }
        }

        let bytes = std::fs::read(entry.path()).map_err(|e| io_at(entry.path(), e))?;
        let hash = hash_bytes(&bytes);
        match tracked {
            None => out.push((rel, "U".into())),
            Some(tf) if tf.hash != hash => out.push((rel, "M".into())),
            Some(_) => {}
        }
    }

    for path in repo.index.files.keys() {
        if !seen.contains(path) {
            out.push((path.clone(), "D".into()));
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

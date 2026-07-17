//! Working-tree scanner: classify each file against the committed index as
//! untracked (`U`), modified (`M`), or deleted (`D`). The `.kvc/` directory and Krita's
//! backup file (`*.kra~`) are ignored by the walk, its autosave artifact
//! (`*-autosave.kra`) by [`is_supported`], and only *supported* file types
//! ([`is_supported`] — `.kra` + palette formats) are ever newly tracked.

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
    /// The file bytes the scan already read, when the caller asked to keep them
    /// (`keep_bytes`) and the retention budget allowed — saves the commit path a second
    /// full read of a big `.kra` (a page-cache miss is a whole extra HDD pass).
    pub bytes: Option<Vec<u8>>,
}

/// Cumulative cap on bytes retained across one scan (`keep_bytes`). Past it, later changed
/// files drop their buffers and the commit path re-reads them — bounds worst-case RAM when a
/// first commit sweeps many large files at once.
const RETAIN_BUDGET: usize = 512 << 20;

/// Returns `(relativePath, status)` pairs for everything that differs from the index.
/// A tracked file whose size+mtime still match the index is assumed unchanged and skipped
/// without reading/hashing it — the win for big `.kra` files. Anything else is hashed and
/// compared against the committed hash (so a size-preserving edit or an mtime touch is still
/// classified correctly).
pub fn scan(repo: &Repo) -> Result<Vec<(String, String)>> {
    Ok(scan_detailed(repo, false)?
        .into_iter()
        .map(|c| (c.rel, c.status))
        .collect())
}

/// [`scan`] with the per-file hash/size/mtime kept, for [`crate::commit::commit_snapshot`].
/// `keep_bytes` additionally hands back each changed file's bytes (under [`RETAIN_BUDGET`]).
pub fn scan_detailed(repo: &Repo, keep_bytes: bool) -> Result<Vec<ScanChange>> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut retained = 0usize;

    // Racy-clean guard (cf. git's index): the size+mtime fast path can't distinguish "unchanged"
    // from "rewritten within the same filesystem mtime tick that the index was last written" — a
    // quick re-save right after a commit keeps the same mtime and, if the byte size is unchanged
    // too ("v1" -> "v2"), the edit would be silently skipped. The index file's own on-disk mtime is
    // the threshold: a working file whose mtime is >= it might have been touched in that same tick,
    // so it's re-hashed rather than trusted. Files committed in an earlier tick keep the fast path;
    // an unreadable index (0) forces hashing everywhere — correct, just slower.
    let index_mtime = std::fs::metadata(crate::repo::kvc_dir(&repo.root).join("index.json"))
        .map(|m| crate::repo::size_mtime(&m).1)
        .unwrap_or(0);

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

        // Tracking guardrail: only Krita documents and diffable palette formats are ever newly
        // tracked. An unsupported file that isn't already in the index is ignored entirely —
        // never staged, hashed, or committed. Already-tracked files stay tracked, so a repo that
        // predates this rule isn't silently pruned.
        if !repo.index.files.contains_key(&rel) && !is_supported(&rel) {
            continue;
        }
        seen.insert(rel.clone());

        // Fast path: a tracked file with matching size+mtime is unchanged — don't read it. Skipped
        // only when the file's mtime predates the index write (`mtime < index_mtime`); a file in the
        // index's own tick is racy (see above) and falls through to hashing.
        let tracked = repo.index.files.get(&rel);
        let (size, mtime) = entry
            .metadata()
            .map(|m| crate::repo::size_mtime(&m))
            .unwrap_or((0, 0));
        if let Some(tf) = tracked {
            if size == tf.size
                && mtime == tf.mtime
                && (size, mtime) != (0, 0)
                && mtime < index_mtime
            {
                continue;
            }
        }

        let bytes = std::fs::read(entry.path()).map_err(|e| io_at(entry.path(), e))?;
        let hash = hash_bytes(&bytes);
        let status = match tracked {
            None => Some("U"),
            Some(tf) if tf.hash != hash => Some("M"),
            Some(_) => None,
        };
        if let Some(status) = status {
            let kept = if keep_bytes && retained + bytes.len() <= RETAIN_BUDGET {
                retained += bytes.len();
                Some(bytes)
            } else {
                None
            };
            out.push(ScanChange {
                rel: rel.clone(),
                status: status.into(),
                hash,
                size,
                mtime,
                bytes: kept,
            });
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
                bytes: None,
            });
        }
    }
    Ok(out)
}

/// The only files Krita VCS tracks: Krita documents (`.kra`) and the color-palette formats it
/// can diff (`.gpl`/`.kpl`/`.aco`/`.ase`). Everything else in the project folder is left alone.
/// One lowercase pass (this runs per untracked file on scan — kept allocation-lean).
pub fn is_supported(rel: &str) -> bool {
    let lower = rel.to_lowercase();
    // Krita's autosave artifact ends in .kra but isn't the artist's document — tracking it would
    // give scratch state its own chain shard. Here rather than in the scan walk (where the
    // backup file is skipped) so an already-tracked one stays tracked.
    if lower.ends_with("-autosave.kra") {
        return false;
    }
    [".kra", ".gpl", ".kpl", ".aco", ".ase"]
        .iter()
        .any(|ext| lower.ends_with(ext))
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

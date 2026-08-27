//! Working-tree scanner: classify the tracked document against the committed index as
//! untracked (`U`), modified (`M`), or deleted (`D`).
//!
//! One store tracks exactly one `.kra`, so there is nothing to *discover* — the document was
//! designated at init. This used to walk the whole project folder looking for anything
//! trackable; now it stats one file, which is also why scanning an art folder holding fifty
//! 400 MB `.kra` files costs nothing.

use crate::error::{io_at, Result};
use crate::repo::{hash_bytes, Repo};

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
    /// (`keep_bytes`) — saves the commit path a second full read of a big `.kra` (a page-cache
    /// miss is a whole extra HDD pass).
    pub bytes: Option<Vec<u8>>,
}

/// Returns `(relativePath, status)` pairs for the tracked document if it differs from the index.
/// A document whose size+mtime still match the index is assumed unchanged and skipped without
/// reading/hashing it — the win for big `.kra` files.
pub fn scan(repo: &Repo) -> Result<Vec<(String, String)>> {
    Ok(scan_detailed(repo, false)?
        .into_iter()
        .map(|c| (c.rel, c.status))
        .collect())
}

/// [`scan`] with the hash/size/mtime kept, for [`crate::commit::commit_snapshot`].
/// `keep_bytes` additionally hands back the file's bytes.
pub fn scan_detailed(repo: &Repo, keep_bytes: bool) -> Result<Vec<ScanChange>> {
    // Racy-clean guard (cf. git's index): the size+mtime fast path can't distinguish "unchanged"
    // from "rewritten within the same filesystem mtime tick that the index was last written" — a
    // quick re-save right after a commit keeps the same mtime and, if the byte size is unchanged
    // too, the edit would be silently skipped. The index file's own on-disk mtime is the
    // threshold: a working file whose mtime is >= it might have been touched in that same tick,
    // so it's re-hashed rather than trusted. A file committed in an earlier tick keeps the fast
    // path; an unreadable index (0) forces hashing — correct, just slower.
    let index_mtime = std::fs::metadata(repo.store.join("index.json"))
        .map(|m| crate::repo::size_mtime(&m).1)
        .unwrap_or(0);

    let mut out = Vec::new();
    for rel in repo.tracked_paths() {
        let abs = crate::repo::safe_join(&repo.root, &rel)?;
        let tracked = repo.index.files.get(&rel);
        let meta = match std::fs::metadata(&abs) {
            Ok(m) if m.is_file() => m,
            // Absent (or replaced by a directory) — a deletion if we had it committed, and
            // nothing at all if we didn't.
            _ => {
                if tracked.is_some() {
                    out.push(ScanChange {
                        rel,
                        status: "D".into(),
                        hash: String::new(),
                        size: 0,
                        mtime: 0,
                        bytes: None,
                    });
                }
                continue;
            }
        };

        let (size, mtime) = crate::repo::size_mtime(&meta);
        if let Some(tf) = tracked {
            if size == tf.size
                && mtime == tf.mtime
                && (size, mtime) != (0, 0)
                && mtime < index_mtime
            {
                continue;
            }
        }

        let bytes = std::fs::read(&abs).map_err(|e| io_at(&abs, e))?;
        let hash = hash_bytes(&bytes);
        let status = match tracked {
            None => "U",
            Some(tf) if tf.hash != hash => "M",
            Some(_) => continue,
        };
        out.push(ScanChange {
            rel,
            status: status.into(),
            hash,
            size,
            mtime,
            bytes: keep_bytes.then_some(bytes),
        });
    }
    Ok(out)
}

/// The only files Krita VCS tracks: Krita documents. Standalone palette files (`.gpl`/`.kpl`/
/// `.aco`/`.ase`) were dropped when tracking went per-document — a `.kra`'s *embedded* document
/// palettes are still parsed and diffed off the `.kra` itself (`commands::kra_palette_dtos`),
/// which is where the value was, and a palette is not a thing an artist versions on its own.
///
/// A suffix match on the whole path, not an extension parse.
pub fn is_supported(rel: &str) -> bool {
    let lower = rel.to_lowercase();
    // Krita's autosave artifact ends in .kra but isn't the artist's document; its backup file
    // (`*.kra~`) doesn't end in `.kra` at all and so falls out for free.
    if lower.ends_with("-autosave.kra") {
        return false;
    }
    lower.ends_with(".kra")
}

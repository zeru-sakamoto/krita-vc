//! Working-tree scanner: classify the tracked document against the committed index as
//! untracked (`U`), modified (`M`), or deleted (`D`).
//!
//! One store tracks exactly one `.kra`, so there is nothing to *discover* — the document was
//! designated at init. This used to walk the whole project folder looking for anything
//! trackable; now it stats one file, which is also why scanning an art folder holding fifty
//! 400 MB `.kra` files costs nothing.

use crate::error::{io_at, Result};
use crate::repo::{hash_bytes, Repo};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Ceiling on what a scan hands back through `keep_bytes`. Retaining the buffer saves the commit
/// path a second full read of the document, which is the right trade for ordinary art; past this
/// it stops being one, because the commit then holds the whole archive *plus* the entry chunk
/// and the prepared objects it is building. Over-budget documents fall back to the re-read in
/// [`crate::commit::store_change`] — that fallback has always been there for exactly this.
pub const RETAIN_BUDGET: u64 = 512 << 20; // 512 MB

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
    /// Set by [`crate::commit::commit_selected`] when `bytes`/`hash` were replaced with a
    /// **synthesized** `.kra` holding only the layers the artist ticked — so the content being
    /// committed is deliberately *not* what sits on disk.
    ///
    /// Load-bearing: it makes [`crate::commit::store_change`] set `TrackedFile::partial`, which is
    /// what keeps the artwork scanning dirty afterwards. The fast path below skips a file whose
    /// size+mtime match the index, and the working file's do — so without the flag the very next
    /// scan would report the artwork **clean** and the unticked layers would silently vanish from
    /// the Changes panel.
    pub partial: bool,
}

/// Returns `(relativePath, status)` pairs for the tracked document if it differs from the index.
/// A document whose size+mtime still match the index is assumed unchanged and skipped without
/// reading/hashing it — the win for big `.kra` files. A document whose last commit was a layer
/// subset ([`crate::repo::TrackedFile::partial`]) is reported modified on the same evidence,
/// also without reading it, and so is one already read at this size and mtime (`worktree.json`):
/// a saved-but-unversioned painting is read once per save, not once per poll.
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
        // The one place that looks beside the artwork regularly, so it's where a crashed
        // working-tree write's artwork-sized temp gets cleared up.
        crate::repo::remove_stale_kvctmp(&abs);
        let tracked = repo.index.files.get(&rel);
        let meta = match std::fs::metadata(&abs) {
            Ok(m) if m.is_file() => m,
            // Absent (or replaced by a directory) — a deletion if we had it committed, and
            // nothing at all if we didn't.
            _ => {
                if tracked.is_some() {
                    out.push(unread(rel, "D", 0, 0));
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
                if !tf.partial {
                    continue;
                }
                // The last commit stored a layer *subset* of this file, so it is dirty by
                // construction — reading it to rediscover that is the one thing this path exists
                // to avoid (`kvc status` is on the plugin's 1.5s poll). Callers that want the
                // bytes fall through and read as usual.
                if !keep_bytes {
                    out.push(unread(rel, "M", size, mtime));
                    continue;
                }
            }
        }

        // Every caller that doesn't want the bytes uses only `rel`/`status`, and neither needs a
        // read for a document with no committed version (it's `U` whatever it holds), nor for one
        // this scan's predecessor already hashed at this exact size and mtime. That second case is
        // the normal state while painting — saved but not yet a version — and the Krita docker
        // polls `kvc status` every 1.5 s on Krita's UI thread: reading and hashing the whole
        // painting on every tick was 78 ms at 105 MB, 136 ms at 195 MB, and seconds from a cold
        // disk.
        if !keep_bytes {
            let known = match tracked {
                None => Some(String::new()),
                Some(_) => remembered_hash(&repo.store, &rel, size, mtime),
            };
            if let Some(hash) = known {
                match tracked {
                    Some(tf) if tf.hash == hash => continue,
                    Some(_) => out.push(unread(rel, "M", size, mtime)),
                    None => out.push(unread(rel, "U", size, mtime)),
                }
                continue;
            }
        }

        let bytes = std::fs::read(&abs).map_err(|e| io_at(&abs, e))?;
        let hash = hash_bytes(&bytes);
        if !keep_bytes {
            remember_hash(&repo.store, &rel, size, mtime, &hash);
        }
        let status = match tracked {
            None => "U",
            Some(tf) if tf.hash != hash => "M",
            Some(_) => continue,
        };
        // Budget on what was actually read, not the pre-read `size` — the file may have grown
        // between the stat and the read, and it is the buffer in hand that costs the RAM.
        let retain = keep_bytes && bytes.len() as u64 <= RETAIN_BUDGET;
        out.push(ScanChange {
            rel,
            status: status.into(),
            hash,
            size,
            mtime,
            bytes: retain.then_some(bytes),
            partial: false,
        });
    }
    Ok(out)
}

/// A change reported without reading the file, so without a hash — same as the deletion arm.
fn unread(rel: String, status: &str, size: u64, mtime: u64) -> ScanChange {
    ScanChange {
        rel,
        status: status.into(),
        hash: String::new(),
        size,
        mtime,
        bytes: None,
        partial: false,
    }
}

/// What the last scan that had to read the working file learned: its size and mtime as stat'ed
/// before the read, and the hash of what it read. `worktree.json` in the store — a cache, written
/// best-effort and read only when it matches exactly, so a missing or unreadable one just means
/// the next scan reads the file again.
#[derive(Serialize, Deserialize)]
struct Worktree {
    rel: String,
    size: u64,
    mtime: u64,
    hash: String,
}

/// The hash `worktree.json` remembers for `rel` at this size and mtime. The same racy-clean guard
/// the index gets in [`scan_detailed`]: trusted only when the file's mtime is strictly older than
/// the sidecar's own, since a rewrite inside the tick the file was stat'ed in would keep its mtime.
fn remembered_hash(store: &Path, rel: &str, size: u64, mtime: u64) -> Option<String> {
    let path = store.join(crate::repo::WORKTREE_FILE);
    let written = crate::repo::size_mtime(&std::fs::metadata(&path).ok()?).1;
    let w: Worktree = serde_json::from_slice(&std::fs::read(&path).ok()?).ok()?;
    (w.rel == rel
        && w.size == size
        && w.mtime == mtime
        && (size, mtime) != (0, 0)
        && mtime < written)
        .then_some(w.hash)
}

/// Record what this scan read (see [`remembered_hash`]). Temp-then-rename under a per-process
/// name, because `kvc status` (no lock — it's the docker's poll) and the desktop app can both be
/// scanning; whichever lands last wins, and both are whole, true records.
fn remember_hash(store: &Path, rel: &str, size: u64, mtime: u64, hash: &str) {
    let record = Worktree {
        rel: rel.to_string(),
        size,
        mtime,
        hash: hash.to_string(),
    };
    let Ok(bytes) = serde_json::to_vec(&record) else {
        return;
    };
    let path = store.join(crate::repo::WORKTREE_FILE);
    let tmp = store.join(format!(
        "{}.{}.tmp",
        crate::repo::WORKTREE_FILE,
        std::process::id()
    ));
    if std::fs::write(&tmp, bytes).is_err() || crate::repo::rename_retrying(&tmp, &path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
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

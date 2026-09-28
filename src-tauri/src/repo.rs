//! `.kvc/` document store: on-disk layout, JSON state schema, and lifecycle (init / open).
//!
//! **One `.kra` document = one history.** A document's store is entirely self-contained and
//! lives beside its `.kra` in a shared, hidden container folder — so an art folder holding
//! seven tracked paintings grows one `.kvc/` with seven independent stores inside it, not seven
//! folders. Nothing is shared between them (deliberately: sharing `objects/` is what would
//! force a project/document split and a repo-wide GC that can delete another document's blobs;
//! cross-document tile dedup is worth roughly nothing, since dedup pays off *within* one
//! painting's history).
//!
//! Layout:
//! ```text
//! artfolder/
//!   painting.kra
//!   .kvc/                    hidden; holds README.txt and one store per tracked document
//!     README.txt
//!     painting-a3f9c1/       <- the store; `Repo::store` points here
//! ```
//!
//! Inside one store:
//! ```text
//! <store>/
//!   doc.json       which document this store belongs to (relpath, display name, created)
//!   config.json    engine config (delta-chain threshold, tile size, cache budget)
//!   index.json     committed head per tracked file (drives the scanner)
//!   worktree.json  what the last scan learned about the saved-but-unversioned file (a cache)
//!   chains/        delta-stream versions (drives storage/restore), zstd-compressed bincode:
//!                  one shard for the manifest and small entries, one per tiled layer entry
//!   commits.log    commit log, JSON-lines, append-only (a commit appends one line instead of
//!                  rewriting the whole history; undo/GC rewrite it)
//!   branches.json  branch name -> tip commit id, plus the current branch
//!   stashes.json   work set aside off to the side of history (also a GC root); absent = empty
//!   objects/       content-addressed blobs (<hash>.full / <hash>.patch)
//!   cache/         capped raster PNGs (bounded, see `raster::cache_prune`)
//! ```

use crate::error::{io_at, KvcError, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

pub const KVC_DIR: &str = ".kvc";

/// The scan's memory of the working file's last hash — see `scan::scan_detailed`.
pub const WORKTREE_FILE: &str = "worktree.json";

/// Join a repo-relative path onto `root`, refusing anything that could escape the repository.
/// Committed file paths live in `commits.log` (plain JSON that travels with a shared `.kvc/`
/// store) and `file` args arrive from the frontend, so both are untrusted: `Path::join` with an
/// absolute path silently replaces `root`, and `..` walks out of it. Only `Normal` components are
/// allowed — this rejects absolute paths, drive/UNC prefixes, root, and `..`.
pub fn safe_join(root: &Path, rel: &str) -> Result<PathBuf> {
    if rel.is_empty() {
        return Err(KvcError::BadPath(rel.to_string()));
    }
    let mut out = root.to_path_buf();
    for comp in Path::new(rel).components() {
        match comp {
            Component::Normal(seg) => out.push(seg),
            _ => return Err(KvcError::BadPath(rel.to_string())),
        }
    }
    Ok(out)
}

/// Max bytes we'll inflate from a single archive entry — a decompression-bomb guard so a
/// malicious `.kra`/`.kpl` can't turn one zip entry into gigabytes of RAM.
// ponytail: fixed 2 GiB ceiling — well above any real single entry, far below an OOM bomb.
// Derive it from canvas dims if a legitimate file ever trips it.
pub const MAX_ARCHIVE_ENTRY_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Read an archive entry into memory with the [`MAX_ARCHIVE_ENTRY_BYTES`] cap. `take` bounds the
/// actual read, so it doesn't trust the entry's declared (spoofable) uncompressed size.
pub fn read_entry_capped(r: impl std::io::Read) -> Result<Vec<u8>> {
    read_capped(r, MAX_ARCHIVE_ENTRY_BYTES)
}

fn read_capped(r: impl std::io::Read, cap: u64) -> Result<Vec<u8>> {
    use std::io::Read;
    let mut buf = Vec::new();
    r.take(cap + 1).read_to_end(&mut buf)?;
    if buf.len() as u64 > cap {
        return Err(KvcError::CorruptZip(
            "archive entry is implausibly large (possible decompression bomb)".into(),
        ));
    }
    Ok(buf)
}

/// Real OS-level exclusive lock over one `.kvc/` store (`.kvc/kvc.lock`, held via
/// `File::try_lock`). The engine has no internal locking, so every mutating entry point — the
/// desktop app's Tauri commands and the `kvc` CLI alike — takes this so a plugin commit can't
/// interleave with a desktop commit/switch/GC into a torn write. Unlike a plain marker file,
/// the OS releases this the moment the holding process's file handle closes — cleanly, on a
/// panic-unwind drop, or on a crash/force-kill — so there is no "stale lock" state to clean up:
/// the very next `try_lock()` on an orphaned file just succeeds.
// The File is never read/written after acquire — it's held purely so its Drop (closing the
// handle) releases the OS lock; the compiler can't see that use, hence the allow.
pub struct RepoLock(#[allow(dead_code)] std::fs::File);

impl RepoLock {
    /// `op` is a short present-participle label ("committing", "switching branches") written
    /// into the `kvc.lock.info` sidecar so a blocked caller's error can say what's holding it,
    /// not just that something is. It goes in a *separate* file rather than `kvc.lock` itself
    /// because Windows enforces a locked byte range against ordinary reads too (unlike POSIX
    /// `flock`, which is purely advisory) — a blocked caller reading `kvc.lock` directly would
    /// hit `ERROR_LOCK_VIOLATION`. The sidecar is never locked, so it's always readable.
    pub fn acquire(kra_path: &Path, op: &str) -> Result<Self> {
        let store = store_dir_for(kra_path);
        let path = store.join("kvc.lock");
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&path)
            .map_err(|e| io_at(&path, e))?;
        match file.try_lock() {
            Ok(()) => {
                let _ = write_lock_info(&store, op); // best-effort; never blocks the acquire
                Ok(RepoLock(file))
            }
            Err(std::fs::TryLockError::WouldBlock) => Err(KvcError::Locked(
                // Name the *artwork*, not the store — "painting.kra — committing for 3s" is
                // what the artist can act on.
                lock_holder_description(kra_path, &lock_info_path(&store)),
            )),
            Err(std::fs::TryLockError::Error(e)) => Err(io_at(&path, e)),
        }
    }
}
// No `impl Drop`: dropping `File` closes the handle, and the OS releases the lock along with
// it — including when the process is killed, which is exactly the case a marker file can't.

fn lock_info_path(store: &Path) -> PathBuf {
    store.join("kvc.lock.info")
}

/// Best-effort rewrite of the `kvc.lock.info` sidecar right after acquiring the real lock.
fn write_lock_info(store: &Path, op: &str) -> std::io::Result<()> {
    std::fs::write(lock_info_path(store), op)
}

/// Best-effort "<repo> — <op> for <age>" detail for a `Locked` error: which repo, what the
/// other holder is doing (from the sidecar's contents), and how long it's been going (from
/// the sidecar's mtime, rewritten on every successful acquire) — enough to tell a genuinely
/// slow operation from one that's been stuck a suspiciously long time.
fn lock_holder_description(root: &Path, info_path: &Path) -> String {
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.display().to_string());
    let op = std::fs::read_to_string(info_path)
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "writing".to_string());
    let age = std::fs::metadata(info_path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .map(format_age);
    match age {
        Some(age) => format!("{name} — {op} for {age}"),
        None => format!("{name} — {op}"),
    }
}

fn format_age(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else {
        format!("{}h", secs / 3600)
    }
}

fn walk_err(e: walkdir::Error) -> KvcError {
    match e.into_io_error() {
        Some(io) => KvcError::Io(io),
        None => KvcError::Io(std::io::Error::new(
            std::io::ErrorKind::Other,
            "directory walk failed",
        )),
    }
}

fn zip_err(e: zip::result::ZipError) -> KvcError {
    KvcError::CorruptZip(e.to_string())
}

/// Reopen a just-written zip and confirm it's actually readable before the caller reports
/// success: every entry landed, and — if one was written — `MANIFEST.json` is present and
/// parses. A backup nobody can verify isn't a backup. Split out from [`Repo::export_zip_multi`] so the
/// verification itself is directly unit-testable against a hand-built bad archive.
fn verify_zip(dest: &Path, expected_entries: usize, expect_manifest: bool) -> Result<()> {
    let readback = std::fs::File::open(dest).map_err(|e| io_at(dest, e))?;
    let mut za = zip::ZipArchive::new(readback).map_err(zip_err)?;
    if za.len() != expected_entries {
        return Err(KvcError::CorruptZip(format!(
            "backup verification failed: wrote {expected_entries} entries, archive has {}",
            za.len()
        )));
    }
    if expect_manifest {
        let mut mf = za.by_name("MANIFEST.json").map_err(zip_err)?;
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut mf, &mut buf).map_err(|e| io_at(dest, e))?;
        serde_json::from_slice::<BackupManifest>(&buf)
            .map_err(|e| KvcError::CorruptZip(format!("unreadable backup manifest: {e}")))?;
    }
    Ok(())
}

/// Where one artwork in a backup would land, and what's already there. Purely a proposal —
/// nothing is written until the caller turns rows it kept into [`ImportItem`]s.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestorePlan {
    pub dir: String,
    pub relpath: String,
    pub original_dir: String,
    /// The folder it was backed up from still exists, so it can go straight back.
    pub original_dir_exists: bool,
    /// Where it will actually go: the original folder if that still exists, else a subfolder of
    /// the fallback the user picked.
    pub dest_dir: String,
    pub dest_path: String,
    /// A file is already there — restoring overwrites it.
    pub occupied: bool,
    /// …and it's already a tracked artwork, so its history would be replaced too.
    pub tracked: bool,
}

/// One version, stripped to what a side-by-side comparison shows. Deliberately not a whole
/// [`Commit`]: the compare view lists version number, message and date, and `files` would drag
/// every content hash of every version across the IPC boundary for nothing.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionRow {
    pub id: String,
    pub message: String,
    pub timestamp: String,
    pub author: String,
    pub branch: String,
}

impl From<&Commit> for VersionRow {
    fn from(c: &Commit) -> Self {
        VersionRow {
            id: c.id.clone(),
            message: c.message.clone(),
            timestamp: c.timestamp.clone(),
            author: c.author.clone(),
            branch: c.branch.clone(),
        }
    }
}

impl Repo {
    /// Resolve every artwork in an archive to a destination. Kept in Rust rather than joining
    /// paths in the UI: this is a filesystem question (does the original folder still exist? is
    /// something already sitting there?) and platform path joining is not the frontend's job.
    pub fn plan_restore(archive: &Path, fallback_dir: Option<&Path>) -> Result<Vec<RestorePlan>> {
        let manifest = Self::read_backup_manifest(archive)?;
        Ok(manifest
            .entries
            .iter()
            .map(|e| {
                let original = Path::new(&e.original_dir);
                let original_dir_exists = !e.original_dir.is_empty() && original.is_dir();
                // One subfolder per artwork under the fallback, named by the archive's own
                // folder — the slug is unique per document, so two artworks that share a
                // filename can't collide the way a flat restore would let them.
                let dest_dir = if original_dir_exists {
                    original.to_path_buf()
                } else {
                    fallback_dir.map(|f| f.join(&e.dir)).unwrap_or_default()
                };
                let dest_path = safe_join(&dest_dir, &e.relpath).unwrap_or_default();
                RestorePlan {
                    dir: e.dir.clone(),
                    relpath: e.relpath.clone(),
                    original_dir: e.original_dir.clone(),
                    original_dir_exists,
                    dest_dir: dest_dir.to_string_lossy().into_owned(),
                    dest_path: dest_path.to_string_lossy().into_owned(),
                    occupied: dest_path.is_file(),
                    tracked: Repo::is_repo(&dest_path),
                }
            })
            .collect())
    }
}

/// Store files not worth carrying into a backup. `cache/` regenerates on demand and is budgeted
/// at [`Config::cache_max_bytes`] (256 MB by default) *per store*, so it's easily the largest
/// disposable chunk; `trash/` is already-deleted history serving out its retention window; the
/// rest is transient process state a restored store must not inherit.
fn skip_in_backup(rel: &str) -> bool {
    rel.starts_with("cache/")
        || rel.starts_with("trash/")
        || rel.starts_with("kvc.lock")
        || rel == WORKTREE_FILE
        || rel.ends_with(".tmp")
        || rel.ends_with(".kvctmp")
}

/// Stream one file into an open archive, straight from its handle. `compress` only for what still
/// shrinks: the `.kra` is already a zip, and objects, packs and chain shards are zstd, so deflating
/// them again was most of a backup's time (7.2 s of 7.6 s on a 105 MB painting) for a 13% smaller
/// archive. The JSON state files and logs still deflate well.
fn zip_file(
    zw: &mut ZipWriter<std::fs::File>,
    name: &str,
    path: &Path,
    compress: bool,
) -> Result<()> {
    // One open per file: its length comes from the handle. A backup of a long history is
    // thousands of small files, and on Windows each open is a trip through the filter drivers.
    let mut f = std::fs::File::open(path).map_err(|e| io_at(path, e))?;
    let len = f.metadata().map_err(|e| io_at(path, e))?.len();
    let method = if compress {
        CompressionMethod::Deflated
    } else {
        CompressionMethod::Stored
    };
    let opts = SimpleFileOptions::default()
        .compression_method(method)
        .large_file(len >= u32::MAX as u64);
    zw.start_file(name, opts).map_err(zip_err)?;
    std::io::copy(&mut f, zw).map_err(|e| io_at(path, e))?;
    Ok(())
}

/// Write one artwork (document + store) into an open backup archive under `dir/`.
fn zip_one_document(
    zw: &mut ZipWriter<std::fs::File>,
    kra_path: &Path,
    dir: &str,
    entry_count: &mut usize,
) -> Result<BackupEntry> {
    if !Repo::is_repo(kra_path) {
        return Err(KvcError::NotARepo(kra_path.to_path_buf()));
    }
    // Reading, but under the lock every writer takes: the Krita docker can commit, switch or set
    // work aside through `kvc` mid-backup, and zipping across one of those pairs the artwork from
    // one side of it with a store from the other — an artwork that doesn't match its own history.
    // A busy artwork fails into the caller's list instead ("busy: … committing for 3s"). A store
    // the lock file can't even be created in (read-only media) can't be written by anyone else
    // either, so it's backed up without one rather than refused.
    let _lock = match RepoLock::acquire(kra_path, "backing up") {
        Ok(lock) => Some(lock),
        Err(e @ KvcError::Locked(_)) => return Err(e),
        Err(_) => None,
    };
    let store = store_dir_for(kra_path);
    let relpath = doc_relpath(kra_path)?;

    // The document itself.
    zip_file(zw, &format!("{dir}/{relpath}"), kra_path, false)?;
    *entry_count += 1;

    // Its store, rebased under `<dir>/.kvc/<slug>/` so the layout survives extraction even when
    // the store currently lives under a custom root somewhere else entirely.
    let slug = store
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "store".into());
    for entry in walkdir::WalkDir::new(&store) {
        let entry = entry.map_err(walk_err)?;
        if !entry.file_type().is_file() {
            continue;
        }
        // Zip entry names are `/`-separated regardless of platform.
        let rel = entry
            .path()
            .strip_prefix(&store)
            .unwrap_or(entry.path())
            .to_string_lossy()
            .replace('\\', "/");
        if skip_in_backup(&rel) {
            continue;
        }
        let compress = !(rel.starts_with("objects/") || rel.starts_with("chains/"));
        zip_file(
            zw,
            &format!("{dir}/{KVC_DIR}/{slug}/{rel}"),
            entry.path(),
            compress,
        )?;
        *entry_count += 1;
    }

    // Best-effort branch state: a document whose branches will not load still gets backed up and
    // still gets a manifest entry, because the entry is what makes it importable later.
    let (branch, tip_commit) = Repo::open_light(kra_path)
        .ok()
        .map(|r| {
            (
                r.branches.current.clone(),
                r.branches.tip().unwrap_or("").to_string(),
            )
        })
        .unwrap_or_default();
    Ok(BackupEntry {
        dir: dir.to_string(),
        relpath,
        original_dir: doc_root(kra_path).to_string_lossy().into_owned(),
        branch,
        tip_commit,
    })
}

/// The body of [`Repo::export_zip_multi`]: every artwork it can back up into one verified archive
/// at `path`, and the ones it couldn't. An archive of nothing is an error, never a file on disk
/// claiming to be a backup.
fn write_backup(kra_paths: &[PathBuf], path: &Path) -> Result<Vec<String>> {
    let file = std::fs::File::create(path).map_err(|e| io_at(path, e))?;
    let mut zw = ZipWriter::new(file);
    let mut entry_count = 0usize;
    let (mut entries, mut failed) = (Vec::new(), Vec::new());
    let mut seen = HashSet::new();

    for kra_path in kra_paths {
        // The salted slug is unique per document by construction (it hashes the parent dir
        // too), so two same-named paintings from different folders can't collide and no
        // dedup bookkeeping is needed.
        let salt = kra_path
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let dir = store_slug(kra_path, &salt);
        if !seen.insert(dir.clone()) {
            continue; // the same document listed twice
        }
        match zip_one_document(&mut zw, kra_path, &dir, &mut entry_count) {
            Ok(entry) => entries.push(entry),
            Err(_) => failed.push(kra_path.to_string_lossy().into_owned()),
        }
    }

    if entries.is_empty() {
        return Err(KvcError::Io(std::io::Error::other(
            "nothing could be backed up",
        )));
    }

    let manifest = BackupManifest {
        version: 2,
        timestamp: now_iso(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        entries,
    };
    let bytes = serde_json::to_vec(&manifest).map_err(|e| KvcError::BadIndex(e.to_string()))?;
    zw.start_file(
        "MANIFEST.json",
        SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
    )
    .map_err(zip_err)?;
    zw.write_all(&bytes).map_err(|e| io_at(path, e))?;
    entry_count += 1;
    zw.finish().map_err(zip_err)?;
    verify_zip(path, entry_count, true)?;
    Ok(failed)
}

fn failed_import(dir: &str, path: &str, store: &str, error: String) -> ImportResult {
    ImportResult {
        dir: dir.to_string(),
        path: path.to_string(),
        name: Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        store: store.to_string(),
        problems: Vec::new(),
        error: Some(error),
        replaced_artwork: None,
        replaced_history: None,
    }
}

/// `path` with `suffix` appended to its file name.
fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

/// Where Replace keeps the artwork it replaces: `art.replaced-<time>.kra`, beside it and still
/// something Krita opens with a double-click.
fn replaced_artwork_path(dest: &Path, stamp: &str) -> PathBuf {
    let stem = dest
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    dest.with_file_name(format!("{stem}.replaced-{stamp}.kra"))
}

/// Restore one artwork out of an open archive. See [`Repo::import_zip`] for why the store's
/// location is recomputed here rather than taken from the archive.
///
/// Replace never destroys what it replaces. The incoming history is unpacked into a staging
/// folder beside its final place first, so a bad archive fails before anything that's already
/// there is touched. Only then are the current artwork and history renamed aside — same folder,
/// same volume, so a rename that can't happen fails instead of quietly turning into a delete the
/// way a Recycle Bin move can on a network share — and the restored ones put in their place.
/// The old history waits beside the new store for a cleanup to age it out
/// (`gc::prune_aged`), and the old artwork stays in the art folder for the artist to keep or
/// bin; `ImportResult` says where both went.
fn import_one(
    za: &mut zip::ZipArchive<std::fs::File>,
    entry: &BackupEntry,
    dest_dir: &Path,
) -> Result<ImportResult> {
    // `safe_join` is the zip-slip guard: entry names are untrusted, and every write below is
    // rooted through it.
    let dest = safe_join(dest_dir, &entry.relpath)?;
    let prefix = format!("{}/{KVC_DIR}/", entry.dir);
    refuse_missing_store_root(&dest)?;

    // Read before anything is written, so a failure reading the archive changes nothing.
    let bytes = {
        let f = za
            .by_name(&format!("{}/{}", entry.dir, entry.relpath))
            .map_err(zip_err)?;
        read_entry_capped(f)?
    };
    std::fs::create_dir_all(dest_dir).map_err(|e| io_at(dest_dir, e))?;

    // Where *this machine* keeps history — never the path baked into the archive.
    let store = store_dir_for(&dest);
    if let Some(container) = store.parent() {
        std::fs::create_dir_all(container).map_err(|e| io_at(container, e))?;
        // Only the default in-folder container gets hidden + a README; a user-chosen store root
        // is a folder they picked themselves and expect to see. Mirrors `Repo::init`.
        if custom_store_root().is_none() {
            dress_container(container);
        }
    }

    // Fixed name, cleared first: a leftover from a restore that crashed midway goes the next
    // time this artwork is restored. Nothing else ever looks at it.
    let staging = with_suffix(&store, ".restoring");
    if staging.exists() {
        std::fs::remove_dir_all(&staging).map_err(|e| io_at(&staging, e))?;
    }
    if let Err(e) = unpack_store(za, &prefix, &staging, &entry.relpath) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(e);
    }

    let stamp = now_iso_filesafe();
    let mut replaced_history = None;
    if store.exists() {
        // Not while something is writing to it — the Krita docker's `kvc`, mid-commit.
        if Repo::is_repo(&dest) {
            drop(RepoLock::acquire(&dest, "restoring a backup")?);
        }
        let aside = with_suffix(&store, &format!(".replaced-{stamp}"));
        if let Err(e) = rename_retrying(&store, &aside) {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(io_at(&store, e));
        }
        replaced_history = Some(aside);
    }
    // Every step from here puts back what came before it if it fails, so a failed Replace
    // leaves the artwork and its history exactly as they were.
    let undo_history = |replaced: &Option<PathBuf>| {
        if let Some(aside) = replaced {
            let _ = rename_retrying(aside, &store);
        }
    };
    if let Err(e) = rename_retrying(&staging, &store) {
        undo_history(&replaced_history);
        let _ = std::fs::remove_dir_all(&staging);
        return Err(io_at(&store, e));
    }
    let mut replaced_artwork = None;
    if dest.is_file() {
        let aside = replaced_artwork_path(&dest, &stamp);
        if let Err(e) = rename_retrying(&dest, &aside) {
            let _ = std::fs::remove_dir_all(&store);
            undo_history(&replaced_history);
            return Err(io_at(&dest, e));
        }
        replaced_artwork = Some(aside);
    }
    if let Err(e) = write_file_atomic(&dest, &bytes) {
        if let Some(aside) = &replaced_artwork {
            let _ = rename_retrying(aside, &dest);
        }
        let _ = std::fs::remove_dir_all(&store);
        undo_history(&replaced_history);
        return Err(e);
    }

    // Reuse the read-only integrity check rather than inventing an import-specific one: it
    // already covers dangling tips, undecodable log lines, broken chains, missing objects and
    // unreadable packs. Findings are reported, not fatal — a partly-good store beats none.
    let mut repo = Repo::open(&dest)?;
    let report = crate::check::check_repository(&mut repo, false)?;
    let shown = |p: Option<PathBuf>| p.map(|p| p.to_string_lossy().into_owned());
    Ok(ImportResult {
        dir: entry.dir.clone(),
        path: dest.to_string_lossy().into_owned(),
        name: entry.relpath.clone(),
        store: store.to_string_lossy().into_owned(),
        problems: report
            .problems
            .iter()
            .map(|p| format!("{}: {}", p.kind, p.detail))
            .collect(),
        error: None,
        replaced_artwork: shown(replaced_artwork),
        replaced_history: shown(replaced_history),
    })
}

/// Extract one artwork's store out of the archive into `into`, then confirm it's the history of
/// the document the archive says it is.
fn unpack_store(
    za: &mut zip::ZipArchive<std::fs::File>,
    prefix: &str,
    into: &Path,
    relpath: &str,
) -> Result<()> {
    std::fs::create_dir_all(into).map_err(|e| io_at(into, e))?;
    for i in 0..za.len() {
        // Filter on the central directory's name first: only this artwork's entries are worth
        // the seek to their local header that `by_index` costs.
        let Some(rel) = za
            .name_for_index(i)
            .and_then(|name| name.strip_prefix(prefix))
            // `<slug>/<rel>` — the archive's slug is payload, `rel` is what we keep.
            .and_then(|after| after.split_once('/'))
            .map(|(_slug, rel)| rel.to_string())
        else {
            continue;
        };
        if rel.is_empty() || rel.ends_with('/') || skip_in_backup(&rel) {
            continue;
        }
        let mut f = za.by_index(i).map_err(zip_err)?;
        if f.is_dir() {
            continue;
        }
        let out = safe_join(into, &rel)?;
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| io_at(parent, e))?;
        }
        let bytes = read_entry_capped(&mut f)?;
        // Plain writes, not `write_atomic`: nothing here is live until the whole store has
        // landed and been renamed into place. Paying an fsync per object would make restoring a
        // large history needlessly slow.
        std::fs::write(&out, &bytes).map_err(|e| io_at(&out, e))?;
    }

    // By construction these match, since nothing renames. A hand-edited archive could disagree,
    // and a mismatch means every later scan and commit targets a file that is not there.
    let meta = read_doc_meta(into)?;
    if meta.relpath != relpath {
        return Err(KvcError::BadIndex(format!(
            "backup is inconsistent: its history is for {:?}, but the archive holds {:?}",
            meta.relpath, relpath
        )));
    }
    Ok(())
}

pub fn objects_dir(store: &Path) -> PathBuf {
    store.join("objects")
}
/// Content-addressed capped-raster cache (see `raster::cache_read`/`cache_write`). Created by
/// `init`; writes `create_dir_all` lazily.
pub fn cache_dir(store: &Path) -> PathBuf {
    store.join("cache")
}
/// Per-file chain shards (see [`ChainStore`]).
pub fn chains_dir(store: &Path) -> PathBuf {
    store.join("chains")
}

/// Which document a store belongs to. The durable record: `relpath` is the document's name
/// within its folder, so a store is still identifiable after the app forgets about it, and a
/// later rename can re-point by content hash without guessing from the directory name.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocMeta {
    /// The document's path relative to [`Repo::root`] — always a bare filename, since a store
    /// always lives in the `.kvc/` beside its own document.
    pub relpath: String,
    pub display_name: String,
    pub created_at: String,
}

/// Directory name for one document's store. The sanitised file stem keeps it recognisable in
/// Explorer; the short hash keeps two documents whose names sanitise identically ("a b.kra" and
/// "a-b.kra") from colliding.
pub fn store_slug(kra_path: &Path, salt: &str) -> String {
    let stem = kra_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut safe: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    safe.truncate(40);
    let safe = safe.trim_matches('-').to_string();
    let name = kra_path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let digest = hash_bytes(format!("{salt}{name}").as_bytes());
    if safe.is_empty() {
        format!("doc-{}", &digest[..12])
    } else {
        format!("{safe}-{}", &digest[..6])
    }
}

/// App-global override for where stores are kept, or `None` for the default (beside the
/// document). Read from a plain JSON file rather than passed in per call, so the Tauri app and
/// the `kvc` CLI — which never sees the app's settings — resolve the same store for the same
/// document.
pub fn custom_store_root() -> Option<PathBuf> {
    if let Some(cached) = CUSTOM_ROOT.read().ok().and_then(|c| c.clone()) {
        return cached;
    }
    let fresh = read_custom_store_root();
    if let Ok(mut c) = CUSTOM_ROOT.write() {
        *c = Some(fresh.clone());
    }
    fresh
}

/// `None` = not yet read; `Some(None)` = read, and there is no override.
static CUSTOM_ROOT: RwLock<Option<Option<PathBuf>>> = RwLock::new(None);

fn read_custom_store_root() -> Option<PathBuf> {
    let raw = std::fs::read_to_string(store_root_config_path()?).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let s = v.get("root")?.as_str()?.trim().to_string();
    (!s.is_empty()).then(|| PathBuf::from(s))
}

/// Set (or clear, with `None`) the app-global store root. Existing stores are **not** moved —
/// the setting only decides where the *next* document's store is created, and where documents
/// created under it are looked up.
pub fn set_custom_store_root(root: Option<&Path>) -> Result<()> {
    let path = store_root_config_path()
        .ok_or_else(|| KvcError::BadIndex("no application data directory".into()))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| io_at(parent, e))?;
    }
    let body = match root {
        Some(r) => json_object_root(&r.to_string_lossy()),
        None => "{}".to_string(),
    };
    std::fs::write(&path, body).map_err(|e| io_at(&path, e))?;
    if let Ok(mut c) = CUSTOM_ROOT.write() {
        *c = None; // re-read on next use
    }
    Ok(())
}

fn json_object_root(value: &str) -> String {
    serde_json::json!({ "root": value }).to_string()
}

fn store_root_config_path() -> Option<PathBuf> {
    Some(app_data_dir()?.join("storeRoot.json"))
}

/// Per-user application data directory, resolved without Tauri so the CLI shares it.
fn app_data_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")));
    Some(base?.join("com.zeru-sakamoto.krita-vc"))
}

/// Where this document's store lives. Default is the `.kvc/` container beside the document, so
/// history travels with the art and lands on the same drive. A custom root moves every *new*
/// store under one folder instead; the slug is then salted with the document's full path, since
/// two folders can hold same-named paintings.
pub fn store_dir_for(kra_path: &Path) -> PathBuf {
    match custom_store_root() {
        Some(root) => {
            let salt = kra_path
                .parent()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            root.join(store_slug(kra_path, &salt))
        }
        None => doc_root(kra_path)
            .join(KVC_DIR)
            .join(store_slug(kra_path, "")),
    }
}

/// The working tree for a document: the folder holding it.
pub fn doc_root(kra_path: &Path) -> PathBuf {
    kra_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// The document's name within [`doc_root`]. Always a bare filename.
pub fn doc_relpath(kra_path: &Path) -> Result<String> {
    kra_path
        .file_name()
        .map(|n| n.to_string_lossy().replace('\\', "/"))
        .filter(|n| !n.is_empty())
        .ok_or_else(|| KvcError::BadPath(kra_path.to_string_lossy().into_owned()))
}

const CONTAINER_README: &str = "\
This folder holds the version history for the artwork in this folder.

Each subfolder is one artwork's saved versions. Deleting this folder deletes every version
of every artwork here — the artwork files themselves are not stored in here, but their
history is, and it cannot be recovered afterwards.

Krita VC creates and manages this folder. You do not need to open it.
";

/// Make the container folder unobtrusive: hidden in Explorer's default view, and carrying a
/// note explaining what it is for anyone who turns hidden files on. Both best-effort — neither
/// failing is a reason to refuse to create a store.
fn dress_container(container: &Path) {
    let readme = container.join("README.txt");
    if !readme.exists() {
        let _ = std::fs::write(&readme, CONTAINER_README);
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
        let wide: Vec<u16> = container
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        unsafe {
            windows_sys::Win32::Storage::FileSystem::SetFileAttributesW(
                wide.as_ptr(),
                FILE_ATTRIBUTE_HIDDEN,
            );
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub version: u32,
    /// Max consecutive bsdiff patches before a fresh full snapshot is forced.
    pub delta_chain_max: usize,
    pub tile_size: u32,
    /// Size budget for the capped-raster cache (`.kvc/cache/`). Oldest entries beyond it are
    /// pruned opportunistically after layer streaming (`raster::cache_prune_throttled`).
    /// `#[serde(default)]` so configs from before the knob existed keep deserializing.
    #[serde(default = "default_cache_max_bytes")]
    pub cache_max_bytes: u64,
    /// Opt-in (config.json knob, no UI): store decoded tile *pixels* — which bsdiff across
    /// versions — instead of Krita's opaque LZF payloads. Shrinks heavily-revised layers
    /// 2-10x at the cost of LZF decode on commit and re-encode on restore; off by default
    /// because that CPU lands on the <10s commit/restore paths of low-end devices. Restores
    /// always honor what the manifest says, so toggling never breaks existing history.
    #[serde(default)]
    pub tile_pixel_deltas: bool,
    /// Opt-in: decode working-tree `.kra` diff entries on demand (re-inflating one archive entry
    /// at a time) instead of holding the whole decompressed document in RAM. Trades a little CPU
    /// for bounded peak memory on low-end devices. Off by default — the in-memory path is faster
    /// for interactive diffs. Purely a diff-view knob; never affects stored data.
    #[serde(default)]
    pub low_memory_diff: bool,
}

fn default_cache_max_bytes() -> u64 {
    256 * 1024 * 1024
}

impl Default for Config {
    fn default() -> Self {
        Config {
            version: 2,
            delta_chain_max: 20,
            tile_size: 64,
            cache_max_bytes: default_cache_max_bytes(),
            tile_pixel_deltas: false,
            low_memory_diff: false,
        }
    }
}

/// Committed head of one tracked file — just enough for the scanner to spot changes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackedFile {
    /// blake3 of the whole working-tree file as last committed.
    pub hash: String,
    pub is_kra: bool,
    /// Size + mtime of the file as last committed, so the scanner can skip re-hashing unchanged
    /// files (a big `.kra` is expensive to read+hash). `#[serde(default)]` = 0 for pre-existing
    /// indexes, which never match a real file and so safely force a re-hash. See [`crate::scan`].
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub mtime: u64,
    /// The committed content is a **layer subset** of what's on disk (`stage::stage_kra`), so the
    /// working file is dirty even though `size`/`mtime` match what we recorded. Lets the scanner
    /// answer "modified" from a `stat` instead of re-reading and re-hashing the whole `.kra` —
    /// which the Krita docker's 1.5s status poll would otherwise pay on every tick, forever, on a
    /// document that may be hundreds of MB. `#[serde(default)]` = false for pre-existing indexes,
    /// which is exactly what they meant.
    #[serde(default)]
    pub partial: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Index {
    /// relative path (forward-slashed) -> committed head
    pub files: BTreeMap<String, TrackedFile>,
}

/// One stored version of a delta stream. A stream is any byte sequence we version
/// (a .kra manifest, a single archive entry, or a single tile).
///
/// The object file's name is fully derivable from `hash` + `base` ([`Version::object_name`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Version {
    /// blake3 of the *reconstructed* bytes — also the object file's base name.
    pub hash: String,
    /// hash of the version this patch applies onto; `None` for a full snapshot.
    pub base: Option<String>,
    /// patches back to the nearest full snapshot (0 = full).
    pub chain_len: usize,
}

impl Version {
    /// The content-addressed object file holding this version's payload:
    /// `<hash>.full` (zstd snapshot) or `<hash>.<base>.patch` (bsdiff against `base`).
    pub fn object_name(&self) -> String {
        match &self.base {
            None => format!("{}.full", self.hash),
            Some(b) => format!("{}.{b}.patch", self.hash),
        }
    }
}

/// streamKey -> ordered versions (head = last). The serialized form of one chain shard.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Chains(pub BTreeMap<String, Vec<Version>>);

/// The document bucket for a stream key: its relpath. Every key embeds it (`kra:{rel}:manifest`,
/// `kra:{rel}:entry:{name}`, `kra:{rel}:tile:{entry}:{x},{y}`). This is the shard the manifest and
/// the small entries live in — and, in a store written before tiles got shards of their own, every
/// tile too, which is why a lookup always faults it in first (see [`ChainStore::shard_for`]). The
/// parse is forgiving: a pathological relpath containing a marker still maps *consistently*.
fn doc_shard_of(key: &str) -> &str {
    if let Some(rest) = key.strip_prefix("kra:") {
        for marker in [":tile:", ":entry:"] {
            if let Some(pos) = rest.find(marker) {
                return &rest[..pos];
            }
        }
        if let Some(pre) = rest.strip_suffix(":manifest") {
            return pre;
        }
        return rest;
    }
    key
}

/// Shard identity for a stream key: one shard per tiled archive entry (a layer's tiles, or the
/// composite's blocks), everything else in the document bucket. A store tracks one document, so
/// per-document sharding meant one shard holding every version of every tile — rewritten whole on
/// each commit and decoded whole by the first lookup of every command, however little either
/// touched. Per entry, a commit that edits two layers rewrites those two shards and the small
/// document one.
fn shard_of(key: &str) -> &str {
    match key
        .strip_prefix("kra:")
        .and_then(|rest| rest.find(":tile:"))
    {
        // `kra:{rel}:tile:{entry}` — the key minus its `:{x},{y}` tail.
        Some(_) => key.rsplit_once(':').map_or(key, |(entry, _)| entry),
        None => doc_shard_of(key),
    }
}

fn shard_file(dir: &Path, shard: &str) -> PathBuf {
    dir.join(format!(
        "{}.bin",
        &blake3::hash(shard.as_bytes()).to_hex()[..16]
    ))
}

/// Sharded delta chains, loaded lazily — one shard per tiled entry plus one for the rest of the
/// document (see [`shard_of`]), so reading or committing costs what the command touches rather than
/// the document's whole tile history.
///
/// A store written before per-entry sharding has everything in the document shard. It is split
/// in memory the first time that shard loads, and the split persists with the next save, which
/// writes the tile shards before the shrunken document shard: until that last write lands, the
/// document shard still holds every key, so a crash anywhere in between only means splitting again.
///
/// Interior mutability (`RwLock`) lets read paths fault shards in from behind `&Repo` (rayon
/// `par_iter` reconstructs included); pushes come only from the serial commit folds.
pub struct ChainStore {
    dir: PathBuf,
    /// Loaded shards by shard name. `Arc` so readers can hold a shard without keeping the map
    /// locked.
    shards: RwLock<HashMap<String, Arc<Chains>>>,
    dirty: Mutex<HashSet<String>>,
    /// Document shards that had tile keys split out of them. [`ChainStore::flush`] writes these
    /// after every other dirty shard — see the type's doc for why the order matters.
    split: Mutex<HashSet<String>>,
    /// Shard files that exist but couldn't be read, noted as they fault in — `true` when the
    /// bytes read fine and didn't decode. Reads carry on with an empty shard so the rest of the
    /// store stays viewable; [`ChainStore::set_aside_unreadable`] stops a save from writing that
    /// empty shard over one.
    unreadable: Mutex<Vec<(PathBuf, bool)>>,
}

impl ChainStore {
    fn empty(dir: PathBuf) -> ChainStore {
        ChainStore {
            dir,
            shards: RwLock::new(HashMap::new()),
            dirty: Mutex::new(HashSet::new()),
            split: Mutex::new(HashSet::new()),
            unreadable: Mutex::new(Vec::new()),
        }
    }

    /// The shard holding `key`, with its document shard faulted in first: in a store from before
    /// per-entry sharding the document shard is where `key` actually is, and loading it is what
    /// splits it out.
    fn shard_for(&self, key: &str) -> Arc<Chains> {
        let (name, doc) = (shard_of(key), doc_shard_of(key));
        if name != doc {
            self.load(doc);
        }
        self.load(name)
    }

    /// Read one shard file. `Err(true)` = there but undecodable, `Err(false)` = unreadable;
    /// a missing file is an empty shard.
    fn read_shard(path: &Path) -> std::result::Result<Chains, bool> {
        match std::fs::read(path) {
            Ok(raw) => decode_chains(&raw).ok_or(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Chains::default()),
            Err(_) => Err(false),
        }
    }

    /// The loaded shard for `name`, faulting it in from disk (missing file = empty shard,
    /// negative-cached so repeat misses don't re-stat). A file that's there but won't read also
    /// comes back empty, and is noted in `unreadable` so no save writes over it. Keys that belong
    /// in another shard — a document shard from before per-entry sharding — are moved there.
    fn load(&self, name: &str) -> Arc<Chains> {
        if let Some(s) = self.shards.read().unwrap().get(name) {
            return s.clone();
        }
        let mut w = self.shards.write().unwrap();
        if let Some(s) = w.get(name) {
            return s.clone();
        }
        let path = shard_file(&self.dir, name);
        let mut loaded = Self::read_shard(&path).unwrap_or_else(|undecodable| {
            self.unreadable
                .lock()
                .unwrap()
                .push((path.clone(), undecodable));
            Chains::default()
        });
        if loaded.0.keys().any(|k| shard_of(k) != name) {
            let (own, foreign): (BTreeMap<_, _>, BTreeMap<_, _>) = std::mem::take(&mut loaded.0)
                .into_iter()
                .partition(|(k, _)| shard_of(k) == name);
            loaded.0 = own;
            let mut dirty = self.dirty.lock().unwrap();
            for (key, versions) in foreign {
                let target = shard_of(&key).to_string();
                let shard = w.entry(target.clone()).or_insert_with(|| {
                    let p = shard_file(&self.dir, &target);
                    Arc::new(Self::read_shard(&p).unwrap_or_else(|undecodable| {
                        self.unreadable.lock().unwrap().push((p, undecodable));
                        Chains::default()
                    }))
                });
                // A copy already in its own shard was written by an earlier, interrupted save
                // of this same split, and is never older than the one left behind here.
                Arc::make_mut(shard).0.entry(key).or_insert(versions);
                dirty.insert(target);
            }
            dirty.insert(name.to_string());
            self.split.lock().unwrap().insert(name.to_string());
        }
        let arc = Arc::new(loaded);
        w.insert(name.to_string(), arc.clone());
        arc
    }

    /// Refuse a save while any shard failed to read — before anything is written, since the save
    /// would replace it with the empty shard it read as, and a shard holds the chain records of
    /// every version of what it covers. A shard whose bytes didn't decode is renamed aside
    /// (`<name>.bin.corrupt-<time>`), which lets the next attempt go ahead on a fresh shard
    /// without destroying what a repair could salvage; the check keeps naming the set-aside file.
    /// One that couldn't be read at all (a scanner holding it, say) is left alone and just
    /// refuses. Every later save through this `Repo` refuses too: its view of the shard is empty.
    fn set_aside_unreadable(&self) -> Result<()> {
        let faults = self.unreadable.lock().unwrap().clone();
        if faults.is_empty() {
            return Ok(());
        }
        let mut detail = Vec::new();
        for (path, undecodable) in &faults {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if *undecodable && path.is_file() {
                let aside_name = format!("{name}.corrupt-{}", now_iso_filesafe());
                rename_retrying(path, &path.with_file_name(&aside_name))
                    .map_err(|e| io_at(path, e))?;
                detail.push(format!(
                    "chains/{name} couldn't be decoded, so it was kept aside as {aside_name}; \
                     saving again starts a fresh one"
                ));
            } else {
                detail.push(format!("chains/{name} couldn't be read"));
            }
        }
        Err(KvcError::DamagedHistory(detail.join("; ")))
    }

    /// The version chain for `key` (cloned — chains are short by design: at most
    /// `delta_chain_max`+1 per snapshot run).
    pub fn chain(&self, key: &str) -> Option<Vec<Version>> {
        self.shard_for(key).0.get(key).cloned()
    }

    /// The object name for one `(key, hash)` pair, without cloning the key's chain.
    ///
    /// [`chain`] is the right shape when a caller wants the whole chain, and its clone is cheap
    /// *per call*. This exists because the storage report doesn't: it resolves one object name
    /// per **tile** per commit, and a real 110 MB painting holds ~45k tiles, so the per-call
    /// clone is multiplied by (tiles x commits) and becomes the dominant cost of the whole
    /// report. Same lookup, no allocation of the surrounding `Vec`.
    pub fn object_name_of(&self, key: &str, hash: &str) -> Option<String> {
        self.shard_for(key)
            .0
            .get(key)?
            .iter()
            .find(|v| v.hash == hash)
            .map(|v| v.object_name())
    }

    /// Append a version to `key`'s chain and mark its shard dirty.
    pub(crate) fn push(&self, key: String, version: Version) {
        let name = shard_of(&key).to_string();
        self.shard_for(&key); // fault in before mutating, so we never clobber an unread shard
        let mut w = self.shards.write().unwrap();
        let entry = w.get_mut(&name).expect("shard just loaded");
        Arc::make_mut(entry).0.entry(key).or_default().push(version);
        self.dirty.lock().unwrap().insert(name);
    }

    /// Write every dirty shard (atomic each) — a document shard that had tiles split out of it
    /// last, once the shards they went to are safely on disk. A failed write leaves its dirty mark
    /// in place for the next save.
    fn flush(&mut self) -> Result<()> {
        let split = self.split.lock().unwrap().clone();
        let mut dirty: Vec<String> = self.dirty.lock().unwrap().iter().cloned().collect();
        if dirty.is_empty() {
            return Ok(());
        }
        dirty.sort_by_key(|name| split.contains(name));
        std::fs::create_dir_all(&self.dir).map_err(|e| io_at(&self.dir, e))?;
        {
            let shards = self.shards.read().unwrap();
            for name in &dirty {
                if let Some(chains) = shards.get(name) {
                    write_chains_file(&shard_file(&self.dir, name), chains)?;
                }
            }
        }
        self.dirty.lock().unwrap().clear();
        self.split.lock().unwrap().clear();
        Ok(())
    }

    fn has_dirty(&self) -> bool {
        !self.dirty.lock().unwrap().is_empty()
    }

    /// Replace the whole store with `new` (GC sweep): partition into shards, write every
    /// non-empty shard — document shards last, for the reason [`ChainStore::flush`] gives — then
    /// delete shard files whose bucket no longer exists. Writes happen before deletes and each
    /// write is atomic, so there is no window without valid chains.
    pub fn rewrite_all(&mut self, new: Chains) -> Result<()> {
        let mut map: HashMap<String, Chains> = HashMap::new();
        for (key, versions) in new.0 {
            map.entry(shard_of(&key).to_string())
                .or_default()
                .0
                .insert(key, versions);
        }
        std::fs::create_dir_all(&self.dir).map_err(|e| io_at(&self.dir, e))?;
        let mut order: Vec<(&String, &Chains)> = map.iter().collect();
        order.sort_by_key(|(name, chains)| {
            chains
                .0
                .keys()
                .next()
                .is_some_and(|k| doc_shard_of(k) == name.as_str())
        });
        let keep: HashSet<PathBuf> = order
            .into_iter()
            .map(|(name, chains)| {
                let path = shard_file(&self.dir, name);
                write_chains_file(&path, chains).map(|_| path)
            })
            .collect::<Result<_>>()?;
        if let Ok(rd) = std::fs::read_dir(&self.dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().is_some_and(|x| x == "bin") && !keep.contains(&p) {
                    let _ = std::fs::remove_file(&p);
                }
            }
        }
        // In-memory state now mirrors disk.
        self.shards = RwLock::new(map.into_iter().map(|(k, v)| (k, Arc::new(v))).collect());
        self.dirty = Mutex::new(HashSet::new());
        self.split = Mutex::new(HashSet::new());
        Ok(())
    }

    /// Every chain across every shard, on-disk and in-memory merged (in-memory wins — it is
    /// never older). Loads the whole store: tests, GC and the check only, never a hot path.
    ///
    /// A key can sit in two files: its own shard, and the document shard of a store whose split
    /// was interrupted before the document shard was rewritten. Its own shard's copy wins, as in
    /// [`ChainStore::load`].
    pub fn export_all(&self) -> Chains {
        let mut all = Chains::default();
        if let Ok(rd) = std::fs::read_dir(&self.dir) {
            for e in rd.flatten() {
                let path = e.path();
                if path.extension().is_none_or(|x| x != "bin") {
                    continue;
                }
                let Some(c) = read_chains_file(&path) else {
                    continue;
                };
                for (key, versions) in c.0 {
                    if shard_file(&self.dir, shard_of(&key)) == path {
                        all.0.insert(key, versions);
                    } else {
                        all.0.entry(key).or_insert(versions);
                    }
                }
            }
        }
        for shard in self.shards.read().unwrap().values() {
            all.0
                .extend(shard.0.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
        all
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommittedFile {
    pub path: String,
    /// 'A' added, 'M' modified, 'D' deleted.
    pub status: String,
    /// For .kra: stream hash of its manifest. For generic files: stream hash of the blob.
    /// `None` for deletions.
    pub content: Option<String>,
    pub is_kra: bool,
    /// blake3 of the whole working-tree file as it sat on disk when this commit recorded it —
    /// lets `undo` rewind the index without reconstructing the file just to hash it.
    /// `None` on records from before the field existed (undo then falls back to reconstructing)
    /// and on deletions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_hash: Option<String>,
    /// Original (uncompressed) byte size of the working file as it sat on disk when this commit
    /// recorded it — feeds the "storage saved vs full-copy-per-version" report. 0 for deletions
    /// and for records from before the field existed (`#[serde(default)]`).
    #[serde(default)]
    pub original_size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Commit {
    pub id: String,
    pub hash: String,
    pub message: String,
    pub author: String,
    pub timestamp: String,
    pub parents: Vec<String>,
    /// Branch the commit was made on. Cosmetic (frontend labels/colors) — never used for
    /// correctness. Pre-branching commits deserialize as `""`.
    #[serde(default)]
    pub branch: String,
    /// Invariant: exactly the diff of this commit's tree against its **first parent's** tree
    /// (merge commits record every path where the merged result differs from the first parent).
    /// `tree_at_commit` relies on this to fold along the first-parent chain only.
    pub files: Vec<CommittedFile>,
    /// Id of the commit a rollback restored, for the history graph's link line. `None` otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restored_from: Option<String>,
    /// Bytes of new objects this version added to the store, counted as they were written — the
    /// storage report's per-version cost. `None` on versions from before it was recorded, which
    /// the report still has to rebuild by replaying their manifests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stored_bytes: Option<u64>,
}

/// Work set aside off to the side of history — see [`crate::stash`].
///
/// Deliberately *not* a `Commit` in `commits.log`: a stash is not history. Keeping it out means
/// it can't show up as a spurious version row in the storage report, and can't block `undo` by
/// looking like a child of the tip. `files` is still `Vec<CommittedFile>` because the content is
/// stored through the very same relpath-keyed streams a commit uses — so a stashed `.kra`'s tiles
/// dedup against committed history for free, and [`crate::gc`] can mark them with the same walk.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stash {
    pub id: String,
    /// User's label for the stash. May be empty — the UI falls back to the file list.
    pub label: String,
    pub author: String,
    pub timestamp: String,
    /// Branch this was set aside on. Display only — a stash can be brought back onto any branch,
    /// including after this one is deleted, because `files` carries its own content hashes and
    /// nothing here is ever looked up.
    pub branch: String,
    pub files: Vec<CommittedFile>,
}

/// The shelf: every stash in the repo, oldest first (so `last()` is "the latest").
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stashes {
    pub stashes: Vec<Stash>,
}

/// Local branches: name -> tip commit id (`""` = branch has no commits yet).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Branches {
    pub current: String,
    pub branches: BTreeMap<String, String>,
    /// Bumped on every write (see [`Repo::save`]/[`Repo::save_branches`]) — lets a long read spot
    /// a write that landed mid-read and discard/retry rather than report a torn snapshot.
    /// `#[serde(default)]` so a pre-existing `branches.json` without this field just starts at 0.
    #[serde(default)]
    pub generation: u64,
}

impl Default for Branches {
    fn default() -> Self {
        let mut branches = BTreeMap::new();
        branches.insert("main".to_string(), String::new());
        Branches {
            current: "main".to_string(),
            branches,
            generation: 0,
        }
    }
}

impl Branches {
    /// Tip commit id of the current branch, `None` if the branch has no commits yet.
    pub fn tip(&self) -> Option<&str> {
        self.branches
            .get(&self.current)
            .map(String::as_str)
            .filter(|t| !t.is_empty())
    }

    pub fn tip_of(&self, name: &str) -> Option<&str> {
        self.branches
            .get(name)
            .map(String::as_str)
            .filter(|t| !t.is_empty())
    }

    pub fn set_tip(&mut self, id: &str) {
        self.branches.insert(self.current.clone(), id.to_string());
    }
}

/// Loaded repository state. Mutated in-memory then flushed with [`Repo::save`].
pub struct Repo {
    /// The **working tree**: the folder holding the tracked document. Everything the engine
    /// writes back to disk is `safe_join`ed onto this.
    pub root: PathBuf,
    /// The **store**: where this document's history lives — normally `<root>/.kvc/<slug>/`,
    /// but anywhere at all if the user set a custom store root. Split from `root` because the
    /// two stopped being the same folder when one art folder started holding several
    /// independent histories.
    pub store: PathBuf,
    /// Which document this store tracks. Held separately from `index`, which only knows about
    /// files that have been *committed* — a freshly-initialised store has an empty index but a
    /// perfectly well-defined document.
    pub doc: DocMeta,
    pub config: Config,
    pub index: Index,
    /// Delta chains, sharded and loaded lazily — see [`ChainStore`].
    pub chains: ChainStore,
    /// Lazily-indexed object packs (large commits write one pack instead of thousands of
    /// loose files) — see [`crate::delta::Packs`].
    pub(crate) packs: crate::delta::Packs,
    pub commits: Vec<Commit>,
    pub branches: Branches,
    /// Work set aside off to the side of history — see [`crate::stash`]. Also a GC root.
    pub stashes: Stashes,
    /// How many of `commits` are already lines in `commits.log`; `save()` appends the rest.
    commits_persisted: usize,
    /// Force a full log rewrite on the next `save()`: set on a torn last line, and whenever
    /// `commits` was truncated (undo, GC) — see [`Repo::note_commits_truncated`].
    commits_rewrite: bool,
    /// The rewrite *removes* commits, which flips the save order — see [`Repo::save`].
    commits_truncated: bool,
    /// Lines in the middle of `commits.log` that won't decode. The store still opens with every
    /// line that does, for viewing; every write refuses ([`Repo::ensure_writable`]).
    log_damage: Option<String>,
    /// Bytes of new objects this `Repo` has written — what a commit records as its
    /// [`Commit::stored_bytes`]. Reset by the commit before it stores anything.
    pub(crate) added_bytes: u64,
    /// Re-hash every object [`Repo::reconstruct`] rebuilds and refuse a mismatch. In-memory only
    /// and off by default — deliberately **not** a user config knob. Turned on by the operations
    /// that write reconstructed bytes into the working tree (switch, rollback, discard, stash
    /// pop, restore-file), where a silently-wrong byte becomes the artist's file. Left off for
    /// diffs and previews: that's the hot loop the app is tuned around, and a wrong pixel in a
    /// preview is not data loss.
    pub verify_reads: bool,
}

/// Written into a backup zip as `MANIFEST.json` (see [`Repo::export_zip_multi`]) so a recovered
/// archive is self-describing — and so [`Repo::import_zip`] knows what's inside without having
/// to infer it from entry names.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupManifest {
    /// 2 = one folder per artwork under `entries`. v1 (a single artwork at the archive root, no
    /// `entries`) was never read by anything but `verify_zip`, so there is no v1 import path;
    /// the field exists so a future reader can tell, not so this one can branch.
    pub version: u32,
    pub timestamp: String,
    pub app_version: String,
    pub entries: Vec<BackupEntry>,
}

/// One artwork inside a backup archive.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupEntry {
    /// Folder inside the archive holding this artwork and its `.kvc/`.
    pub dir: String,
    /// The document's filename. Baked into `doc.json`, `index.json` keys, chain shard names and
    /// every stream key, so import can never rename it — see [`Repo::import_zip`].
    pub relpath: String,
    /// Absolute directory the artwork lived in when backed up. A *hint* restore offers as the
    /// default destination; nothing inside the store depends on it.
    pub original_dir: String,
    /// Best-effort — a document whose branch state won't load still gets backed up, and still
    /// gets an entry, because the entry is what makes it importable.
    pub branch: String,
    pub tip_commit: String,
}

/// One artwork the caller chose to restore, and where to put it. Artworks the user skipped
/// simply aren't in the list.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportItem {
    /// Matches a [`BackupEntry::dir`] in the archive's manifest.
    pub dir: String,
    /// Folder to write the artwork into. Its **history** does not necessarily go beside it —
    /// see [`Repo::import_zip`].
    pub dest_dir: String,
}

/// What one restored artwork ended up as.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub dir: String,
    /// Absolute path of the restored `.kra`.
    pub path: String,
    pub name: String,
    /// Where this machine decided the history goes — beside the artwork, or under the custom
    /// store root. Surfaced so the UI can show it rather than leaving the user to guess.
    pub store: String,
    /// Findings from the post-import integrity check, `"kind: detail"` each. Non-fatal.
    pub problems: Vec<String>,
    /// Set when this artwork failed outright; `path`/`store` are then best-effort. A failed
    /// restore leaves whatever was already there untouched.
    pub error: Option<String>,
    /// Where the artwork that was already at `path` went — Replace keeps it beside the restore.
    pub replaced_artwork: Option<String>,
    /// Where the history already tracked there went: beside the new store, until a cleanup ages
    /// it out.
    pub replaced_history: Option<String>,
}

impl Repo {
    /// Is this `.kra` already tracked? Addressed by the **document**, not its folder — an art
    /// folder holding a tracked painting says nothing about its neighbours.
    pub fn is_repo(kra_path: &Path) -> bool {
        store_dir_for(kra_path).join("config.json").is_file()
    }

    /// Start tracking one `.kra`, creating its store beside it.
    pub fn init(kra_path: &Path) -> Result<()> {
        if !crate::scan::is_supported(&kra_path.to_string_lossy()) {
            return Err(KvcError::Unsupported(kra_path.to_path_buf()));
        }
        if !kra_path.is_file() {
            return Err(KvcError::NotARepo(kra_path.to_path_buf()));
        }
        let store = store_dir_for(kra_path);
        if store.join("config.json").exists() {
            return Err(KvcError::AlreadyRepo(kra_path.to_path_buf()));
        }
        refuse_missing_store_root(kra_path)?;
        // The only "nesting" guard left: stores are siblings by design, so the folder-level
        // ancestor/descendant walks the folder model needed are gone. A store either already
        // exists for this exact document (above) or it doesn't.
        if let Some(parent) = store.parent() {
            std::fs::create_dir_all(parent).map_err(|e| io_at(parent, e))?;
            // Only the default in-folder container gets hidden + a README; a user-chosen store
            // root is a folder they picked themselves and expect to see.
            if custom_store_root().is_none() {
                dress_container(parent);
            }
        }

        std::fs::create_dir_all(objects_dir(&store)).map_err(|e| io_at(&store, e))?;
        std::fs::create_dir_all(cache_dir(&store)).map_err(|e| io_at(&store, e))?;
        std::fs::create_dir_all(chains_dir(&store)).map_err(|e| io_at(&store, e))?;
        let relpath = doc_relpath(kra_path)?;
        write_json(
            &store.join("doc.json"),
            &DocMeta {
                display_name: relpath.clone(),
                relpath,
                created_at: now_iso(),
            },
        )?;
        write_json(&store.join("config.json"), &Config::default())?;
        write_json(&store.join("index.json"), &Index::default())?;
        write_atomic(&store.join("commits.log"), b"")?;
        write_json(&store.join("branches.json"), &Branches::default())?;
        Ok(())
    }

    /// Stop tracking a document: delete its **store**, preferring the OS Recycle Bin so an
    /// accidental delete stays recoverable from Explorer/Finder. Falls back to a permanent
    /// `remove_dir_all` if the trash move fails (e.g. no trash provider on that filesystem) so
    /// the action never gets stuck — the returned bool tells the caller which happened.
    ///
    /// **Never touches the artwork.** Under the folder model this deleted the project tree, art
    /// files included; a document's store holds only history, and destroying someone's painting
    /// because they stopped versioning it would be indefensible.
    pub fn delete(kra_path: &Path) -> Result<bool> {
        if !Self::is_repo(kra_path) {
            return Err(KvcError::NotARepo(kra_path.to_path_buf()));
        }
        let store = store_dir_for(kra_path);
        let trashed = if trash::delete(&store).is_ok() {
            true
        } else {
            std::fs::remove_dir_all(&store).map_err(|e| io_at(&store, e))?;
            false
        };
        // Take the container with it once the last store in it is gone, so an art folder that
        // stops being versioned doesn't keep a hidden folder holding nothing but a README.
        if let Some(container) = store.parent() {
            if container.file_name().map(|n| n == KVC_DIR).unwrap_or(false) {
                let empty = std::fs::read_dir(container).map(|mut d| {
                    d.all(|e| e.map(|e| e.file_name() == "README.txt").unwrap_or(false))
                });
                if empty.unwrap_or(false) {
                    let _ = std::fs::remove_dir_all(container);
                }
            }
        }
        Ok(trashed)
    }

    /// Zip the given documents — each `.kra` **plus** its store — into one archive at `dest`: a
    /// manual, on-demand backup for the user to move to their own cloud storage or an external
    /// drive. It's the only thing that helps against loss the app can't intervene in (the folder
    /// deleted outside the app, disk failure, external corruption).
    ///
    /// Layout, one folder per artwork:
    ///
    /// ```text
    /// MANIFEST.json
    /// <dir>/<name>.kra
    /// <dir>/.kvc/<slug>/…
    /// ```
    ///
    /// Each `<dir>/` is exactly the single-document on-disk shape, so plain extraction still
    /// works — unzip and any one subfolder is a tracked document. Prefer [`Repo::import_zip`]
    /// anyway: only it re-derives where *this machine* keeps history.
    ///
    /// Independent artworks, so one failing (permission denied, store gone, busy) collects into
    /// the returned list instead of aborting the rest. The finished archive is reopened and
    /// checked (entry count + manifest readability) before success is reported — an unverified
    /// backup is not actually a backup.
    ///
    /// Written as `<dest>.partial` and renamed over `dest` only once verified. The default name is
    /// one per day, so a second backup the same day replaces the first, and a run that fails
    /// partway must leave that first one — the only good copy — where it was.
    pub fn export_zip_multi(kra_paths: &[PathBuf], dest: &Path) -> Result<Vec<String>> {
        let mut partial = dest.as_os_str().to_os_string();
        partial.push(".partial");
        let partial = PathBuf::from(partial);
        let failed = match write_backup(kra_paths, &partial) {
            Ok(failed) => failed,
            Err(e) => {
                let _ = std::fs::remove_file(&partial);
                return Err(e);
            }
        };
        if let Err(e) = rename_retrying(&partial, dest) {
            let _ = std::fs::remove_file(&partial);
            return Err(io_at(dest, e));
        }
        Ok(failed)
    }

    /// Read just the `MANIFEST.json` out of a backup archive — what's in it, and where each
    /// artwork came from. Cheap and read-only, so the restore UI can list an archive's contents
    /// and check destinations before committing to anything.
    pub fn read_backup_manifest(archive: &Path) -> Result<BackupManifest> {
        let file = std::fs::File::open(archive).map_err(|e| io_at(archive, e))?;
        let mut za = zip::ZipArchive::new(file).map_err(zip_err)?;
        let mf = za.by_name("MANIFEST.json").map_err(zip_err)?;
        let bytes = read_entry_capped(mf)?;
        serde_json::from_slice(&bytes)
            .map_err(|e| KvcError::CorruptZip(format!("unreadable backup manifest: {e}")))
    }

    /// The versions inside one artwork in a backup archive, **newest first** — read straight out
    /// of the zip, nothing extracted.
    ///
    /// This is what lets restore answer the only question a clash actually poses: is the backup
    /// ahead of the history already on this machine, or behind it? Replace swaps the on-disk
    /// history out (it's kept aside only until a cleanup ages it out), so the artist needs to see
    /// both lists before choosing.
    ///
    /// Scoped to the archived branch tip via [`crate::commit::ancestors`], matching
    /// `list_commits`' default scope so the two sides of the comparison are counted the same way.
    pub fn backup_versions(archive: &Path, dir: &str) -> Result<Vec<VersionRow>> {
        let manifest = Self::read_backup_manifest(archive)?;
        let entry = manifest
            .entries
            .iter()
            .find(|e| e.dir == dir)
            .ok_or_else(|| KvcError::CorruptZip(format!("{dir} is not in this backup")))?;

        let file = std::fs::File::open(archive).map_err(|e| io_at(archive, e))?;
        let mut za = zip::ZipArchive::new(file).map_err(zip_err)?;
        // `<dir>/.kvc/<slug>/commits.log` — the slug is payload (same strip as `import_one`), so
        // find the log by shape rather than by reconstructing this machine's slug. Names come
        // from the central directory already in memory; `by_index` would seek to every entry's
        // local header just to read its name.
        let prefix = format!("{dir}/{KVC_DIR}/");
        let idx = (0..za.len()).find(|&i| {
            za.name_for_index(i)
                .and_then(|n| n.strip_prefix(&prefix))
                .and_then(|after| after.split_once('/'))
                .is_some_and(|(_slug, rel)| rel == "commits.log")
        });
        let Some(idx) = idx else {
            return Ok(Vec::new()); // an artwork backed up before its first commit
        };
        let bytes = read_entry_capped(za.by_index(idx).map_err(zip_err)?)?;
        let commits = parse_commit_log(&bytes).commits;

        // `tip_commit` is best-effort at export time (a document whose branches wouldn't load
        // still gets backed up); with no tip there is nothing to walk, so show the whole log.
        let rows: Vec<&Commit> = if entry.tip_commit.is_empty() {
            commits.iter().collect()
        } else {
            let reach = crate::commit::ancestors(&commits, &entry.tip_commit);
            commits.iter().filter(|c| reach.contains(&c.id)).collect()
        };
        Ok(rows.into_iter().rev().map(VersionRow::from).collect())
    }

    /// Restore artworks out of a backup archive. Failures are per-artwork (reported in the
    /// result) rather than fatal, for the same reason export collects them: these are
    /// independent documents.
    ///
    /// **The archive's `.kvc/<slug>/` path is payload, not a destination.** Import extracts the
    /// `.kra`, then asks *this machine* where that document's history belongs via
    /// [`store_dir_for`] — so a backup made on a machine with no custom store root restores onto
    /// one that has a store root by putting the history under that root (and creating no `.kvc/`
    /// beside the artwork at all), and vice versa. That isn't a preference this flow offers: it
    /// is the app-global setting, and matching what `Repo::init` would have done is mandatory —
    /// history written anywhere else is history every later `open`/`is_repo`/scan/commit and the
    /// `kvc` CLI would look straight past.
    ///
    /// Artworks are never renamed on the way in: the filename is baked into `doc.json`,
    /// `index.json` keys, chain shard filenames, `Commit.files[].path` and every
    /// `kra:{relpath}:…` stream key.
    // ponytail: so a name clash is Replace-or-skip only. Renaming needs a relpath rewrite across
    // all five of those; add it if artists actually hit the clash.
    pub fn import_zip(archive: &Path, items: &[ImportItem]) -> Result<Vec<ImportResult>> {
        let manifest = Self::read_backup_manifest(archive)?;
        let file = std::fs::File::open(archive).map_err(|e| io_at(archive, e))?;
        let mut za = zip::ZipArchive::new(file).map_err(zip_err)?;
        let mut out = Vec::new();
        for item in items {
            let Some(entry) = manifest.entries.iter().find(|e| e.dir == item.dir) else {
                out.push(failed_import(
                    &item.dir,
                    "",
                    "",
                    "not in this backup".to_string(),
                ));
                continue;
            };
            let dest_dir = Path::new(&item.dest_dir);
            match import_one(&mut za, entry, dest_dir) {
                Ok(r) => out.push(r),
                Err(e) => {
                    let path = safe_join(dest_dir, &entry.relpath).unwrap_or_default();
                    let store = store_dir_for(&path);
                    out.push(failed_import(
                        &entry.dir,
                        &path.to_string_lossy(),
                        &store.to_string_lossy(),
                        e.to_string(),
                    ));
                }
            }
        }
        Ok(out)
    }

    /// Resolve a document to `(working-tree root, store dir)`, or say precisely why not.
    ///
    /// The distinction between the two failures is load-bearing: `NotARepo` means "never
    /// versioned", and the UI answers it by offering to start tracking — which, for a document
    /// whose history is merely sitting on a drive that isn't plugged in right now, would create
    /// an empty store and orphan every version the artist ever saved. So an unreachable store
    /// root is its own error and must never degrade to `NotARepo`.
    fn locate(kra_path: &Path) -> Result<(PathBuf, PathBuf)> {
        let store = store_dir_for(kra_path);
        if store.join("config.json").is_file() {
            return Ok((doc_root(kra_path), store));
        }
        Err(locate_failure(kra_path, custom_store_root().as_deref()))
    }

    /// Validate the store and load its state. Chains and packs load lazily, on first use.
    pub fn open(kra_path: &Path) -> Result<Repo> {
        let (root, kvc) = Self::locate(kra_path)?;
        let history = load_history(&kvc)?;
        Self::assemble(root, kvc, history)
    }

    /// [`Repo::open`] for the paths that read history but never rebuild or store content (the
    /// log, branches, the shelf). Since chains load lazily either way, the two are the same today;
    /// the name keeps that promise visible at the call sites.
    pub fn open_light(kra_path: &Path) -> Result<Repo> {
        Self::open(kra_path)
    }

    /// [`Repo::open`] without reading `commits.log`, for the status-type reads that only need the
    /// document, its index, the branches and the shelf: `kvc status` on the Krita docker's poll,
    /// the desktop app's scan and branch list. The log is the one part of a store that grows with
    /// every version, and those callers never look at it.
    ///
    /// Invariant: read-only. `commits` is empty and the damage check never ran, so every write
    /// refuses ([`Repo::ensure_writable`]) rather than rewrite the log from nothing.
    pub fn open_without_log(kra_path: &Path) -> Result<Repo> {
        let (root, kvc) = Self::locate(kra_path)?;
        let history = LoadedHistory {
            commits: Vec::new(),
            branches: read_json_with_backup(&kvc.join("branches.json"))?,
            persisted: 0,
            rewrite: false,
            damage: Some("opened without its history, for reading only".into()),
        };
        Self::assemble(root, kvc, history)
    }

    fn assemble(root: PathBuf, kvc: PathBuf, history: LoadedHistory) -> Result<Repo> {
        Ok(Repo {
            root,
            doc: read_doc_meta(&kvc)?,
            config: read_json(&kvc.join("config.json"))?,
            index: read_json_with_backup(&kvc.join("index.json"))?,
            chains: ChainStore::empty(chains_dir(&kvc)),
            packs: crate::delta::Packs::default(),
            commits: history.commits,
            branches: history.branches,
            stashes: read_stashes(&kvc)?,
            commits_persisted: history.persisted,
            commits_rewrite: history.rewrite,
            commits_truncated: false,
            log_damage: history.damage,
            added_bytes: 0,
            verify_reads: false,
            store: kvc,
        })
    }

    pub fn objects_dir(&self) -> PathBuf {
        objects_dir(&self.store)
    }

    pub fn cache_dir(&self) -> PathBuf {
        cache_dir(&self.store)
    }

    /// Every path this store versions. Exactly one — the tracked document — but returned as a
    /// slice so the commit/scan/diff paths that already loop over "the files in this repo" keep
    /// their shape rather than being rewritten around a singular.
    pub fn tracked_paths(&self) -> Vec<String> {
        vec![self.doc.relpath.clone()]
    }

    /// Mark the in-memory commit list as truncated (undo popped a commit, GC dropped
    /// unreachable ones) so the next [`Repo::save`] rewrites `commits.log` instead of appending.
    pub fn note_commits_truncated(&mut self) {
        self.commits_rewrite = true;
        self.commits_truncated = true;
    }

    /// Refuse to write while the history has a hole in it (damaged lines in the middle of
    /// `commits.log`). Viewing still works; any write would make the loss permanent — the log
    /// rewritten from the shortened list, or new versions built on a gap. Checked by the
    /// operations that touch the working tree before they save, and by every save itself.
    pub fn ensure_writable(&self) -> Result<()> {
        match &self.log_damage {
            Some(detail) => Err(KvcError::DamagedHistory(detail.clone())),
            None => Ok(()),
        }
    }

    /// Flush mutated state atomically. Chains rewrite only their dirty shards — the entries this
    /// commit actually touched; switch/merge/undo mutate only index/commits/branches and skip
    /// chains entirely ([`ChainStore::has_dirty`]). The commit log normally takes one O(1) append
    /// per new commit — never a rewrite that grows with total history.
    ///
    /// Write order matters, and it depends on which way the log moves. When commits are *added*,
    /// `branches.json` (the tips) goes after the log, so a torn append is always an unreachable
    /// orphan record, never a dangling tip. When commits are *removed* (undo), the tip has to
    /// move off the undone commit before the log drops it — the same rule seen from the other
    /// side — so `branches.json` goes first, naming a commit that both the old and the new log
    /// hold. `stashes.json` goes last either way: a stash record must never outlive the chain
    /// content it points at.
    pub fn save(&mut self) -> Result<()> {
        self.ensure_writable()?;
        let kvc = self.store.clone();
        // Before anything is written: the next write to an unreadable shard would replace it with
        // the empty shard it read as, destroying what a repair could salvage.
        self.chains.set_aside_unreadable()?;
        write_json_with_backup(&kvc.join("index.json"), &self.index)?;
        if self.chains.has_dirty() {
            self.chains.flush()?;
        }
        if self.commits_truncated {
            self.write_branches()?;
            self.flush_commits(&kvc)?;
        } else {
            self.flush_commits(&kvc)?;
            self.write_branches()?;
        }
        write_json_with_backup(&kvc.join("stashes.json"), &self.stashes)?;
        Ok(())
    }

    fn write_branches(&mut self) -> Result<()> {
        self.branches.generation = self.branches.generation.wrapping_add(1);
        write_json_with_backup(&self.store.join("branches.json"), &self.branches)
    }

    /// Persist `config` alone — for settings edits, which never touch index/chains/commits/
    /// branches and shouldn't pay for `save()`'s full flush.
    pub fn save_config(&mut self) -> Result<()> {
        write_json(&self.store.join("config.json"), &self.config)
    }

    fn flush_commits(&mut self, kvc: &Path) -> Result<()> {
        let log = kvc.join("commits.log");
        if self.commits_rewrite {
            // A rewrite is the one write that can drop records, so keep the log it replaces.
            // Timestamped rather than one rolling `.bak`, so two undos don't overwrite the copy
            // from before the first; a cleanup ages them out with the trash (`gc::prune_aged`).
            // Within one second the first copy wins: it's the older, more valuable one.
            let copy = kvc.join(format!("commits.log.{}.bak", now_iso_filesafe()));
            if log.is_file() && !copy.exists() {
                std::fs::copy(&log, &copy).map_err(|e| io_at(&copy, e))?;
            }
            write_atomic(&log, &commit_lines(&self.commits)?)?;
            self.commits_rewrite = false;
            self.commits_truncated = false;
            self.commits_persisted = self.commits.len();
        } else if self.commits.len() > self.commits_persisted {
            use std::io::Write;
            let lines = commit_lines(&self.commits[self.commits_persisted..])?;
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log)
                .map_err(|e| io_at(&log, e))?;
            f.write_all(&lines).map_err(|e| io_at(&log, e))?;
            // Fsync the append, not just the state files: `save()`'s "tips go last" ordering is
            // only an ordering if this line is on the platter before `branches.json` names it.
            // Unsynced, a power cut could land the fsynced tip over a log append still in cache —
            // a dangling tip, the one failure this whole ordering exists to prevent. One fsync of
            // a few hundred bytes per commit.
            f.sync_all().map_err(|e| io_at(&log, e))?;
            self.commits_persisted = self.commits.len();
        }
        Ok(())
    }

    /// Flush only `branches.json`: a branch edit touches nothing else, so it shouldn't pay for
    /// [`Repo::save`]'s index, chains and log.
    pub fn save_branches(&mut self) -> Result<()> {
        self.ensure_writable()?;
        self.write_branches()
    }

    /// Flush only `stashes.json` — same reasoning as [`Repo::save_branches`]: dropping a stash
    /// touches nothing else.
    pub fn save_stashes(&self) -> Result<()> {
        self.ensure_writable()?;
        write_json_with_backup(&self.store.join("stashes.json"), &self.stashes)
    }
}

/// Atomic **and durable** write: bytes to a temp file in the same dir, fsync, then rename over
/// the target (Rust's `fs::rename` replaces the destination on Windows and POSIX). Without the
/// fsync the rename can land while the temp's contents are still in the page cache, so a power
/// cut yields a zero-length `branches.json` — atomic against a process crash, not against the
/// machine dying.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    sync_write(&tmp, bytes)?;
    rename_retrying(&tmp, path).map_err(|e| io_at(path, e))?;
    sync_parent_dir(path);
    Ok(())
}

/// Atomic + durable write for a **working-tree** file (the artist's actual art). Same shape as
/// [`write_atomic`], with the temp suffix *appended* rather than substituted: `with_extension`
/// would collapse `a.kra` and `a.gpl` onto one temp path, and `.kvctmp` can never match
/// `scan::is_supported`, so the scanner ignores a crash leftover instead of tracking it.
pub fn write_file_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = kvctmp_of(path);
    if let Err(e) = sync_write(&tmp, bytes) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) = rename_retrying(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(io_at(path, e));
    }
    sync_parent_dir(path);
    Ok(())
}

/// [`write_file_atomic`] for content a writer produces rather than a buffer holds: `write` fills
/// the temp file, which is fsynced, hashed and renamed into place, and the blake3 of what landed is
/// returned for the index. The restore paths rebuild a whole document this way instead of holding
/// it in memory first. The hash is read back rather than taken on the way out, because the zip
/// writer seeks back to patch each entry's header.
pub fn write_file_atomic_with(
    path: &Path,
    write: impl FnOnce(&mut std::fs::File) -> Result<()>,
) -> Result<String> {
    let tmp = kvctmp_of(path);
    let written = (|| {
        let mut f = std::fs::File::create(&tmp).map_err(|e| io_at(&tmp, e))?;
        write(&mut f)?;
        f.sync_all().map_err(|e| io_at(&tmp, e))?;
        drop(f);
        hash_file(&tmp)
    })();
    let hash = match written {
        Ok(hash) => hash,
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
    };
    if let Err(e) = rename_retrying(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(io_at(path, e));
    }
    sync_parent_dir(path);
    Ok(hash)
}

/// [`hash_bytes`] of a file's contents, read 16 MB at a time — each piece hashed in parallel on a
/// worker of the budgeted pool, like a whole buffer would be — so a document is never held whole.
pub fn hash_file(path: &Path) -> Result<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).map_err(|e| io_at(path, e))?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; 16 << 20];
    let parallel = rayon::current_thread_index().is_some();
    loop {
        let mut filled = 0;
        while filled < buf.len() {
            match f.read(&mut buf[filled..]).map_err(|e| io_at(path, e))? {
                0 => break,
                n => filled += n,
            }
        }
        if parallel {
            hasher.update_rayon(&buf[..filled]);
        } else {
            hasher.update(&buf[..filled]);
        }
        if filled < buf.len() {
            break;
        }
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn kvctmp_of(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".kvctmp");
    PathBuf::from(name)
}

/// Delete `path`'s `.kvctmp` if it's a crash leftover. [`write_file_atomic`] writes the whole
/// artwork there first, so an interrupted write leaves an artwork-sized file in the art folder,
/// where nothing else ever looks. An hour old means no write can still be using it.
pub(crate) fn remove_stale_kvctmp(path: &Path) {
    let tmp = kvctmp_of(path);
    let stale = std::fs::metadata(&tmp)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age > std::time::Duration::from_secs(3600));
    if stale {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// `fs::rename`, retried for about a second while Windows reports the target busy. Sync clients
/// (OneDrive, Dropbox) and antivirus scanners open a file for a moment all the time — the artwork,
/// or a temp file we just wrote — which fails the rename with a sharing violation or access
/// denied; the next try usually lands. A real permission problem still fails, a second later.
pub(crate) fn rename_retrying(from: &Path, to: &Path) -> std::io::Result<()> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    let mut pause = std::time::Duration::from_millis(20);
    loop {
        match std::fs::rename(from, to) {
            Err(e) if held_open(&e) && std::time::Instant::now() < deadline => {
                std::thread::sleep(pause);
                pause = (pause * 2).min(std::time::Duration::from_millis(250));
            }
            other => return other,
        }
    }
}

/// ERROR_SHARING_VIOLATION, or access denied — which Windows also reports for a file another
/// process has open without delete sharing. Neither means anything transient elsewhere.
fn held_open(e: &std::io::Error) -> bool {
    cfg!(windows)
        && (e.raw_os_error() == Some(32) || e.kind() == std::io::ErrorKind::PermissionDenied)
}

/// Write and `fsync` one file. `sync_all` (metadata included) rather than `sync_data`: the file
/// is brand new, so its size is part of what has to survive.
pub(crate) fn sync_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut f = std::fs::File::create(path).map_err(|e| io_at(path, e))?;
    f.write_all(bytes).map_err(|e| io_at(path, e))?;
    f.sync_all().map_err(|e| io_at(path, e))?;
    Ok(())
}

/// Fsync the directory holding `path` so the rename itself is durable. POSIX only — on Windows
/// the rename is journaled by the filesystem and a directory handle can't be opened for sync
/// without `FILE_FLAG_BACKUP_SEMANTICS`. Best-effort: a failure here doesn't invalidate the write.
#[cfg(unix)]
pub(crate) fn sync_parent_dir(path: &Path) {
    if let Some(dir) = path.parent() {
        if let Ok(f) = std::fs::File::open(dir) {
            let _ = f.sync_all();
        }
    }
}

#[cfg(not(unix))]
pub(crate) fn sync_parent_dir(_path: &Path) {}

/// Compact (not pretty) — `.kvc/` JSON is machine state; pretty-printing scaled
/// badly with history size back when chains were JSON too.
fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec(value).map_err(|e| KvcError::BadIndex(e.to_string()))?;
    write_atomic(path, &bytes)
}

/// `<path>` with `.bak` appended (not substituted — mirrors [`write_file_atomic`]'s `.kvctmp`
/// suffix, so `branches.json` and a hypothetical `branches.json`-named art file, if one ever
/// existed, wouldn't collide).
fn backup_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".bak");
    PathBuf::from(name)
}

/// [`write_json`], but first best-effort copies the *current* on-disk file to `<path>.bak` — one
/// previous generation of insurance for the handful of small state files (`index.json`,
/// `branches.json`, `stashes.json`) whose loss is unrecoverable (there's no remote to re-fetch
/// from). The backup copy isn't itself atomic/fsynced: a crash mid-copy just leaves a stale or
/// torn `.bak`, and the *primary* file (untouched at that point) is what the next save's copy
/// step refreshes it from — never a regression from today's zero-backup state.
fn write_json_with_backup<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if path.is_file() {
        let _ = std::fs::copy(path, backup_path(path));
    }
    write_json(path, value)
}

/// [`read_json`], falling back to `<path>.bak` on a **decode** failure only — a missing primary
/// file is a different problem (some callers treat "absent" as a valid empty-state default) and
/// shouldn't silently resurrect a stale backup instead of surfacing that. Mirrors
/// `parse_commit_log`'s "degrade, don't propagate garbage" shape for the one-shot whole-file case.
fn read_json_with_backup<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    match read_json(path) {
        Err(KvcError::BadIndex(_)) => {
            let bak = backup_path(path);
            if bak.is_file() {
                if let Ok(v) = read_json(&bak) {
                    return Ok(v);
                }
            }
            read_json(path)
        }
        other => other,
    }
}

/// Chain shard format tag.
const CHAINS_MAGIC: &[u8; 5] = b"KVCC2";

/// One chain shard on disk: `KVCC2` + zstd-compressed bincode. zstd level 1 — the stream keys
/// are highly repetitive, so even the fastest level compresses them several-fold.
fn write_chains_file(path: &Path, chains: &Chains) -> Result<()> {
    let plain = bincode::serialize(chains).map_err(|e| KvcError::BadIndex(e.to_string()))?;
    let z = zstd::encode_all(&plain[..], 1).map_err(KvcError::Io)?;
    let mut bytes = Vec::with_capacity(CHAINS_MAGIC.len() + z.len());
    bytes.extend_from_slice(CHAINS_MAGIC);
    bytes.extend_from_slice(&z);
    write_atomic(path, &bytes)
}

/// Decode a chains payload; `None` on anything unreadable.
fn decode_chains(raw: &[u8]) -> Option<Chains> {
    let plain = zstd::decode_all(raw.strip_prefix(CHAINS_MAGIC.as_slice())?).ok()?;
    bincode::deserialize(&plain).ok()
}

/// Read one chain shard; `None` on missing/unreadable (a missing shard is an empty one).
pub(crate) fn read_chains_file(path: &Path) -> Option<Chains> {
    decode_chains(&std::fs::read(path).ok()?)
}

/// Serialize commits as JSON-lines (one compact record + `\n` per commit).
fn commit_lines(commits: &[Commit]) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    for c in commits {
        serde_json::to_writer(&mut buf, c).map_err(|e| KvcError::BadIndex(e.to_string()))?;
        buf.push(b'\n');
    }
    Ok(buf)
}

/// What a `commits.log` body decoded to.
pub(crate) struct ParsedLog {
    pub commits: Vec<Commit>,
    /// The last line didn't decode: a crash mid-append. `branches.json` is written after the log,
    /// so that record was never a tip — it's dropped, and the next save rewrites the log without it
    /// rather than appending onto the fragment.
    pub torn_tail: bool,
    /// 1-based numbers of undecodable lines with good lines after them. A crash can't do that
    /// (appends only ever tear the last line), so this is damage in place — a failing sector, a
    /// sync client's conflicted copy, a hand edit — and every line that does decode is kept.
    pub damaged: Vec<usize>,
}

/// Parse an append-only `commits.log` body. Split out from `load_history` so the same rules
/// apply to a log read straight out of a backup archive ([`Repo::backup_versions`]), which has no
/// store on disk to open.
///
/// Stopping at the first bad line, as this once did, was right for a torn tail and silently
/// shortened history for damage in the middle: every version after it was dropped, and the next
/// save rewrote the log from that shortened list.
pub(crate) fn parse_commit_log(bytes: &[u8]) -> ParsedLog {
    let mut commits = Vec::new();
    let mut bad = Vec::new();
    let mut last_line = 0;
    for (i, line) in bytes.split(|&b| b == b'\n').enumerate() {
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        last_line = i + 1;
        match serde_json::from_slice::<Commit>(line) {
            Ok(c) => commits.push(c),
            Err(_) => bad.push(i + 1),
        }
    }
    let torn_tail = bad.last() == Some(&last_line);
    if torn_tail {
        bad.pop();
    }
    ParsedLog {
        commits,
        torn_tail,
        damaged: bad,
    }
}

/// The commit history, the branches that point into it, and what the log's state means for the
/// next save.
struct LoadedHistory {
    commits: Vec<Commit>,
    branches: Branches,
    persisted: usize,
    rewrite: bool,
    damage: Option<String>,
}

fn load_history(kvc: &Path) -> Result<LoadedHistory> {
    let path = kvc.join("commits.log");
    let log = parse_commit_log(&std::fs::read(&path).map_err(|e| io_at(&path, e))?);
    let damage = match log.damaged.as_slice() {
        [] => None,
        [line] => Some(format!("commits.log line {line} is unreadable")),
        lines => Some(format!(
            "commits.log lines {} are unreadable",
            lines
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        )),
    };
    Ok(LoadedHistory {
        persisted: log.commits.len(),
        rewrite: log.torn_tail,
        commits: log.commits,
        branches: read_json_with_backup(&kvc.join("branches.json"))?,
        damage,
    })
}

/// Why a document with no store at its expected path failed to open. Split out from
/// [`Repo::locate`] so the rule can be tested without mutating the process-global store root.
pub fn locate_failure(kra_path: &Path, custom_root: Option<&Path>) -> KvcError {
    match custom_root {
        // The store root is configured but not currently mounted — say so. Reporting `NotARepo`
        // here would invite the UI to offer "start tracking", minting an empty store and
        // orphaning every version the artist saved.
        Some(root) if !root.is_dir() => KvcError::StoreUnreachable(root.to_path_buf()),
        _ => KvcError::NotARepo(kra_path.to_path_buf()),
    }
}

/// Before creating a store: a configured store root that isn't there has been moved, renamed or
/// unplugged, and creating it afresh would start a new, empty history beside the real one — the
/// case `StoreUnreachable` exists to prevent. Same rule [`Repo::locate`] applies on open.
fn refuse_missing_store_root(kra_path: &Path) -> Result<()> {
    match locate_failure(kra_path, custom_store_root().as_deref()) {
        e @ KvcError::StoreUnreachable(_) => Err(e),
        _ => Ok(()),
    }
}

/// A store always carries `doc.json`; a missing one means the store is damaged, not empty.
fn read_doc_meta(store: &Path) -> Result<DocMeta> {
    read_json(&store.join("doc.json"))
}

/// Read just `branches.json`'s generation counter — for a cheap before/after staleness check
/// around a read command, without paying for a full [`Repo::open_light`] (which also re-reads
/// `commits.log`, the part that actually scales with history). A missing `branches.json` reads as
/// generation 0; the open it brackets is what reports it.
pub fn read_branches_generation(store: &Path) -> Result<u64> {
    let path = store.join("branches.json");
    if !path.is_file() {
        return Ok(0);
    }
    read_json_with_backup::<Branches>(&path).map(|b| b.generation)
}

/// An absent `stashes.json` is an empty shelf — that's the whole migration for repos that
/// predate stashing.
fn read_stashes(kvc: &Path) -> Result<Stashes> {
    let path = kvc.join("stashes.json");
    if path.is_file() {
        read_json_with_backup(&path)
    } else {
        Ok(Stashes::default())
    }
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let bytes = std::fs::read(path).map_err(|e| io_at(path, e))?;
    serde_json::from_slice(&bytes)
        .map_err(|e| KvcError::BadIndex(format!("{}: {e}", path.display())))
}

/// blake3 of a byte slice as lowercase hex. Multi-MB buffers (whole .kra files on scan/commit)
/// hash multi-core when already on a worker of the budgeted pool; small buffers (tiles) stay on
/// the cheap single-threaded path. Off the pool — the cheap reads `commands::run` keeps out of it
/// — `update_rayon` would spill onto rayon's global pool: every core at normal priority, exactly
/// what `cpu` exists to prevent.
pub fn hash_bytes(bytes: &[u8]) -> String {
    if bytes.len() >= 1 << 20 && rayon::current_thread_index().is_some() {
        let mut h = blake3::Hasher::new();
        h.update_rayon(bytes);
        h.finalize().to_hex().to_string()
    } else {
        blake3::hash(bytes).to_hex().to_string()
    }
}

/// `(size, mtime)` for a path, for the scanner's re-hash cache. mtime is **nanoseconds** since the
/// epoch — second resolution is too coarse (a save in the same second as the last commit, at the
/// same size, would be missed). Best-effort: 0 if unavailable (forces a re-hash, which is safe).
/// Relies on the OS updating mtime on every save (Krita rewrites the file, so it does);
/// a tool that preserves mtime while changing same-size content would slip past — upgrade path is
/// git's "racy" rule (re-hash anything whose mtime isn't strictly older than the last index write).
pub fn size_mtime(meta: &std::fs::Metadata) -> (u64, u64) {
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    (meta.len(), mtime)
}

/// Current time as ISO-8601 UTC (`YYYY-MM-DDTHH:MM:SSZ`), no date crate.
pub fn now_iso() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    epoch_to_iso(secs)
}

/// [`now_iso`] with `:` replaced by `-` — a valid directory/file name on Windows, which
/// forbids colons outside a drive letter.
pub fn now_iso_filesafe() -> String {
    now_iso().replace(':', "-")
}

/// Unix epoch seconds -> ISO-8601 UTC. Civil-from-days (Howard Hinnant) algorithm.
pub fn epoch_to_iso(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);

    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_join_allows_normal_relative_paths() {
        let root = Path::new("/repo");
        assert_eq!(safe_join(root, "art.kra").unwrap(), root.join("art.kra"));
        assert_eq!(
            safe_join(root, "sub/dir/art.kra").unwrap(),
            root.join("sub").join("dir").join("art.kra")
        );
    }

    #[test]
    fn read_capped_enforces_the_limit() {
        use std::io::Read;
        // Under the cap: full content returned.
        assert_eq!(read_capped(&b"hello"[..], 10).unwrap(), b"hello");
        // Over the cap: a clean error, not an unbounded read (a decompression-bomb guard).
        let big = std::io::repeat(1u8).take(50);
        assert!(matches!(read_capped(big, 10), Err(KvcError::CorruptZip(_))));
    }

    fn write_test_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let file = std::fs::File::create(path).unwrap();
        let mut zw = ZipWriter::new(file);
        for (name, data) in entries {
            zw.start_file(*name, SimpleFileOptions::default()).unwrap();
            zw.write_all(data).unwrap();
        }
        zw.finish().unwrap();
    }

    #[test]
    fn verify_zip_accepts_matching_entry_count_and_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("backup.zip");
        let manifest = serde_json::to_vec(&BackupManifest {
            version: 2,
            timestamp: now_iso(),
            app_version: "1.1.0".into(),
            entries: vec![BackupEntry {
                dir: "art-abc123".into(),
                relpath: "art.kra".into(),
                original_dir: "/somewhere".into(),
                branch: "main".into(),
                tip_commit: String::new(),
            }],
        })
        .unwrap();
        write_test_zip(&path, &[("a.gpl", b"data"), ("MANIFEST.json", &manifest)]);

        assert!(verify_zip(&path, 2, true).is_ok());
    }

    #[test]
    fn verify_zip_rejects_entry_count_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("backup.zip");
        write_test_zip(&path, &[("a.gpl", b"data")]);

        // The archive really has one entry — claiming two must fail, not silently pass.
        assert!(matches!(
            verify_zip(&path, 2, false),
            Err(KvcError::CorruptZip(_))
        ));
    }

    #[test]
    fn verify_zip_rejects_missing_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("backup.zip");
        write_test_zip(&path, &[("a.gpl", b"data")]);

        assert!(matches!(
            verify_zip(&path, 1, true),
            Err(KvcError::CorruptZip(_))
        ));
    }

    #[test]
    fn safe_join_rejects_escapes() {
        let root = Path::new("/repo");
        // Parent traversal, empty, and absolute/root paths are all refused.
        for bad in [
            "",
            "..",
            "../evil",
            "a/../../evil",
            "/etc/passwd",
            "/abs/path",
        ] {
            assert!(
                matches!(safe_join(root, bad), Err(KvcError::BadPath(_))),
                "expected {bad:?} to be rejected"
            );
        }
    }

    #[test]
    #[cfg(windows)]
    fn safe_join_rejects_windows_drive_and_unc() {
        let root = Path::new(r"C:\repo");
        for bad in [
            r"C:\Windows\System32\evil",
            r"\\server\share\evil",
            r"..\..\evil",
        ] {
            assert!(
                matches!(safe_join(root, bad), Err(KvcError::BadPath(_))),
                "expected {bad:?} to be rejected"
            );
        }
    }
}

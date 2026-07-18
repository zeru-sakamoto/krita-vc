//! `.kvc/` repository: on-disk layout, JSON state schema, and lifecycle (init / open).
//!
//! Layout:
//! ```text
//! .kvc/
//!   config.json    engine config (delta-chain threshold, tile size, cache budget)
//!   index.json     committed head per tracked file (drives the scanner)
//!   chains/        delta-stream versions, one shard per tracked file (drives storage/restore);
//!                  zstd-compressed bincode per shard. Repos from before sharding carry a
//!                  monolithic chains.bin (or older chains.json) instead — split on first save
//!   commits.log    commit log, JSON-lines, append-only (a commit appends one line instead of
//!                  rewriting the whole history; undo/GC rewrite it). Legacy repos carry a
//!                  commits.json instead — migrated to the log on first save
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
    pub fn acquire(root: &Path, op: &str) -> Result<Self> {
        let path = kvc_dir(root).join("kvc.lock");
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&path)
            .map_err(|e| io_at(&path, e))?;
        match file.try_lock() {
            Ok(()) => {
                let _ = write_lock_info(root, op); // best-effort; never blocks the acquire
                Ok(RepoLock(file))
            }
            Err(std::fs::TryLockError::WouldBlock) => Err(KvcError::Locked(
                lock_holder_description(root, &lock_info_path(root)),
            )),
            Err(std::fs::TryLockError::Error(e)) => Err(io_at(&path, e)),
        }
    }
}
// No `impl Drop`: dropping `File` closes the handle, and the OS releases the lock along with
// it — including when the process is killed, which is exactly the case a marker file can't.

fn lock_info_path(root: &Path) -> PathBuf {
    kvc_dir(root).join("kvc.lock.info")
}

/// Best-effort rewrite of the `kvc.lock.info` sidecar right after acquiring the real lock.
fn write_lock_info(root: &Path, op: &str) -> std::io::Result<()> {
    std::fs::write(lock_info_path(root), op)
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

pub fn kvc_dir(root: &Path) -> PathBuf {
    root.join(KVC_DIR)
}
pub fn objects_dir(root: &Path) -> PathBuf {
    kvc_dir(root).join("objects")
}
/// Content-addressed capped-raster cache (see `raster::cache_read`/`cache_write`). Created by
/// `init`; writes `create_dir_all` lazily so repos from before the cache existed keep working.
pub fn cache_dir(root: &Path) -> PathBuf {
    kvc_dir(root).join("cache")
}
/// Per-file chain shards (see [`ChainStore`]).
pub fn chains_dir(root: &Path) -> PathBuf {
    kvc_dir(root).join("chains")
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

/// Read `config.json`, applying version migrations in-memory; `dirty` = the caller's `save()`
/// should persist the migrated form. v1 → v2: the cache budget default dropped 512 → 256 MB,
/// and 512 MB always meant "old default", never a user choice (there is no UI knob).
fn read_config(kvc: &Path) -> Result<(Config, bool)> {
    let mut config: Config = read_json(&kvc.join("config.json"))?;
    let mut dirty = false;
    if config.version < 2 {
        if config.cache_max_bytes == 512 * 1024 * 1024 {
            config.cache_max_bytes = default_cache_max_bytes();
        }
        config.version = 2;
        dirty = true;
    }
    Ok((config, dirty))
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
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Index {
    /// relative path (forward-slashed) -> committed head
    pub files: BTreeMap<String, TrackedFile>,
}

/// One stored version of a delta stream. A stream is any byte sequence we version
/// (a generic file, a .kra manifest, a single layer entry, or a single tile).
///
/// The object file's name is fully derivable from `hash` + `base` ([`Version::object_name`]);
/// pre-KVCC2 shards stored it redundantly per version — dropped because chains grow with
/// total tile-version history, so every duplicated 64-hex hash was paid forever.
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

/// Pre-KVCC2 on-disk `Version` (bincode is not self-describing — the exact old shape is
/// needed to decode legacy shards/monoliths). `object` is dropped on conversion.
#[derive(Deserialize)]
struct VersionV1 {
    hash: String,
    #[allow(dead_code)]
    object: String,
    base: Option<String>,
    chain_len: usize,
}

impl From<VersionV1> for Version {
    fn from(v: VersionV1) -> Version {
        Version {
            hash: v.hash,
            base: v.base,
            chain_len: v.chain_len,
        }
    }
}

/// streamKey -> ordered versions (head = last). The serialized form of one chain shard
/// (and of the pre-sharding monolithic chains file).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Chains(pub BTreeMap<String, Vec<Version>>);

/// Shard identity for a stream key. Every key embeds the tracked file's relpath
/// (`file:{rel}`, `kra:{rel}:manifest`, `kra:{rel}:entry:{name}`, `kra:{rel}:tile:{e}:{x},{y}`),
/// and all of one file's keys must land in one shard so a commit rewrites exactly the shards
/// of the files it touched. The parse is forgiving: a pathological relpath containing a marker
/// still maps *consistently* (same bucket for every key of that file), which is all sharding
/// needs — a shard is a bucket, not an identity.
fn shard_of(key: &str) -> &str {
    if let Some(rest) = key.strip_prefix("file:") {
        return rest;
    }
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

fn shard_file(dir: &Path, shard: &str) -> PathBuf {
    dir.join(format!(
        "{}.bin",
        &blake3::hash(shard.as_bytes()).to_hex()[..16]
    ))
}

/// Per-file-sharded delta chains, loaded lazily. The monolithic predecessor (`chains.bin`) held
/// every version of every tile stream ever committed, was rewritten in full on every commit and
/// parsed in full on every open — the one cost that grew with *total repo history* instead of
/// with the change at hand. Shards make both proportional to the files actually touched.
///
/// Interior mutability (`RwLock`) lets read paths fault shards in from behind `&Repo` (rayon
/// `par_iter` reconstructs included); pushes come only from the serial commit folds.
pub struct ChainStore {
    dir: PathBuf,
    /// Loaded shards by shard name (the relpath bucket). `Arc` so readers can hold a shard
    /// without keeping the map locked.
    shards: RwLock<HashMap<String, Arc<Chains>>>,
    dirty: Mutex<HashSet<String>>,
    /// Set when this store was populated from a legacy monolithic chains file: the next
    /// [`ChainStore::flush`] writes every shard, then deletes the monolith. Until that delete,
    /// the monolith remains the source of truth on open — a crash mid-split just re-runs it.
    retire_monolith: bool,
}

impl ChainStore {
    fn empty(dir: PathBuf) -> ChainStore {
        ChainStore {
            dir,
            shards: RwLock::new(HashMap::new()),
            dirty: Mutex::new(HashSet::new()),
            retire_monolith: false,
        }
    }

    /// Partition a legacy monolithic chains map into all-dirty shards (split persists on the
    /// next save).
    fn from_legacy(dir: PathBuf, legacy: Chains) -> ChainStore {
        let mut map: HashMap<String, Chains> = HashMap::new();
        for (key, versions) in legacy.0 {
            map.entry(shard_of(&key).to_string())
                .or_default()
                .0
                .insert(key, versions);
        }
        let dirty: HashSet<String> = map.keys().cloned().collect();
        let shards = map.into_iter().map(|(k, v)| (k, Arc::new(v))).collect();
        ChainStore {
            dir,
            shards: RwLock::new(shards),
            dirty: Mutex::new(dirty),
            retire_monolith: true,
        }
    }

    /// The loaded shard for `name`, faulting it in from disk (missing file = empty shard,
    /// negative-cached so repeat misses don't re-stat).
    fn load(&self, name: &str) -> Arc<Chains> {
        if let Some(s) = self.shards.read().unwrap().get(name) {
            return s.clone();
        }
        let mut w = self.shards.write().unwrap();
        if let Some(s) = w.get(name) {
            return s.clone();
        }
        let loaded = read_chains_file(&shard_file(&self.dir, name)).unwrap_or_default();
        let arc = Arc::new(loaded);
        w.insert(name.to_string(), arc.clone());
        arc
    }

    /// The version chain for `key` (cloned — chains are short by design: at most
    /// `delta_chain_max`+1 per snapshot run).
    pub fn chain(&self, key: &str) -> Option<Vec<Version>> {
        self.load(shard_of(key)).0.get(key).cloned()
    }

    /// Append a version to `key`'s chain and mark its shard dirty.
    pub(crate) fn push(&self, key: String, version: Version) {
        let name = shard_of(&key).to_string();
        self.load(&name); // fault in before mutating, so we never clobber an unread shard
        let mut w = self.shards.write().unwrap();
        let entry = w.get_mut(&name).expect("shard just loaded");
        Arc::make_mut(entry).0.entry(key).or_default().push(version);
        self.dirty.lock().unwrap().insert(name);
    }

    /// Write every dirty shard (atomic each), then retire a legacy monolith if one is pending.
    /// A failed write leaves its dirty mark in place for the next save.
    fn flush(&mut self, kvc: &Path) -> Result<()> {
        let dirty: Vec<String> = self.dirty.lock().unwrap().iter().cloned().collect();
        if dirty.is_empty() && !self.retire_monolith {
            return Ok(());
        }
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
        if self.retire_monolith {
            let _ = std::fs::remove_file(kvc.join("chains.bin"));
            let _ = std::fs::remove_file(kvc.join("chains.json"));
            self.retire_monolith = false;
        }
        Ok(())
    }

    fn has_dirty(&self) -> bool {
        self.retire_monolith || !self.dirty.lock().unwrap().is_empty()
    }

    /// Replace the whole store with `new` (GC sweep): partition into shards, write every
    /// non-empty shard, then delete shard files whose bucket no longer exists. Writes happen
    /// before deletes and each write is atomic, so there is no window without valid chains.
    pub fn rewrite_all(&mut self, kvc: &Path, new: Chains) -> Result<()> {
        let mut map: HashMap<String, Chains> = HashMap::new();
        for (key, versions) in new.0 {
            map.entry(shard_of(&key).to_string())
                .or_default()
                .0
                .insert(key, versions);
        }
        std::fs::create_dir_all(&self.dir).map_err(|e| io_at(&self.dir, e))?;
        let keep: HashSet<PathBuf> = map
            .iter()
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
        // In-memory state now mirrors disk; a pending legacy monolith is finally retired too.
        self.shards = RwLock::new(map.into_iter().map(|(k, v)| (k, Arc::new(v))).collect());
        self.dirty = Mutex::new(HashSet::new());
        if self.retire_monolith {
            let _ = std::fs::remove_file(kvc.join("chains.bin"));
            let _ = std::fs::remove_file(kvc.join("chains.json"));
            self.retire_monolith = false;
        }
        Ok(())
    }

    /// Every chain across every shard, on-disk and in-memory merged (in-memory wins — it is
    /// never older). Loads the whole store: tests and GC only, never a hot path.
    pub fn export_all(&self) -> Chains {
        let mut all = Chains::default();
        if let Ok(rd) = std::fs::read_dir(&self.dir) {
            for e in rd.flatten() {
                if e.path().extension().is_some_and(|x| x == "bin") {
                    if let Some(c) = read_chains_file(&e.path()) {
                        all.0.extend(c.0);
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
}

impl Default for Branches {
    fn default() -> Self {
        let mut branches = BTreeMap::new();
        branches.insert("main".to_string(), String::new());
        Branches {
            current: "main".to_string(),
            branches,
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

    /// Migration for repos created before branching existed: everything lives on `main`,
    /// whose tip is the newest commit. Persisted by the next `save()`.
    fn migrated(commits: &[Commit]) -> Branches {
        let mut b = Branches::default();
        if let Some(last) = commits.last() {
            b.set_tip(&last.id);
        }
        b
    }
}

/// Loaded repository state. Mutated in-memory then flushed with [`Repo::save`].
pub struct Repo {
    pub root: PathBuf,
    pub config: Config,
    pub index: Index,
    /// Delta chains, sharded per tracked file and loaded lazily — see [`ChainStore`].
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
    /// Force a full log rewrite on the next `save()`: set on legacy `commits.json` migration
    /// and whenever `commits` was truncated (undo, GC) — see [`Repo::note_commits_truncated`].
    commits_rewrite: bool,
    /// `config` was migrated on load — the next `save()` persists it (saves otherwise never
    /// touch `config.json`).
    config_dirty: bool,
}

impl Repo {
    pub fn is_repo(root: &Path) -> bool {
        kvc_dir(root).join("config.json").is_file()
    }

    /// Create a fresh `.kvc/` in `root`.
    pub fn init(root: &Path) -> Result<()> {
        let kvc = kvc_dir(root);
        if kvc.join("config.json").exists() {
            return Err(KvcError::AlreadyRepo(root.to_path_buf()));
        }

        // Ancestor guard: refuse to nest inside an existing repo. Just path-component
        // walking + a config.json stat per ancestor, no directory traversal.
        for ancestor in root.ancestors().skip(1) {
            if Self::is_repo(ancestor) {
                return Err(KvcError::NestedRepo(ancestor.to_path_buf()));
            }
        }

        // Descendant guard: refuse to swallow an existing repo somewhere below root.
        // root may not exist yet (created below via create_dir_all), so skip the walk
        // entirely rather than let WalkDir error on a missing path.
        if root.is_dir() {
            let walker = walkdir::WalkDir::new(root)
                .into_iter()
                .filter_entry(|e| e.file_name() != KVC_DIR);
            for entry in walker {
                let entry = entry.map_err(walk_err)?;
                if entry.depth() > 0 && entry.file_type().is_dir() && Self::is_repo(entry.path()) {
                    return Err(KvcError::ContainsRepo(entry.path().to_path_buf()));
                }
            }
        }

        std::fs::create_dir_all(objects_dir(root)).map_err(|e| io_at(&kvc, e))?;
        std::fs::create_dir_all(cache_dir(root)).map_err(|e| io_at(&kvc, e))?;
        std::fs::create_dir_all(chains_dir(root)).map_err(|e| io_at(&kvc, e))?;
        write_json(&kvc.join("config.json"), &Config::default())?;
        write_json(&kvc.join("index.json"), &Index::default())?;
        write_atomic(&kvc.join("commits.log"), b"")?;
        write_json(&kvc.join("branches.json"), &Branches::default())?;
        Ok(())
    }

    /// Delete a repository folder (its whole tree, art files included), preferring the OS
    /// Recycle Bin so an accidental delete stays recoverable from Explorer/Finder. Falls back to
    /// a permanent `remove_dir_all` if the trash move fails (e.g. no trash provider on that
    /// filesystem) so the action never gets stuck — the returned bool tells the caller which
    /// happened. Guarded by [`is_repo`] so a stray path can't be wiped.
    pub fn delete(root: &Path) -> Result<bool> {
        if !Self::is_repo(root) {
            return Err(KvcError::NotARepo(root.to_path_buf()));
        }
        if trash::delete(root).is_ok() {
            return Ok(true);
        }
        std::fs::remove_dir_all(root).map_err(|e| io_at(root, e))?;
        Ok(false)
    }

    /// Zip a repository's whole folder (art files + `.kvc/`) to `dest` — a manual, on-demand
    /// backup for the user to move to their own cloud storage or an external drive. It's the
    /// only thing that helps against loss the app can't intervene in (the project folder deleted
    /// outside the app, disk failure, external corruption): extracting the zip anywhere and
    /// Browsing to it "just works" since [`is_repo`] only checks for `.kvc/config.json`.
    pub fn export_zip(root: &Path, dest: &Path) -> Result<()> {
        if !Self::is_repo(root) {
            return Err(KvcError::NotARepo(root.to_path_buf()));
        }
        let file = std::fs::File::create(dest).map_err(|e| io_at(dest, e))?;
        let mut zw = ZipWriter::new(file);
        let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        for entry in walkdir::WalkDir::new(root) {
            let entry = entry.map_err(walk_err)?;
            if !entry.file_type().is_file() {
                continue;
            }
            // Zip entry names are `/`-separated regardless of platform.
            let rel = entry
                .path()
                .strip_prefix(root)
                .unwrap_or(entry.path())
                .to_string_lossy()
                .replace('\\', "/");
            zw.start_file(rel, opts).map_err(zip_err)?;
            let bytes = std::fs::read(entry.path()).map_err(|e| io_at(entry.path(), e))?;
            zw.write_all(&bytes)?;
        }
        zw.finish().map_err(zip_err)?;
        Ok(())
    }

    /// Validate `.kvc/` and load its state. Chains load lazily per shard; only a repo still
    /// carrying the pre-sharding monolithic chains file pays a one-time full parse here (the
    /// split persists on the next save).
    pub fn open(root: &Path) -> Result<Repo> {
        if !Self::is_repo(root) {
            return Err(KvcError::NotARepo(root.to_path_buf()));
        }
        let kvc = kvc_dir(root);
        let (commits, persisted, migrate) = read_commits(&kvc)?;
        let branches = read_branches(&kvc, &commits)?;
        let chains = if kvc.join("chains.bin").is_file() || kvc.join("chains.json").is_file() {
            ChainStore::from_legacy(chains_dir(root), read_chains(&kvc)?)
        } else {
            ChainStore::empty(chains_dir(root))
        };
        let (config, config_dirty) = read_config(&kvc)?;
        Ok(Repo {
            root: root.to_path_buf(),
            config,
            index: read_json(&kvc.join("index.json"))?,
            chains,
            packs: crate::delta::Packs::default(),
            commits,
            branches,
            stashes: read_stashes(&kvc)?,
            commits_persisted: persisted,
            commits_rewrite: migrate,
            config_dirty,
        })
    }

    /// Like [`Repo::open`] but never touches chains — even the legacy-monolith parse is
    /// skipped. For read paths that never reconstruct or store (scan, log).
    /// Invariant: never call `reconstruct`/`store_stream`/`prepare_stream` on a light repo
    /// (on a legacy repo the store would come up empty).
    pub fn open_light(root: &Path) -> Result<Repo> {
        if !Self::is_repo(root) {
            return Err(KvcError::NotARepo(root.to_path_buf()));
        }
        let kvc = kvc_dir(root);
        let (commits, persisted, migrate) = read_commits(&kvc)?;
        let branches = read_branches(&kvc, &commits)?;
        let (config, config_dirty) = read_config(&kvc)?;
        Ok(Repo {
            root: root.to_path_buf(),
            config,
            index: read_json(&kvc.join("index.json"))?,
            chains: ChainStore::empty(chains_dir(root)),
            packs: crate::delta::Packs::default(),
            commits,
            branches,
            stashes: read_stashes(&kvc)?,
            commits_persisted: persisted,
            commits_rewrite: migrate,
            config_dirty,
        })
    }

    pub fn objects_dir(&self) -> PathBuf {
        objects_dir(&self.root)
    }

    pub fn cache_dir(&self) -> PathBuf {
        cache_dir(&self.root)
    }

    /// Mark the in-memory commit list as truncated (undo popped a commit, GC dropped
    /// unreachable ones) so the next [`Repo::save`] rewrites `commits.log` instead of appending.
    pub fn note_commits_truncated(&mut self) {
        self.commits_rewrite = true;
    }

    /// Flush mutated state atomically. Chains rewrite only their dirty shards — the shards of
    /// files this commit actually touched; switch/merge/undo mutate only index/commits/branches
    /// and skip chains entirely ([`ChainStore::has_dirty`]). The commit log normally takes one
    /// O(1) append per new commit — never a rewrite that grows with total history. Write order
    /// matters: `branches.json` (the tips) goes last, so a torn log append is always an
    /// unreachable orphan record, never a dangling branch tip. `stashes.json` follows for the
    /// same reason — a stash record must never outlive the chain content it points at.
    pub fn save(&mut self) -> Result<()> {
        let kvc = kvc_dir(&self.root);
        if self.config_dirty {
            write_json(&kvc.join("config.json"), &self.config)?;
            self.config_dirty = false;
        }
        write_json(&kvc.join("index.json"), &self.index)?;
        if self.chains.has_dirty() {
            self.chains.flush(&kvc)?;
        }
        self.flush_commits(&kvc)?;
        write_json(&kvc.join("branches.json"), &self.branches)?;
        write_json(&kvc.join("stashes.json"), &self.stashes)?;
        Ok(())
    }

    /// Persist `config` alone — for settings edits, which never touch index/chains/commits/
    /// branches and shouldn't pay for `save()`'s full flush.
    pub fn save_config(&mut self) -> Result<()> {
        write_json(&kvc_dir(&self.root).join("config.json"), &self.config)?;
        self.config_dirty = false;
        Ok(())
    }

    fn flush_commits(&mut self, kvc: &Path) -> Result<()> {
        let log = kvc.join("commits.log");
        if self.commits_rewrite {
            write_atomic(&log, &commit_lines(&self.commits)?)?;
            // Retire a legacy commits.json once the log is safely in place (mirror of the
            // chains-monolith retirement).
            let _ = std::fs::remove_file(kvc.join("commits.json"));
            self.commits_rewrite = false;
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
            self.commits_persisted = self.commits.len();
        }
        Ok(())
    }

    /// Flush only `branches.json` — safe on a [`Repo::open_light`] repo, where a full
    /// [`Repo::save`] rewrites index/commits from possibly-partial state.
    pub fn save_branches(&self) -> Result<()> {
        write_json(&kvc_dir(&self.root).join("branches.json"), &self.branches)
    }

    /// Flush only `stashes.json` — same reasoning as [`Repo::save_branches`]: dropping a stash
    /// runs on an `open_light` repo, where a full [`Repo::save`] would rewrite index/commits from
    /// possibly-partial state.
    pub fn save_stashes(&self) -> Result<()> {
        write_json(&kvc_dir(&self.root).join("stashes.json"), &self.stashes)
    }
}

/// Atomic write: bytes to a temp file in the same dir, then rename over the target
/// (Rust's `fs::rename` replaces the destination on Windows and POSIX).
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).map_err(|e| io_at(&tmp, e))?;
    std::fs::rename(&tmp, path).map_err(|e| io_at(path, e))?;
    Ok(())
}

/// Compact (not pretty) — `.kvc/` JSON is machine state; pretty-printing scaled
/// badly with history size back when chains were JSON too.
fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec(value).map_err(|e| KvcError::BadIndex(e.to_string()))?;
    write_atomic(path, &bytes)
}

/// Chain shard format tag. Bincode is not self-describing, so dropping `Version.object`
/// needed an explicit version: `KVCC2`-prefixed shards hold the slim `Version`; bare-zstd
/// files are pre-KVCC2 and decode through [`VersionV1`]. Old shards convert lazily — each
/// upgrades the next time its file is dirtied (or all at once in GC's `rewrite_all`).
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

/// Decode a chains payload in either format; `None` on anything unreadable.
fn decode_chains(raw: &[u8]) -> Option<Chains> {
    if let Some(body) = raw.strip_prefix(CHAINS_MAGIC.as_slice()) {
        let plain = zstd::decode_all(body).ok()?;
        bincode::deserialize(&plain).ok()
    } else {
        // Pre-KVCC2: bare zstd(bincode) with the redundant `object` field per version.
        let plain = zstd::decode_all(raw).ok()?;
        let legacy: BTreeMap<String, Vec<VersionV1>> = bincode::deserialize(&plain).ok()?;
        Some(Chains(
            legacy
                .into_iter()
                .map(|(k, vs)| (k, vs.into_iter().map(Version::from).collect()))
                .collect(),
        ))
    }
}

/// Read one chain shard; `None` on missing/unreadable (a missing shard is an empty one).
fn read_chains_file(path: &Path) -> Option<Chains> {
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

/// Load the commit history: `(commits, lines persisted in the log, migrate-from-legacy)`.
/// Prefers `commits.log` (JSON-lines). A torn trailing line — a crash mid-append — is dropped
/// silently: `branches.json` is written after the log, so a torn record is never a branch tip.
/// A repo from before the log carries `commits.json` instead; it's parsed here and the caller
/// flags a rewrite so the next save writes the log and retires the legacy file.
fn read_commits(kvc: &Path) -> Result<(Vec<Commit>, usize, bool)> {
    let log = kvc.join("commits.log");
    if log.is_file() {
        let bytes = std::fs::read(&log).map_err(|e| io_at(&log, e))?;
        let mut commits = Vec::new();
        let mut torn = false;
        for line in bytes.split(|&b| b == b'\n') {
            if line.is_empty() {
                continue;
            }
            match serde_json::from_slice::<Commit>(line) {
                Ok(c) => commits.push(c),
                Err(_) => {
                    // Torn tail from a crash mid-append — an orphan record, drop it. Flag a
                    // rewrite so the partial line is scrubbed rather than appended onto.
                    torn = true;
                    break;
                }
            }
        }
        let n = commits.len();
        Ok((commits, n, torn))
    } else {
        let commits: Vec<Commit> = read_json(&kvc.join("commits.json"))?;
        Ok((commits, 0, true))
    }
}

/// Load a pre-sharding monolithic chains file: `chains.bin` (zstd bincode, always pre-KVCC2),
/// else the even older `chains.json` (self-describing JSON — its extra `object` field is
/// simply ignored by serde). Retired by the first save after opening ([`ChainStore::flush`]).
fn read_chains(kvc: &Path) -> Result<Chains> {
    let bin = kvc.join("chains.bin");
    if bin.is_file() {
        let raw = std::fs::read(&bin).map_err(|e| io_at(&bin, e))?;
        decode_chains(&raw).ok_or_else(|| KvcError::BadIndex(bin.display().to_string()))
    } else {
        read_json(&kvc.join("chains.json"))
    }
}

/// Load `branches.json`, migrating pre-branching repos in-memory (no write on the read path;
/// the next `save()` persists it).
fn read_branches(kvc: &Path, commits: &[Commit]) -> Result<Branches> {
    let path = kvc.join("branches.json");
    if path.is_file() {
        read_json(&path)
    } else {
        Ok(Branches::migrated(commits))
    }
}

/// An absent `stashes.json` is an empty shelf — that's the whole migration for repos that
/// predate stashing.
fn read_stashes(kvc: &Path) -> Result<Stashes> {
    let path = kvc.join("stashes.json");
    if path.is_file() {
        read_json(&path)
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
/// hash multi-core; small buffers (tiles) stay on the cheap single-threaded path.
pub fn hash_bytes(bytes: &[u8]) -> String {
    if bytes.len() >= 1 << 20 {
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

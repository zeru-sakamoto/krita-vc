//! `.kvc/` repository: on-disk layout, JSON state schema, and lifecycle (init / open).
//!
//! Layout:
//! ```text
//! .kvc/
//!   config.json    engine config (delta-chain threshold, tile size)
//!   index.json     committed head per tracked file (drives the scanner)
//!   chains.bin     every stored version of every delta stream (drives storage/restore);
//!                  zstd-compressed bincode — repos from before this format carry a legacy
//!                  chains.json instead, migrated on the next commit
//!   commits.json   commit log
//!   branches.json  branch name -> tip commit id, plus the current branch
//!   objects/       content-addressed blobs (<hash>.full / <hash>.patch)
//! ```

use crate::error::{io_at, KvcError, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const KVC_DIR: &str = ".kvc";

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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub version: u32,
    /// Max consecutive bsdiff patches before a fresh full snapshot is forced.
    pub delta_chain_max: usize,
    pub tile_size: u32,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            version: 1,
            delta_chain_max: 20,
            tile_size: 64,
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
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Index {
    /// relative path (forward-slashed) -> committed head
    pub files: BTreeMap<String, TrackedFile>,
}

/// One stored version of a delta stream. A stream is any byte sequence we version
/// (a generic file, a .kra manifest, a single layer entry, or a single tile).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Version {
    /// blake3 of the *reconstructed* bytes — also the object file's base name.
    pub hash: String,
    /// `<hash>.full` (zstd snapshot) or `<hash>.patch` (bsdiff against `base`).
    pub object: String,
    /// hash of the version this patch applies onto; `None` for a full snapshot.
    pub base: Option<String>,
    /// patches back to the nearest full snapshot (0 = full).
    pub chain_len: usize,
}

/// streamKey -> ordered versions (head = last).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Chains(pub BTreeMap<String, Vec<Version>>);

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
    pub chains: Chains,
    pub commits: Vec<Commit>,
    pub branches: Branches,
    /// True once a new stream version has been committed (`Repo::commit_prepared` — the only
    /// chain mutation). [`Repo::save`] skips rewriting the chains file (the largest state file)
    /// when clean, so switch/merge/undo never pay for it.
    pub(crate) chains_dirty: bool,
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
        std::fs::create_dir_all(objects_dir(root)).map_err(|e| io_at(&kvc, e))?;
        std::fs::create_dir_all(cache_dir(root)).map_err(|e| io_at(&kvc, e))?;
        write_json(&kvc.join("config.json"), &Config::default())?;
        write_json(&kvc.join("index.json"), &Index::default())?;
        write_chains(&kvc, &Chains::default())?;
        write_json(&kvc.join("commits.json"), &Vec::<Commit>::new())?;
        write_json(&kvc.join("branches.json"), &Branches::default())?;
        Ok(())
    }

    /// Permanently delete a repository folder (its whole tree, art files included).
    /// Guarded by [`is_repo`] so a stray path can't be wiped.
    pub fn delete(root: &Path) -> Result<()> {
        if !Self::is_repo(root) {
            return Err(KvcError::NotARepo(root.to_path_buf()));
        }
        std::fs::remove_dir_all(root).map_err(|e| io_at(root, e))
    }

    /// Validate `.kvc/` and load its state.
    pub fn open(root: &Path) -> Result<Repo> {
        if !Self::is_repo(root) {
            return Err(KvcError::NotARepo(root.to_path_buf()));
        }
        let kvc = kvc_dir(root);
        let commits: Vec<Commit> = read_json(&kvc.join("commits.json"))?;
        let branches = read_branches(&kvc, &commits)?;
        Ok(Repo {
            root: root.to_path_buf(),
            config: read_json(&kvc.join("config.json"))?,
            index: read_json(&kvc.join("index.json"))?,
            chains: read_chains(&kvc)?,
            commits,
            branches,
            chains_dirty: false,
        })
    }

    /// Like [`Repo::open`] but skips the chains file — by far the largest state file (every
    /// version of every tile stream). For read paths that never touch storage (scan, log).
    /// Invariant: never call `reconstruct`/`store_stream`/`prepare_stream` on a light repo.
    pub fn open_light(root: &Path) -> Result<Repo> {
        if !Self::is_repo(root) {
            return Err(KvcError::NotARepo(root.to_path_buf()));
        }
        let kvc = kvc_dir(root);
        let commits: Vec<Commit> = read_json(&kvc.join("commits.json"))?;
        let branches = read_branches(&kvc, &commits)?;
        Ok(Repo {
            root: root.to_path_buf(),
            config: read_json(&kvc.join("config.json"))?,
            index: read_json(&kvc.join("index.json"))?,
            chains: Chains::default(),
            commits,
            branches,
            chains_dirty: false,
        })
    }

    pub fn objects_dir(&self) -> PathBuf {
        objects_dir(&self.root)
    }

    pub fn cache_dir(&self) -> PathBuf {
        cache_dir(&self.root)
    }

    /// Flush mutated state atomically. The chains file — the largest, every version of every
    /// tile stream — is only rewritten when a new stream version was actually committed
    /// (`chains_dirty`); switch/merge/undo mutate only index/commits/branches.
    pub fn save(&mut self) -> Result<()> {
        let kvc = kvc_dir(&self.root);
        write_json(&kvc.join("index.json"), &self.index)?;
        if self.chains_dirty {
            write_chains(&kvc, &self.chains)?;
            self.chains_dirty = false;
        }
        write_json(&kvc.join("commits.json"), &self.commits)?;
        write_json(&kvc.join("branches.json"), &self.branches)?;
        Ok(())
    }

    /// Flush only `branches.json` — safe on a [`Repo::open_light`] repo, where a full
    /// [`Repo::save`] rewrites index/commits from possibly-partial state.
    pub fn save_branches(&self) -> Result<()> {
        write_json(&kvc_dir(&self.root).join("branches.json"), &self.branches)
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

/// ponytail: compact (not pretty) — `.kvc/` JSON is machine state; pretty-printing scaled
/// badly with history size back when chains were JSON too.
fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec(value).map_err(|e| KvcError::BadIndex(e.to_string()))?;
    write_atomic(path, &bytes)
}

/// `chains.bin`: zstd-compressed bincode. Chains are rewritten in full on every commit and
/// parsed on every `Repo::open`, and dwarf the other state files (one entry per version of
/// every tile stream ever committed) — JSON there dominated commit/switch time *and* disk.
/// zstd level 1: the stream keys are highly repetitive, so even the fastest level compresses
/// them several-fold. A successful write retires a legacy `chains.json`.
fn write_chains(kvc: &Path, chains: &Chains) -> Result<()> {
    let plain = bincode::serialize(chains).map_err(|e| KvcError::BadIndex(e.to_string()))?;
    let bytes = zstd::encode_all(&plain[..], 1).map_err(KvcError::Io)?;
    write_atomic(&kvc.join("chains.bin"), &bytes)?;
    let legacy = kvc.join("chains.json");
    if legacy.exists() {
        let _ = std::fs::remove_file(legacy);
    }
    Ok(())
}

/// Load chains, preferring `chains.bin`; repos from before the binary format fall back to
/// their legacy `chains.json` (migrated by the next [`write_chains`]).
fn read_chains(kvc: &Path) -> Result<Chains> {
    let bin = kvc.join("chains.bin");
    if bin.is_file() {
        let raw = std::fs::read(&bin).map_err(|e| io_at(&bin, e))?;
        let plain = zstd::decode_all(&raw[..]).map_err(KvcError::Io)?;
        bincode::deserialize(&plain).map_err(|e| KvcError::BadIndex(e.to_string()))
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
/// ponytail: relies on the OS updating mtime on every save (Krita rewrites the file, so it does);
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

//! `.kvc/` repository: on-disk layout, JSON state schema, and lifecycle (init / open).
//!
//! Layout:
//! ```text
//! .kvc/
//!   config.json    engine config (delta-chain threshold, tile size)
//!   index.json     committed head per tracked file (drives the scanner)
//!   chains.json    every stored version of every delta stream (drives storage/restore)
//!   commits.json   commit log
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
    pub files: Vec<CommittedFile>,
}

/// Loaded repository state. Mutated in-memory then flushed with [`Repo::save`].
pub struct Repo {
    pub root: PathBuf,
    pub config: Config,
    pub index: Index,
    pub chains: Chains,
    pub commits: Vec<Commit>,
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
        write_json(&kvc.join("config.json"), &Config::default())?;
        write_json(&kvc.join("index.json"), &Index::default())?;
        write_json(&kvc.join("chains.json"), &Chains::default())?;
        write_json(&kvc.join("commits.json"), &Vec::<Commit>::new())?;
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
        Ok(Repo {
            root: root.to_path_buf(),
            config: read_json(&kvc.join("config.json"))?,
            index: read_json(&kvc.join("index.json"))?,
            chains: read_json(&kvc.join("chains.json"))?,
            commits: read_json(&kvc.join("commits.json"))?,
        })
    }

    pub fn objects_dir(&self) -> PathBuf {
        objects_dir(&self.root)
    }

    /// Flush mutated state atomically.
    pub fn save(&self) -> Result<()> {
        let kvc = kvc_dir(&self.root);
        write_json(&kvc.join("index.json"), &self.index)?;
        write_json(&kvc.join("chains.json"), &self.chains)?;
        write_json(&kvc.join("commits.json"), &self.commits)?;
        Ok(())
    }
}

/// Atomic JSON write: serialize to a temp file in the same dir, then rename over the
/// target (Rust's `fs::rename` replaces the destination on Windows and POSIX).
fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| KvcError::BadIndex(e.to_string()))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &bytes).map_err(|e| io_at(&tmp, e))?;
    std::fs::rename(&tmp, path).map_err(|e| io_at(path, e))?;
    Ok(())
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let bytes = std::fs::read(path).map_err(|e| io_at(path, e))?;
    serde_json::from_slice(&bytes)
        .map_err(|e| KvcError::BadIndex(format!("{}: {e}", path.display())))
}

/// blake3 of a byte slice as lowercase hex.
pub fn hash_bytes(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
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

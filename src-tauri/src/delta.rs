//! Content-addressed delta-chain storage. A "stream" is any versioned byte sequence
//! (a generic file, a .kra manifest, a layer entry, or a single tile). Each new version
//! is stored either as a full zstd snapshot or a bsdiff patch against the previous head;
//! a configurable threshold caps consecutive patches so restores stay fast.

use crate::error::{io_at, KvcError, Result};
use crate::repo::{hash_bytes, Repo, Version};
use qbsdiff::{Bsdiff, Bspatch};
use std::io::Cursor;
use std::path::Path;

/// The result of preparing a version without touching the repo — either the content already
/// exists (dedup) or a new version plus the `(object_name, bytes)` still to be written.
/// Split out so the CPU-heavy work (reconstruct + bsdiff + verify + zstd) can run in parallel
/// across independent streams before a cheap serial fold applies them.
pub enum Prepared {
    Dedup(String),
    New {
        version: Version,
        object: (String, Vec<u8>),
    },
}

impl Prepared {
    /// Content hash of the prepared version — known before the serial fold, so callers can
    /// build references (e.g. manifest tile refs) while still preparing in parallel.
    pub fn hash(&self) -> &str {
        match self {
            Prepared::Dedup(h) => h,
            Prepared::New { version, .. } => &version.hash,
        }
    }
}

/// Per-stream storage knobs. `zstd_level` applies to full snapshots; `patch_floor` is the
/// minimum byte size for bsdiff patching (streams below it always store as fulls).
pub(crate) struct StoreOpts {
    pub zstd_level: i32,
    pub patch_floor: usize,
}

impl Default for StoreOpts {
    fn default() -> Self {
        StoreOpts {
            zstd_level: 3,
            patch_floor: 64 * 1024,
        }
    }
}

impl Repo {
    /// Store `bytes` as the next version of `key`. Returns the content hash. Identical
    /// content already in the chain is deduplicated (no new object, no new version).
    pub fn store_stream(&mut self, key: &str, bytes: &[u8]) -> Result<String> {
        let prepared = self.prepare_stream(key, bytes)?;
        self.commit_prepared(key, prepared)
    }

    /// Compute the next version for `key` without mutating the repo or writing to disk. Read-only
    /// (`&self`), so many independent streams can be prepared in parallel. The head it patches
    /// against is read here, so callers must not commit versions to `key` between prepare and
    /// commit — safe for a single commit where each stream key appears once.
    pub fn prepare_stream(&self, key: &str, bytes: &[u8]) -> Result<Prepared> {
        // Krita tiles are already LZF-compressed and raw entries that sniff as compressed
        // (PNG/zip) barely shrink at any zstd level — but level 3 over thousands of tiles was
        // the single largest CPU term of a whole-document commit. Level 1 costs ~nothing in
        // size on such payloads; diff-friendly text (manifests, XML) keeps level 3.
        let level = if looks_compressed(bytes) || key.contains(":tile:") {
            1
        } else {
            3
        };
        self.prepare_stream_opts(
            key,
            bytes,
            StoreOpts {
                zstd_level: level,
                ..Default::default()
            },
        )
    }

    /// [`Repo::prepare_stream`] with explicit storage knobs (compression level, patch floor).
    pub(crate) fn prepare_stream_opts(
        &self,
        key: &str,
        bytes: &[u8],
        opts: StoreOpts,
    ) -> Result<Prepared> {
        let hash = hash_bytes(bytes);
        let max = self.config.delta_chain_max;

        let (dedup, head) = match self.chains.chain(key) {
            Some(v) => (v.iter().any(|x| x.hash == hash), v.last().cloned()),
            None => (false, None),
        };
        if dedup {
            return Ok(Prepared::Dedup(hash));
        }

        // Try to store as a patch against the head; fall back to a full snapshot if the head
        // can't be reconstructed (a previously corrupted chain) or the patch doesn't round-trip.
        // ponytail: verifying the patch here guarantees every stored version rebuilds, so a
        // corrupt chain can never reach a commit and brick it. The extra bspatch is cheap next
        // to the bsdiff we already ran.
        let patched = match &head {
            // Patching only pays for large, diff-friendly data (the .kra manifests). Small
            // streams (tiles) cost a chain-walk reconstruct + suffix-sort bsdiff to save a
            // couple of KB, and every later read walks the whole chain back; already-compressed
            // content (mergedimage.png etc.) yields patches near full size. Both go straight
            // to a 1-object zstd full: commits and reads become a single read + decode.
            // ponytail: patch-floor gate + magic sniff — tune here if storage ever matters more.
            _ if bytes.len() < opts.patch_floor || looks_compressed(bytes) => None,
            // Patch against the current head while under the chain threshold.
            Some(h) if h.chain_len + 1 <= max => match self.reconstruct(key, &h.hash) {
                Ok(base) => {
                    let mut patch = Vec::new();
                    Bsdiff::new(&base, bytes).compare(Cursor::new(&mut patch))?;
                    let mut check = Vec::new();
                    Bspatch::new(&patch)?.apply(&base, Cursor::new(&mut check))?;
                    if check == bytes {
                        // Name patches by (result, base): a patch is only valid against its base,
                        // so two streams reaching the same content from different bases can't collide.
                        let object = format!("{hash}.{}.patch", h.hash);
                        Some((
                            Version {
                                hash: hash.clone(),
                                base: Some(h.hash.clone()),
                                chain_len: h.chain_len + 1,
                            },
                            (object, patch),
                        ))
                    } else {
                        None
                    }
                }
                Err(_) => None,
            },
            _ => None,
        };

        let (version, object) = match patched {
            Some(v) => v,
            // First version, threshold reached, an unreconstructable head, or a non-round-tripping
            // patch -> fresh full snapshot (a full can never fail the integrity check).
            None => {
                let compressed = zstd::encode_all(bytes, opts.zstd_level)?;
                let object = format!("{hash}.full");
                (
                    Version {
                        hash: hash.clone(),
                        base: None,
                        chain_len: 0,
                    },
                    (object, compressed),
                )
            }
        };

        Ok(Prepared::New { version, object })
    }

    /// Apply a [`Prepared`] version: write its object (content-addressed, so idempotent) and push
    /// it onto the chain. Returns the content hash.
    pub fn commit_prepared(&mut self, key: &str, prepared: Prepared) -> Result<String> {
        match prepared {
            Prepared::Dedup(hash) => Ok(hash),
            Prepared::New { version, object } => {
                if !self.object_exists(&object.0) {
                    write_loose(&self.objects_dir(), &object.0, &object.1)?;
                }
                Ok(self.push_version(key.to_string(), version))
            }
        }
    }

    /// Apply many [`Prepared`] versions at once, then fold the chain pushes serially. Returns
    /// the content hashes in input order.
    ///
    /// Large batches (≥ [`PACK_MIN_OBJECTS`] distinct new objects — the whole-document first
    /// commit, a many-tile edit) write **one pack file** instead of one file per object:
    /// Windows charges every file *create* a per-file screening cost (Defender real-time
    /// scanning, worse for low-reputation binaries like a freshly installed app), which
    /// measured ~28s of a 33s initial large-canvas commit — parallelism doesn't help because
    /// the cost is in the create itself. Small batches keep loose per-object files (simple,
    /// and per-object dedup semantics stay byte-for-byte observable).
    pub fn commit_prepared_batch(&mut self, items: Vec<(String, Prepared)>) -> Result<Vec<String>> {
        use rayon::prelude::*;
        let objects = self.objects_dir();
        // Distinct new objects not already stored (loose or packed) — identical content under
        // two stream keys shares one object name, and re-commits dedup against disk.
        let mut seen = std::collections::HashSet::new();
        let candidates: Vec<&(String, Vec<u8>)> = items
            .iter()
            .filter_map(|(_, p)| match p {
                Prepared::New { object, .. } => Some(object),
                Prepared::Dedup(_) => None,
            })
            .filter(|o| seen.insert(o.0.as_str()))
            .collect();
        // Existence probes in parallel (thousands of serial stats hurt on cold HDDs), cheapest
        // first: the in-memory pack index, then the sharded loose path, then the legacy flat one.
        let pack_index = self.packs.index(&objects);
        let new_objs: Vec<&(String, Vec<u8>)> = candidates
            .into_par_iter()
            .filter(|o| {
                !pack_index.contains_key(&o.0)
                    && !objects.join(&o.0[..2]).join(&o.0).exists()
                    && !objects.join(&o.0).exists()
            })
            .collect();

        if new_objs.len() >= PACK_MIN_OBJECTS {
            self.packs.write_pack(&objects, &new_objs)?;
        } else {
            new_objs
                .par_iter()
                .try_for_each(|o| write_loose(&objects, &o.0, &o.1))?;
        }
        Ok(items
            .into_iter()
            .map(|(key, p)| match p {
                Prepared::Dedup(hash) => hash,
                Prepared::New { version, .. } => self.push_version(key, version),
            })
            .collect())
    }

    /// Whether `name` already exists in the store, loose (sharded or legacy flat) or packed.
    fn object_exists(&self, name: &str) -> bool {
        let objects = self.objects_dir();
        objects.join(&name[..2]).join(name).exists()
            || objects.join(name).exists()
            || self.packs.contains(&objects, name)
    }

    /// Read an object's raw bytes: loose sharded, legacy flat, then packs.
    fn read_object_bytes(&self, name: &str) -> Result<Vec<u8>> {
        let objects = self.objects_dir();
        match read_loose(&objects, name) {
            Err(KvcError::MissingObject(_)) => self.packs.read(&objects, name),
            other => other,
        }
    }

    fn push_version(&mut self, key: String, version: Version) -> String {
        let hash = version.hash.clone();
        self.chains.push(key, version);
        hash
    }

    /// Rebuild the exact bytes for version `hash` of `key`, walking the patch chain back
    /// to its full snapshot. Integrity is guaranteed at write time (every patch is
    /// round-trip-verified in `prepare_stream`, objects are content-addressed), so the
    /// read path skips re-hashing — it's the hottest loop in the visual diff.
    pub fn reconstruct(&self, key: &str, hash: &str) -> Result<Vec<u8>> {
        let chain = self
            .chains
            .chain(key)
            .ok_or_else(|| KvcError::NotTracked(key.to_string()))?;
        let v = chain
            .iter()
            .find(|x| x.hash == hash)
            .ok_or_else(|| KvcError::MissingObject(format!("{key}@{hash}")))?;

        let raw = self.read_object_bytes(&v.object_name())?;
        let bytes = match &v.base {
            None => zstd::decode_all(&raw[..])?,
            Some(base) => {
                let base_bytes = self.reconstruct(key, base)?;
                let mut out = Vec::new();
                Bspatch::new(&raw)?.apply(&base_bytes, Cursor::new(&mut out))?;
                out
            }
        };
        Ok(bytes)
    }
}

/// Read-through cache of reconstructed tile bytes, keyed by content hash and scoped to a
/// single diff request (no invalidation needed). The before/after sides of a modified layer
/// share most tiles, so each shared tile reconstructs once instead of twice.
pub struct TileCache(std::sync::Mutex<std::collections::HashMap<String, std::sync::Arc<Vec<u8>>>>);

impl TileCache {
    pub fn new() -> Self {
        Self(Default::default())
    }

    pub fn get_or_reconstruct(
        &self,
        repo: &Repo,
        key: &str,
        hash: &str,
    ) -> Result<std::sync::Arc<Vec<u8>>> {
        if let Some(v) = self.0.lock().unwrap().get(hash) {
            return Ok(v.clone());
        }
        // ponytail: racing threads may reconstruct the same hash twice — harmless, idempotent.
        let bytes = std::sync::Arc::new(repo.reconstruct(key, hash)?);
        self.0
            .lock()
            .unwrap()
            .insert(hash.to_string(), bytes.clone());
        Ok(bytes)
    }
}

/// Already-compressed payloads (PNG, zip, zstd) don't bsdiff usefully — patches come out
/// near full size while costing a suffix sort.
pub(crate) fn looks_compressed(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x89PNG")
        || bytes.starts_with(b"PK\x03\x04")
        || bytes.starts_with(&[0x28, 0xB5, 0x2F, 0xFD])
}

/// Content-addressed loose write into a 256-way sharded layout (`objects/<hash[..2]>/<name>`) —
/// a flat directory with 100k+ tiny files degrades NTFS lookups and amplifies Defender scans.
/// Names are hashes, so an existing file (sharded or legacy flat) is identical — skip it.
pub(crate) fn write_loose(objects: &Path, name: &str, data: &[u8]) -> Result<()> {
    let dir = objects.join(&name[..2]);
    let path = dir.join(name);
    if path.exists() || objects.join(name).exists() {
        return Ok(());
    }
    std::fs::create_dir_all(&dir).map_err(|e| io_at(&dir, e))?;
    std::fs::write(&path, data).map_err(|e| io_at(&path, e))
}

/// Read a loose object, preferring the sharded path; repos from before sharding keep their flat
/// objects readable forever (no migration needed — content-addressed files never change).
fn read_loose(objects: &Path, name: &str) -> Result<Vec<u8>> {
    let sharded = objects.join(&name[..2]).join(name);
    match std::fs::read(&sharded) {
        Ok(bytes) => Ok(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let flat = objects.join(name);
            std::fs::read(&flat).map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    KvcError::MissingObject(name.to_string())
                } else {
                    io_at(&flat, e)
                }
            })
        }
        Err(e) => Err(io_at(&sharded, e)),
    }
}

// --- pack files ---------------------------------------------------------------------------
// One commit's new objects, batched into a single file. Format:
// `KVCP1` | u32-LE index length | zstd(bincode(Vec<(name, rel_offset, len)>)) | payloads.
// Offsets are relative to the end of the index; the file is named by the blake3 of its index
// (which names every contained object), written temp-then-rename.

/// Batches below this stay loose — dedup behavior stays file-observable for small commits and
/// tests, and a pack of three tiles wouldn't pay for its indirection.
pub const PACK_MIN_OBJECTS: usize = 32;

const PACK_MAGIC: &[u8; 5] = b"KVCP1";

pub(crate) fn pack_dir(objects: &Path) -> std::path::PathBuf {
    objects.join("pack")
}

type PackIndex = std::collections::HashMap<String, (std::path::PathBuf, u64, u32)>;

/// Lazily-loaded index over every pack file: object name -> (pack path, absolute offset, len).
/// Interior mutability so reconstruct paths can fault it in from behind `&Repo` (rayon included).
/// The index is handed out as an `Arc` snapshot so parallel lookups (dedup filter, tile
/// reconstructs) never hold the mutex during their probes.
pub struct Packs(std::sync::Mutex<Option<std::sync::Arc<PackIndex>>>);

impl Default for Packs {
    fn default() -> Self {
        Packs(std::sync::Mutex::new(None))
    }
}

impl Packs {
    /// Snapshot of the loaded index (faulted in on first use). Lock held only for the clone.
    pub(crate) fn index(&self, objects: &Path) -> std::sync::Arc<PackIndex> {
        let mut guard = self.0.lock().unwrap();
        guard
            .get_or_insert_with(|| std::sync::Arc::new(load_pack_indexes(objects)))
            .clone()
    }

    pub(crate) fn contains(&self, objects: &Path, name: &str) -> bool {
        self.index(objects).contains_key(name)
    }

    /// Drop the loaded index (packs changed on disk — GC rewrites); rebuilt on next use.
    pub(crate) fn invalidate(&self) {
        *self.0.lock().unwrap() = None;
    }

    pub(crate) fn read(&self, objects: &Path, name: &str) -> Result<Vec<u8>> {
        let (path, off, len) = self
            .index(objects)
            .get(name)
            .cloned()
            .ok_or_else(|| KvcError::MissingObject(name.to_string()))?;
        read_exact_at(&path, off, len as usize)
    }

    /// Write `objs` as one pack file and register its entries in the loaded index.
    pub(crate) fn write_pack(&self, objects: &Path, objs: &[&(String, Vec<u8>)]) -> Result<()> {
        use std::io::Write;
        let index: Vec<(String, u64, u32)> = {
            let mut off = 0u64;
            objs.iter()
                .map(|(name, data)| {
                    let e = (name.clone(), off, data.len() as u32);
                    off += data.len() as u64;
                    e
                })
                .collect()
        };
        let idx_plain =
            bincode::serialize(&index).map_err(|e| KvcError::BadIndex(e.to_string()))?;
        let idx_bytes = zstd::encode_all(&idx_plain[..], 1)?;
        let pack_name = crate::repo::hash_bytes(&idx_bytes);

        let dir = pack_dir(objects);
        std::fs::create_dir_all(&dir).map_err(|e| io_at(&dir, e))?;
        let path = dir.join(format!("{pack_name}.pack"));
        if !path.exists() {
            let tmp = path.with_extension("tmp");
            {
                let file = std::fs::File::create(&tmp).map_err(|e| io_at(&tmp, e))?;
                let mut w = std::io::BufWriter::new(file);
                w.write_all(PACK_MAGIC)?;
                w.write_all(&(idx_bytes.len() as u32).to_le_bytes())?;
                w.write_all(&idx_bytes)?;
                for (_, data) in objs {
                    w.write_all(data)?;
                }
                w.into_inner().map_err(|e| KvcError::Io(e.into_error()))?;
            }
            std::fs::rename(&tmp, &path).map_err(|e| io_at(&path, e))?;
        }

        // Keep the in-memory index coherent for reads later in this session. `make_mut`
        // copy-on-writes if a reader still holds a snapshot Arc (stale snapshots are safe:
        // they just miss the objects this pack added, same as before it was written).
        let payload_base = (PACK_MAGIC.len() + 4 + idx_bytes.len()) as u64;
        {
            let mut guard = self.0.lock().unwrap();
            let arc = guard.get_or_insert_with(|| std::sync::Arc::new(load_pack_indexes(objects)));
            let idx = std::sync::Arc::make_mut(arc);
            for (name, off, len) in index {
                idx.insert(name, (path.clone(), payload_base + off, len));
            }
        }
        Ok(())
    }
}

/// Scan `objects/pack/*.pack` headers into one name -> location map. Corrupt or truncated
/// packs are skipped (their objects then read as missing, surfacing as `MissingObject`).
fn load_pack_indexes(objects: &Path) -> PackIndex {
    let mut map = PackIndex::new();
    let Ok(rd) = std::fs::read_dir(pack_dir(objects)) else {
        return map;
    };
    for e in rd.flatten() {
        let path = e.path();
        if path.extension().is_none_or(|x| x != "pack") {
            continue;
        }
        let Some(entries) = read_pack_header(&path) else {
            continue;
        };
        for (name, off, len) in entries {
            map.insert(name, (path.clone(), off, len));
        }
    }
    map
}

/// Parse one pack's header, returning entries with **absolute** file offsets.
pub(crate) fn read_pack_header(path: &Path) -> Option<Vec<(String, u64, u32)>> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut head = [0u8; 9];
    f.read_exact(&mut head).ok()?;
    if &head[..5] != PACK_MAGIC {
        return None;
    }
    let idx_len = u32::from_le_bytes(head[5..9].try_into().unwrap()) as usize;
    let mut idx_bytes = vec![0u8; idx_len];
    f.read_exact(&mut idx_bytes).ok()?;
    let plain = zstd::decode_all(&idx_bytes[..]).ok()?;
    let entries: Vec<(String, u64, u32)> = bincode::deserialize(&plain).ok()?;
    let base = (9 + idx_len) as u64;
    Some(
        entries
            .into_iter()
            .map(|(n, off, len)| (n, base + off, len))
            .collect(),
    )
}

/// Positional read of exactly `len` bytes at `off` — thread-safe (no shared seek cursor), so
/// parallel tile reconstructs can hit one pack concurrently.
pub(crate) fn read_exact_at(path: &Path, off: u64, len: usize) -> Result<Vec<u8>> {
    let f = std::fs::File::open(path).map_err(|e| io_at(path, e))?;
    let mut buf = vec![0u8; len];
    let mut read = 0usize;
    while read < len {
        #[cfg(windows)]
        let n = {
            use std::os::windows::fs::FileExt;
            f.seek_read(&mut buf[read..], off + read as u64)
                .map_err(|e| io_at(path, e))?
        };
        #[cfg(unix)]
        let n = {
            use std::os::unix::fs::FileExt;
            f.read_at(&mut buf[read..], off + read as u64)
                .map_err(|e| io_at(path, e))?
        };
        if n == 0 {
            return Err(KvcError::MissingObject(format!(
                "{} truncated at {off}+{read}",
                path.display()
            )));
        }
        read += n;
    }
    Ok(buf)
}

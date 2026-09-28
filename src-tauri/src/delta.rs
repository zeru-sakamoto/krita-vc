//! Content-addressed delta-chain storage. A "stream" is any versioned byte sequence
//! (a .kra manifest, an archive entry, or a single tile). Each new version
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

/// Longest patch chain a `.kra` manifest stream may grow before a full snapshot — see
/// [`Repo::prepare_stream_opts`].
pub const MANIFEST_CHAIN_MAX: usize = 5;

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
        // A manifest is a multi-megabyte JSON that every diff, restore and commit loads, and a
        // load replays every patch back to the last full snapshot: at the default 20 that was
        // 250 ms a load on a long history, three loads per Version Map node. Five caps a load at
        // five patches, for a full snapshot (a few MB compressed) every five versions instead of
        // every twenty — small next to the tiles a version stores.
        let max = if key.ends_with(":manifest") {
            self.config.delta_chain_max.min(MANIFEST_CHAIN_MAX)
        } else {
            self.config.delta_chain_max
        };

        let (dedup, head) = match self.chains.chain(key) {
            Some(v) => (v.iter().any(|x| x.hash == hash), v.last().cloned()),
            None => (false, None),
        };
        if dedup {
            return Ok(Prepared::Dedup(hash));
        }

        // Try to store as a patch against the head; fall back to a full snapshot if the head
        // can't be reconstructed (a previously corrupted chain) or the patch doesn't round-trip.
        // Verifying the patch here guarantees every stored version rebuilds, so a
        // corrupt chain can never reach a commit and brick it. The extra bspatch is cheap next
        // to the bsdiff we already ran.
        let patched = match &head {
            // Patching only pays for large, diff-friendly data (the .kra manifests). Small
            // streams (tiles) cost a chain-walk reconstruct + suffix-sort bsdiff to save a
            // couple of KB, and every later read walks the whole chain back; already-compressed
            // content (mergedimage.png etc.) yields patches near full size. Both go straight
            // to a 1-object zstd full: commits and reads become a single read + decode.
            // Patch-floor gate + magic sniff — tune here if storage ever matters more.
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
                    self.added_bytes += object.1.len() as u64;
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
        // first: the in-memory pack index, then the loose path.
        let pack_index = self.packs.index(&objects);
        let new_objs: Vec<&(String, Vec<u8>)> = candidates
            .into_par_iter()
            .filter(|o| !pack_index.contains_key(&o.0) && !loose_path(&objects, &o.0).exists())
            .collect();

        // What this commit actually adds — usually a few MB, however large the painting.
        let adding: u64 = new_objs.iter().map(|o| o.1.len() as u64).sum();
        crate::diskspace::check_available(&self.store, adding)?;
        self.added_bytes += adding;
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

    /// Whether `name` already exists in the store, packed or loose.
    pub(crate) fn object_exists(&self, name: &str) -> bool {
        let objects = self.objects_dir();
        self.packs.contains(&objects, name) || loose_path(&objects, name).exists()
    }

    /// Read an object's raw bytes. Packs first: any commit of 32 or more new objects is one, so
    /// after the first commit nearly every tile lives in a pack, and asking the in-memory index
    /// costs nothing — probing the loose path first cost a failed file open per packed read.
    fn read_object_bytes(&self, name: &str) -> Result<Vec<u8>> {
        let objects = self.objects_dir();
        match self.packs.read(&objects, name) {
            Err(KvcError::MissingObject(_)) => read_loose(&objects, name),
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
    /// read path skips re-hashing — it's the hottest loop in the visual diff. That reasoning
    /// covers engine bugs but not bit rot or a failing disk, so the paths that write the result
    /// into the working tree set [`Repo::verify_reads`] and pay one blake3 pass per object;
    /// recursion means the whole patch chain gets verified link by link.
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
        if self.verify_reads && crate::repo::hash_bytes(&bytes) != hash {
            return Err(KvcError::Corrupt(format!("{key}@{hash}")));
        }
        Ok(bytes)
    }

    /// [`Repo::reconstruct`] with a caller-owned memo keyed by content hash. Plain `reconstruct`
    /// replays the patch chain independently for every version, re-walking shared prefixes — so
    /// reconstructing every version of one stream is quadratic in chain length. Threading a memo
    /// across those calls rebuilds each version from its immediate predecessor exactly once,
    /// collapsing the whole run to linear. A content hash is a pure function of the bytes, so the
    /// memo is safe to share across stream keys (identical content dedups). Used by GC, the check
    /// and the storage report, which rebuild many versions in one pass.
    pub fn reconstruct_cached(
        &self,
        key: &str,
        hash: &str,
        memo: &mut ReconstructMemo,
    ) -> Result<std::sync::Arc<Vec<u8>>> {
        if let Some(bytes) = memo.get(hash) {
            return Ok(bytes);
        }
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
                let base_bytes = self.reconstruct_cached(key, base, memo)?;
                let mut out = Vec::new();
                Bspatch::new(&raw)?.apply(&base_bytes, Cursor::new(&mut out))?;
                out
            }
        };
        if self.verify_reads && crate::repo::hash_bytes(&bytes) != hash {
            return Err(KvcError::Corrupt(format!("{key}@{hash}")));
        }
        let bytes = std::sync::Arc::new(bytes);
        // Only a version something patches against is worth keeping. Every tile is a full
        // snapshot nothing builds on, and keeping those is how a scrub came to hold the whole
        // decompressed history.
        if chain.iter().any(|x| x.base.as_deref() == Some(hash)) {
            memo.put(hash, bytes.clone());
        }
        Ok(bytes)
    }
}

/// The memo [`Repo::reconstruct_cached`] threads through a pass over many versions: the last few
/// patch bases it rebuilt, least recently used out first, shared by `Arc` so a hit doesn't copy.
///
/// Bounded because it used to hold everything: marking 200 versions of a 45,000-tile painting kept
/// 975 MB of manifests, and a scrub kept the entire decompressed history. Four is plenty for a walk
/// in history order, where a version's base is almost always the one rebuilt just before it; out of
/// order, a miss costs one replay of a chain `delta_chain_max` long at most.
#[derive(Default)]
pub struct ReconstructMemo(std::collections::VecDeque<(String, std::sync::Arc<Vec<u8>>)>);

impl ReconstructMemo {
    const CAP: usize = 4;

    fn get(&mut self, hash: &str) -> Option<std::sync::Arc<Vec<u8>>> {
        let at = self.0.iter().position(|(h, _)| h == hash)?;
        let entry = self.0.remove(at)?;
        let bytes = entry.1.clone();
        self.0.push_back(entry);
        Some(bytes)
    }

    fn put(&mut self, hash: &str, bytes: std::sync::Arc<Vec<u8>>) {
        if self.0.len() == Self::CAP {
            self.0.pop_front();
        }
        self.0.push_back((hash.to_string(), bytes));
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
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
        // Racing threads may reconstruct the same hash twice — harmless, idempotent.
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

/// Where a loose object lives: a 256-way sharded layout (`objects/<hash[..2]>/<name>`) — a flat
/// directory with 100k+ tiny files degrades NTFS lookups and amplifies Defender scans.
fn loose_path(objects: &Path, name: &str) -> std::path::PathBuf {
    objects.join(&name[..2]).join(name)
}

/// Content-addressed loose write. Names are hashes, so an existing file is identical — skip it.
pub(crate) fn write_loose(objects: &Path, name: &str, data: &[u8]) -> Result<()> {
    let path = loose_path(objects, name);
    let dir = objects.join(&name[..2]);
    if path.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(&dir).map_err(|e| io_at(&dir, e))?;
    // Temp-then-rename, because the dedup above trusts *existence*: a plain write interrupted by
    // a crash would leave a truncated file under a name that claims a hash it doesn't have, and
    // every later commit storing that content would skip the write and trust it forever. A crash
    // leftover is swept by GC, which deletes anything in `objects/` it can't name.
    //
    // And fsynced before the rename. `save()` fsyncs the chain shard and log line that name this
    // object, and an fsync makes only *its own* file durable: NTFS journals the rename and the
    // length but not the contents, so after a power cut the reference could survive and the
    // object come back as zeros, taking the newest version with it. At most `PACK_MIN_OBJECTS`
    // of these per commit; anything bigger is one pack.
    let tmp = dir.join(format!("{name}.tmp"));
    if let Err(e) = crate::repo::sync_write(&tmp, data) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    crate::repo::rename_retrying(&tmp, &path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        io_at(&path, e)
    })?;
    crate::repo::sync_parent_dir(&path);
    Ok(())
}

/// Read a loose object.
fn read_loose(objects: &Path, name: &str) -> Result<Vec<u8>> {
    let path = loose_path(objects, name);
    std::fs::read(&path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            KvcError::MissingObject(name.to_string())
        } else {
            io_at(&path, e)
        }
    })
}

// --- pack files ---------------------------------------------------------------------------
// One commit's new objects, batched into a single file. Format:
// `KVCP2` | u32-LE index length | zstd(bincode(Vec<(name, rel_offset, len)>)) | u64-LE body
// length | payloads. Offsets are relative to the end of the index; the file is named by the
// blake3 of its index (which names every contained object), written temp-then-rename.
//
// `body_len` is a self-check: it lets a truncated pack (interrupted copy, bad backup restore)
// be recognized as corrupt at header-parse time instead of surfacing as garbage bytes — or a
// `MissingObject` for every entry after it — when something later tries to read out of it.

/// Batches below this stay loose — dedup behavior stays file-observable for small commits and
/// tests, and a pack of three tiles wouldn't pay for its indirection.
pub const PACK_MIN_OBJECTS: usize = 32;

const PACK_MAGIC: &[u8; 5] = b"KVCP2";

pub(crate) fn pack_dir(objects: &Path) -> std::path::PathBuf {
    objects.join("pack")
}

/// One pack, open for reading. Every object read out of it goes through this one handle,
/// positionally (`FileExt`'s positional reads share no cursor, so parallel reads are fine):
/// reopening the pack per object was 42 µs of a packed read's 88 µs. Held only as long as the
/// `Repo` whose index holds it; Rust opens files with delete sharing on Windows, so a cleanup can
/// still move a pack aside while one is open.
pub(crate) struct PackFile {
    path: std::path::PathBuf,
    file: std::fs::File,
}

type PackIndex = std::collections::HashMap<String, (std::sync::Arc<PackFile>, u64, u32)>;

/// Lazily-loaded index over every pack file: object name -> (pack, absolute offset, len).
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
        let (pack, off, len) = self
            .index(objects)
            .get(name)
            .cloned()
            .ok_or_else(|| KvcError::MissingObject(name.to_string()))?;
        read_exact_at(&pack.file, &pack.path, off, len as usize)
    }

    /// Write `objs` as one pack file and register its entries in the loaded index.
    pub(crate) fn write_pack(&self, objects: &Path, objs: &[&(String, Vec<u8>)]) -> Result<()> {
        use std::io::Write;
        // The index records lengths as u32; an object of 4 GiB or more fails the commit instead of
        // being silently truncated in it.
        let too_big = |len: usize| {
            u32::try_from(len)
                .map_err(|_| KvcError::BadIndex(format!("{len}-byte object is too big to pack")))
        };
        let index: Vec<(String, u64, u32)> = {
            let mut off = 0u64;
            objs.iter()
                .map(|(name, data)| {
                    let e = (name.clone(), off, too_big(data.len())?);
                    off += data.len() as u64;
                    Ok(e)
                })
                .collect::<Result<_>>()?
        };
        let idx_plain =
            bincode::serialize(&index).map_err(|e| KvcError::BadIndex(e.to_string()))?;
        let idx_bytes = zstd::encode_all(&idx_plain[..], 1)?;
        let pack_name = crate::repo::hash_bytes(&idx_bytes);
        let body_len: u64 = objs.iter().map(|(_, data)| data.len() as u64).sum();

        let dir = pack_dir(objects);
        std::fs::create_dir_all(&dir).map_err(|e| io_at(&dir, e))?;
        let path = dir.join(format!("{pack_name}.pack"));
        if !path.exists() {
            let tmp = path.with_extension("tmp");
            let at_tmp = |e| io_at(&tmp, e);
            {
                let file = std::fs::File::create(&tmp).map_err(at_tmp)?;
                let mut w = std::io::BufWriter::new(file);
                w.write_all(PACK_MAGIC).map_err(at_tmp)?;
                w.write_all(&too_big(idx_bytes.len())?.to_le_bytes())
                    .map_err(at_tmp)?;
                w.write_all(&idx_bytes).map_err(at_tmp)?;
                w.write_all(&body_len.to_le_bytes()).map_err(at_tmp)?;
                for (_, data) in objs {
                    w.write_all(data).map_err(at_tmp)?;
                }
                // Fsynced before the rename, for the reason `write_loose` gives: the chain shard
                // and log that name these objects are fsynced right after, and must never
                // outlive them. One fsync per large commit, on data that has to reach the disk.
                let file = w.into_inner().map_err(|e| at_tmp(e.into_error()))?;
                file.sync_all().map_err(at_tmp)?;
            }
            crate::repo::rename_retrying(&tmp, &path).map_err(|e| io_at(&path, e))?;
            crate::repo::sync_parent_dir(&path);
        }

        // Keep the in-memory index coherent for reads later in this session. `make_mut`
        // copy-on-writes if a reader still holds a snapshot Arc (stale snapshots are safe:
        // they just miss the objects this pack added, same as before it was written).
        let payload_base = (PACK_MAGIC.len() + 4 + idx_bytes.len() + 8) as u64;
        let pack = std::sync::Arc::new(PackFile {
            file: std::fs::File::open(&path).map_err(|e| io_at(&path, e))?,
            path,
        });
        {
            let mut guard = self.0.lock().unwrap();
            let arc = guard.get_or_insert_with(|| std::sync::Arc::new(load_pack_indexes(objects)));
            let idx = std::sync::Arc::make_mut(arc);
            for (name, off, len) in index {
                idx.insert(name, (pack.clone(), payload_base + off, len));
            }
        }
        Ok(())
    }
}

/// Scan `objects/pack/*.pack` headers into one name -> location map, keeping each pack open for
/// the reads to come. Corrupt or truncated packs are skipped (their objects then read as missing,
/// surfacing as `MissingObject`).
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
        let Ok(mut file) = std::fs::File::open(&path) else {
            continue;
        };
        let Some(entries) = pack_header(&mut file) else {
            continue;
        };
        let pack = std::sync::Arc::new(PackFile { path, file });
        for (name, off, len) in entries {
            map.insert(name, (pack.clone(), off, len));
        }
    }
    map
}

/// Parse one pack's header, returning entries with **absolute** file offsets. The declared body
/// length is checked against the file's real length — a truncated pack is rejected here (skipped,
/// same as any other unparseable header) rather than surfacing later as `MissingObject`/garbage
/// bytes for whatever it happened to still contain.
pub(crate) fn read_pack_header(path: &Path) -> Option<Vec<(String, u64, u32)>> {
    pack_header(&mut std::fs::File::open(path).ok()?)
}

fn pack_header(f: &mut std::fs::File) -> Option<Vec<(String, u64, u32)>> {
    use std::io::Read;
    let mut head = [0u8; 9];
    f.read_exact(&mut head).ok()?;
    if &head[..5] != PACK_MAGIC {
        return None;
    }
    let idx_len = u32::from_le_bytes(head[5..9].try_into().unwrap()) as usize;
    let file_len = f.metadata().ok()?.len();
    // A corrupt header could claim a multi-GB index; the index bytes live in this same file, so
    // idx_len can never legitimately exceed the file's length — reject rather than pre-allocate.
    if idx_len as u64 > file_len {
        return None;
    }
    let mut idx_bytes = vec![0u8; idx_len];
    f.read_exact(&mut idx_bytes).ok()?;
    let plain = zstd::decode_all(&idx_bytes[..]).ok()?;
    let entries: Vec<(String, u64, u32)> = bincode::deserialize(&plain).ok()?;
    let mut body_len_bytes = [0u8; 8];
    f.read_exact(&mut body_len_bytes).ok()?;
    let body_len = u64::from_le_bytes(body_len_bytes);
    let header_len = 9 + idx_len as u64 + 8;
    if file_len.checked_sub(header_len) != Some(body_len) {
        return None;
    }
    Some(
        entries
            .into_iter()
            .map(|(n, off, len)| (n, header_len + off, len))
            .collect(),
    )
}

/// Positional read of exactly `len` bytes at `off` of `f` (opened from `path`, which only names
/// it in errors) — thread-safe (no shared seek cursor), so parallel tile reconstructs can hit one
/// pack concurrently.
pub(crate) fn read_exact_at(
    f: &std::fs::File,
    path: &Path,
    off: u64,
    len: usize,
) -> Result<Vec<u8>> {
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

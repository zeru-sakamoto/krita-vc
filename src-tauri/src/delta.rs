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
        let hash = hash_bytes(bytes);
        let max = self.config.delta_chain_max;

        let (dedup, head) = match self.chains.0.get(key) {
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
            // ponytail: 64 KB gate + magic sniff — tune here if storage ever matters more.
            _ if bytes.len() < 64 * 1024 || looks_compressed(bytes) => None,
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
                                object: object.clone(),
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
                let compressed = zstd::encode_all(bytes, 3)?;
                let object = format!("{hash}.full");
                (
                    Version {
                        hash: hash.clone(),
                        object: object.clone(),
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
                let objects = self.objects_dir();
                write_object(&objects, &object.0, &object.1)?;
                Ok(self.push_version(key.to_string(), version))
            }
        }
    }

    /// Apply many [`Prepared`] versions at once: all new objects are written **in parallel**
    /// (content-addressed writes are independent and idempotent — on Windows especially,
    /// thousands of serial small-file creates were a dominant commit cost), then the chain
    /// pushes fold serially. Returns the content hashes in input order.
    pub fn commit_prepared_batch(&mut self, items: Vec<(String, Prepared)>) -> Result<Vec<String>> {
        use rayon::prelude::*;
        let objects = self.objects_dir();
        items.par_iter().try_for_each(|(_, p)| match p {
            Prepared::New { object, .. } => write_object(&objects, &object.0, &object.1),
            Prepared::Dedup(_) => Ok(()),
        })?;
        Ok(items
            .into_iter()
            .map(|(key, p)| match p {
                Prepared::Dedup(hash) => hash,
                Prepared::New { version, .. } => self.push_version(key, version),
            })
            .collect())
    }

    fn push_version(&mut self, key: String, version: Version) -> String {
        self.chains_dirty = true;
        let hash = version.hash.clone();
        self.chains.0.entry(key).or_default().push(version);
        hash
    }

    /// Rebuild the exact bytes for version `hash` of `key`, walking the patch chain back
    /// to its full snapshot. Integrity is guaranteed at write time (every patch is
    /// round-trip-verified in `prepare_stream`, objects are content-addressed), so the
    /// read path skips re-hashing — it's the hottest loop in the visual diff.
    pub fn reconstruct(&self, key: &str, hash: &str) -> Result<Vec<u8>> {
        let chain = self
            .chains
            .0
            .get(key)
            .ok_or_else(|| KvcError::NotTracked(key.to_string()))?;
        let v = chain
            .iter()
            .find(|x| x.hash == hash)
            .ok_or_else(|| KvcError::MissingObject(format!("{key}@{hash}")))?;

        let objects = self.objects_dir();
        let raw = read_object(&objects, &v.object)?;
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

/// Content-addressed write: object names are hashes, so an existing file is identical — skip it.
fn write_object(objects: &Path, name: &str, data: &[u8]) -> Result<()> {
    let path = objects.join(name);
    if path.exists() {
        return Ok(());
    }
    std::fs::write(&path, data).map_err(|e| io_at(&path, e))
}

fn read_object(objects: &Path, name: &str) -> Result<Vec<u8>> {
    let path = objects.join(name);
    std::fs::read(&path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            KvcError::MissingObject(name.to_string())
        } else {
            io_at(&path, e)
        }
    })
}

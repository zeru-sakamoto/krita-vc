//! Content-addressed delta-chain storage. A "stream" is any versioned byte sequence
//! (a generic file, a .kra manifest, a layer entry, or a single tile). Each new version
//! is stored either as a full zstd snapshot or a bsdiff patch against the previous head;
//! a configurable threshold caps consecutive patches so restores stay fast.

use crate::error::{io_at, KvcError, Result};
use crate::repo::{hash_bytes, Repo, Version};
use qbsdiff::{Bsdiff, Bspatch};
use std::io::Cursor;
use std::path::Path;

impl Repo {
    /// Store `bytes` as the next version of `key`. Returns the content hash. Identical
    /// content already in the chain is deduplicated (no new object, no new version).
    pub fn store_stream(&mut self, key: &str, bytes: &[u8]) -> Result<String> {
        let hash = hash_bytes(bytes);
        let objects = self.objects_dir();
        let max = self.config.delta_chain_max;

        let (dedup, head) = match self.chains.0.get(key) {
            Some(v) => (v.iter().any(|x| x.hash == hash), v.last().cloned()),
            None => (false, None),
        };
        if dedup {
            return Ok(hash);
        }

        // Try to store as a patch against the head; fall back to a full snapshot if the head
        // can't be reconstructed (a previously corrupted chain) or the patch doesn't round-trip.
        // ponytail: verifying the patch here guarantees every stored version rebuilds, so a
        // corrupt chain can never reach a commit and brick it. The extra bspatch is cheap next
        // to the bsdiff we already ran.
        let patched = match &head {
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
                        write_object(&objects, &object, &patch)?;
                        Some(Version {
                            hash: hash.clone(),
                            object,
                            base: Some(h.hash.clone()),
                            chain_len: h.chain_len + 1,
                        })
                    } else {
                        None
                    }
                }
                Err(_) => None,
            },
            _ => None,
        };

        let version = match patched {
            Some(v) => v,
            // First version, threshold reached, an unreconstructable head, or a non-round-tripping
            // patch -> fresh full snapshot (a full can never fail the integrity check).
            None => {
                let compressed = zstd::encode_all(bytes, 3)?;
                let object = format!("{hash}.full");
                write_object(&objects, &object, &compressed)?;
                Version {
                    hash: hash.clone(),
                    object,
                    base: None,
                    chain_len: 0,
                }
            }
        };

        self.chains
            .0
            .entry(key.to_string())
            .or_default()
            .push(version);
        Ok(hash)
    }

    /// Rebuild the exact bytes for version `hash` of `key`, walking the patch chain back
    /// to its full snapshot. Verifies the reconstruction against `hash` (integrity guard).
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

        if hash_bytes(&bytes) != hash {
            return Err(KvcError::MissingObject(format!("{key}@{hash} (integrity)")));
        }
        Ok(bytes)
    }
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

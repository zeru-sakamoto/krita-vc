//! `.kvc/ops.log` — an append-only record of destructive operations (undo, discard, cleanup,
//! branch delete). None of these are bugs to guard against; they're exactly what the artist
//! asked for. But they leave no trace elsewhere, so "my work disappeared" has nothing to
//! reconstruct the sequence from. This is that record — support/recovery only, no UI reads it.
//!
//! Same JSON-lines append shape as `commits.log` (`Repo::flush_commits`), and for the same
//! reason: `OpenOptions::append` + `sync_all` so a torn write is always the *last* line, never a
//! corrupted one earlier in the file. Rotation is size-capped, truncate-oldest — mutating ops
//! are rare (never per-commit), so a 2 MB cap is effectively years of activity, and the rewrite
//! it occasionally costs is trivial next to the operation that triggered it.

use crate::error::{io_at, KvcError, Result};
use crate::repo::{kvc_dir, write_file_atomic, Repo};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Past this size, the next append rotates the file down to its newest half first.
const MAX_BYTES: u64 = 2 << 20;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpsLogEntry {
    ts_ms: u64,
    op: String,
    branch: String,
    tip_before: Option<String>,
    tip_after: Option<String>,
    detail: Option<String>,
}

fn ops_log_path(root: &Path) -> PathBuf {
    kvc_dir(root).join("ops.log")
}

/// Append one record. Best-effort in spirit (a failure here shouldn't fail the operation it's
/// describing), but the caller decides that — this returns `Result` like every other engine
/// call so a lock/IO problem isn't silently swallowed.
pub fn append(
    repo: &Repo,
    op: &str,
    tip_before: Option<&str>,
    tip_after: Option<&str>,
    detail: Option<String>,
) -> Result<()> {
    let path = ops_log_path(&repo.root);
    rotate_if_needed(&path)?;

    let entry = OpsLogEntry {
        ts_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
        op: op.to_string(),
        branch: repo.branches.current.clone(),
        tip_before: tip_before.map(str::to_string),
        tip_after: tip_after.map(str::to_string),
        detail,
    };
    let mut line = serde_json::to_vec(&entry).map_err(|e| KvcError::BadIndex(e.to_string()))?;
    line.push(b'\n');

    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| io_at(&path, e))?;
    f.write_all(&line).map_err(|e| io_at(&path, e))?;
    f.sync_all().map_err(|e| io_at(&path, e))
}

/// If `path` is already at/over [`MAX_BYTES`], keep only its newest half of lines. Absent or
/// under-cap: no-op, so the common append path never pays for a read.
fn rotate_if_needed(path: &Path) -> Result<()> {
    let len = match std::fs::metadata(path) {
        Ok(m) => m.len(),
        Err(_) => return Ok(()), // doesn't exist yet — nothing to rotate
    };
    if len < MAX_BYTES {
        return Ok(());
    }
    let text = std::fs::read_to_string(path).map_err(|e| io_at(path, e))?;
    let lines: Vec<&str> = text.lines().filter(|l| !l.is_empty()).collect();
    let keep = &lines[lines.len() / 2..];
    let mut bytes = keep.join("\n").into_bytes();
    if !bytes.is_empty() {
        bytes.push(b'\n');
    }
    write_file_atomic(path, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::Repo;

    fn repo(dir: &Path) -> Repo {
        Repo::init(dir).unwrap();
        Repo::open(dir).unwrap()
    }

    #[test]
    fn appends_one_line_per_call() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = repo(tmp.path());
        append(&repo, "undo", Some("a"), Some("b"), None).unwrap();
        append(&repo, "discard", Some("b"), Some("b"), None).unwrap();
        let text = std::fs::read_to_string(ops_log_path(&repo.root)).unwrap();
        assert_eq!(text.lines().count(), 2);
        assert!(text.lines().next().unwrap().contains("\"op\":\"undo\""));
    }

    #[test]
    fn rotation_keeps_only_the_newest_half_once_over_the_cap() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = repo(tmp.path());
        let path = ops_log_path(&repo.root);
        // Fabricate an over-cap log directly rather than appending real entries until 2MB —
        // each line is padded to a known size so the total comfortably clears MAX_BYTES.
        let line = format!("{{\"op\":\"filler\",\"pad\":\"{}\"}}\n", "x".repeat(80));
        let count = (MAX_BYTES as usize / line.len()) + 1000;
        let mut oversized = String::with_capacity(line.len() * count);
        for i in 0..count {
            oversized.push_str(&format!(
                "{{\"op\":\"filler-{i}\",\"pad\":\"{}\"}}\n",
                "x".repeat(80)
            ));
        }
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &oversized).unwrap();
        assert!(std::fs::metadata(&path).unwrap().len() >= MAX_BYTES);

        append(&repo, "cleanup", None, None, Some("freed 10".into())).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert!(
            lines.len() < count + 1,
            "rotation should have dropped the oldest half"
        );
        assert!(
            !lines.first().unwrap().contains("filler-0\""),
            "oldest entries should be gone"
        );
        assert!(lines.last().unwrap().contains("\"op\":\"cleanup\""));
    }
}

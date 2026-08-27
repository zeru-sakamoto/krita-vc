//! Cheap free-space precheck before a write-heavy operation (commit/switch/rollback). Atomic
//! working-tree writes already made a mid-write out-of-disk failure *safe* (the artist keeps
//! the old file, see `repo::write_file_atomic`) — this just turns it into a clear error raised
//! *before* anything is touched, instead of a mid-operation IO error partway through a batch of
//! files.
//!
//! Deliberately a simple worst-case estimate, not a precise per-object accounting: `needed * 2`
//! covers `write_file_atomic`'s brief doubling (temp file written before the old one is replaced),
//! and callers sum the same size fields they already have on hand (`ScanChange::size`,
//! `CommittedFile::original_size`) rather than computing anything new.

use crate::error::{KvcError, Result};
use crate::repo::Repo;
use std::path::Path;

/// Free bytes on the volume containing `path`, or `None` if that can't be determined (an
/// unsupported platform, or the syscall failing) — callers treat `None` as "skip the check"
/// rather than blocking an operation the check itself couldn't evaluate.
#[cfg(windows)]
pub fn free_bytes(path: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    // The API wants a drive/directory prefix, not necessarily an existing file; the repo root
    // always exists, so this is safe as-is.
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    wide.push(0);
    let mut free_available = 0u64;
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut free_available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        None
    } else {
        Some(free_available)
    }
}

#[cfg(not(windows))]
pub fn free_bytes(_path: &Path) -> Option<u64> {
    None
}

/// Refuse up front if the volume holding the **store** doesn't look like it has room for
/// `needed_bytes` (doubled — see module docs). The store, not `repo.root`: these bytes are
/// objects and chains, and a custom store root can put them on a different drive than the
/// artwork. A `None` from [`free_bytes`] always passes: the check is a belt-and-suspenders
/// precaution, not a hard dependency of any write.
pub fn check_available(repo: &Repo, needed_bytes: u64) -> Result<()> {
    evaluate(free_bytes(&repo.store), needed_bytes)
}

/// The comparison itself, split out from the syscall so it's unit-testable without faking a
/// real volume.
fn evaluate(available: Option<u64>, needed_bytes: u64) -> Result<()> {
    let Some(available) = available else {
        return Ok(());
    };
    let needed = needed_bytes.saturating_mul(2);
    if available < needed {
        return Err(KvcError::InsufficientDiskSpace { needed, available });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_bytes_on_current_dir_is_plausible() {
        // Smoke test only — the real value is machine-dependent. On an unsupported platform
        // this is `None`, which is also a valid pass.
        if let Some(bytes) = free_bytes(Path::new(".")) {
            assert!(bytes > 0);
        }
    }

    #[test]
    fn refuses_when_available_is_below_double_the_need() {
        let err = evaluate(Some(100), 60).unwrap_err();
        assert!(matches!(
            err,
            KvcError::InsufficientDiskSpace {
                needed: 120,
                available: 100
            }
        ));
    }

    #[test]
    fn passes_when_available_covers_double_the_need() {
        assert!(evaluate(Some(200), 60).is_ok());
    }

    #[test]
    fn skips_the_check_when_free_space_is_unknown() {
        assert!(evaluate(None, u64::MAX).is_ok());
    }
}

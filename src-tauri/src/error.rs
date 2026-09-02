//! Typed error boundaries for the .kvc engine. Every fallible engine call returns
//! `Result<_, KvcError>`; Tauri commands convert to `String` for the frontend.

use std::io;
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum KvcError {
    #[error("not tracked yet: {0}")]
    NotARepo(PathBuf),

    #[error("already tracked: {0}")]
    AlreadyRepo(PathBuf),

    #[error("only Krita documents (.kra) can be tracked: {0}")]
    Unsupported(PathBuf),

    // Distinct from `NotARepo` on purpose, and the distinction is load-bearing: `NotARepo` means
    // "never versioned", which the UI answers by offering to start tracking. Answering it that
    // way for a document whose history is merely on a drive that isn't plugged in would create an
    // empty store and orphan every version the artist saved. The "history isn't reachable" prefix
    // is matched by the frontend — keep it stable.
    #[error("history isn't reachable: the version history is kept in {0}, which isn't available right now")]
    StoreUnreachable(PathBuf),

    #[error("corrupted or unreadable .kra archive: {0}")]
    CorruptZip(String),

    #[error("malformed Krita tile block: {0}")]
    BadTiles(String),

    #[error("stored object missing from objects/: {0}")]
    MissingObject(String),

    #[error("corrupted repository index: {0}")]
    BadIndex(String),

    // A stored object rebuilt to bytes that don't hash to the name it was filed under — bit rot,
    // a failing disk, or something outside the app editing `.kvc/`. Distinct from `MissingObject`:
    // the data is there, it's just not what it claims to be.
    #[error("corrupted stored object: {0}")]
    Corrupt(String),

    #[error("permission denied accessing {0}")]
    PermissionDenied(PathBuf),

    #[error("file not tracked: {0}")]
    NotTracked(String),

    #[error("nothing to commit")]
    Nothing,

    #[error("no such commit: {0}")]
    NoCommit(String),

    // The "unsaved changes" prefix is matched by the frontend to show a friendly
    // save-first prompt — keep it stable.
    #[error("unsaved changes: save or discard your work before switching branches")]
    DirtyTree,

    // Deliberately does NOT start with "unsaved changes" — that prefix is matched above for the
    // branch-switch prompt, and a stash conflict needs its own frontend dialog.
    #[error("stash conflict: {0} changed since you set this aside — save or discard first")]
    StashConflict(String),

    #[error("no such stash: {0}")]
    NoStash(String),

    // A .kra layer merge (bringing a set-aside change back onto edited work) couldn't be done
    // cleanly — the pop leaves the working tree and the stash untouched rather than write a file
    // Krita can't open.
    #[error("couldn't merge set-aside work: {0}")]
    MergeFailed(String),

    // Synthesizing the .kra for a layer-subset commit (`stage::stage_kra`) couldn't be done
    // cleanly. Same contract as MergeFailed: nothing is written, so the working tree and the
    // history are both untouched. Its own variant because the artist here picked layers to save
    // and has never heard of a set-aside.
    #[error("couldn't save just those layers: {0}")]
    StageFailed(String),

    #[error("no such branch: {0}")]
    NoBranch(String),

    #[error("branch already exists: {0}")]
    BranchExists(String),

    #[error("invalid branch name: {0}")]
    BadBranchName(String),

    #[error("nothing to merge: {0}")]
    NothingToMerge(String),

    #[error("cannot delete the branch you are working on")]
    DeleteCurrent,

    #[error("cannot delete the main branch")]
    DeleteMain,

    #[error("cannot undo: {0}")]
    CannotUndo(String),

    #[error("unsafe path outside the repository: {0}")]
    BadPath(String),

    #[error("repository is busy (locked by another process): {0}")]
    Locked(String),

    #[error("not enough free disk space: this needs about {needed} bytes, {available} available")]
    InsufficientDiskSpace { needed: u64, available: u64 },

    #[error(transparent)]
    Io(#[from] io::Error),
}

pub type Result<T> = std::result::Result<T, KvcError>;

/// Map an IO error against a path, promoting permission failures to a clearer variant.
pub fn io_at(path: &std::path::Path, e: io::Error) -> KvcError {
    if e.kind() == io::ErrorKind::PermissionDenied {
        KvcError::PermissionDenied(path.to_path_buf())
    } else {
        KvcError::Io(e)
    }
}

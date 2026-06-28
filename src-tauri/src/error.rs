//! Typed error boundaries for the .kvc engine. Every fallible engine call returns
//! `Result<_, KvcError>`; Tauri commands convert to `String` for the frontend.

use std::io;
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum KvcError {
    #[error("not a .kvc repository: {0}")]
    NotARepo(PathBuf),

    #[error("a .kvc repository already exists at {0}")]
    AlreadyRepo(PathBuf),

    #[error("corrupted or unreadable .kra archive: {0}")]
    CorruptZip(String),

    #[error("malformed Krita tile block: {0}")]
    BadTiles(String),

    #[error("stored object missing from objects/: {0}")]
    MissingObject(String),

    #[error("corrupted repository index: {0}")]
    BadIndex(String),

    #[error("permission denied accessing {0}")]
    PermissionDenied(PathBuf),

    #[error("file not tracked: {0}")]
    NotTracked(String),

    #[error("nothing to commit")]
    Nothing,

    #[error("no such commit: {0}")]
    NoCommit(String),

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

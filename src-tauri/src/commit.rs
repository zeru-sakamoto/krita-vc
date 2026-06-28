//! Commit orchestration and file restoration. `commit_snapshot` scans the working tree,
//! routes each change through the .kra engine or the generic blob store, records a commit,
//! and flushes state. `file_at_commit` rebuilds a file's exact bytes from any commit.

use crate::error::{io_at, KvcError, Result};
use crate::repo::{hash_bytes, Commit, CommittedFile, Repo, TrackedFile};
use crate::{kra, scan};

/// Commit every working-tree change. Returns the new commit (or `Nothing` if clean).
pub fn commit_snapshot(repo: &mut Repo, message: &str, author: &str) -> Result<Commit> {
    let changes = scan::scan(repo)?;
    if changes.is_empty() {
        return Err(KvcError::Nothing);
    }

    let mut files = Vec::new();
    for (rel, status) in changes {
        if status == "D" {
            repo.index.files.remove(&rel);
            files.push(CommittedFile {
                path: rel,
                status: "D".into(),
                content: None,
                is_kra: false,
            });
            continue;
        }

        let abs = repo.root.join(&rel);
        let bytes = std::fs::read(&abs).map_err(|e| io_at(&abs, e))?;
        let is_kra = rel.to_lowercase().ends_with(".kra");

        let content = if is_kra {
            kra::commit_kra(repo, &rel, &bytes)?
        } else {
            repo.store_stream(&format!("file:{rel}"), &bytes)?
        };

        repo.index
            .files
            .insert(rel.clone(), TrackedFile { hash: hash_bytes(&bytes), is_kra });

        files.push(CommittedFile {
            path: rel,
            status: if status == "U" { "A".into() } else { "M".into() },
            content: Some(content),
            is_kra,
        });
    }

    let timestamp = crate::repo::now_iso();
    let parents: Vec<String> = repo.commits.last().map(|c| vec![c.id.clone()]).unwrap_or_default();
    let id = commit_id(&timestamp, message, &files);

    let commit = Commit {
        id: id.clone(),
        hash: id,
        message: message.to_string(),
        author: author.to_string(),
        timestamp,
        parents,
        files,
    };
    repo.commits.push(commit.clone());
    repo.save()?;
    Ok(commit)
}

/// Reconstruct the exact bytes of `relpath` as recorded in commit `commit_id`.
pub fn file_at_commit(repo: &Repo, relpath: &str, commit_id: &str) -> Result<Vec<u8>> {
    let commit = repo
        .commits
        .iter()
        .find(|c| c.id == commit_id)
        .ok_or_else(|| KvcError::NoCommit(commit_id.to_string()))?;
    let file = commit
        .files
        .iter()
        .find(|f| f.path == relpath)
        .ok_or_else(|| KvcError::NotTracked(relpath.to_string()))?;
    let content = file
        .content
        .as_ref()
        .ok_or_else(|| KvcError::NotTracked(format!("{relpath} (deleted in this commit)")))?;

    if file.is_kra {
        kra::reconstruct_kra(repo, relpath, content)
    } else {
        repo.reconstruct(&format!("file:{relpath}"), content)
    }
}

fn commit_id(timestamp: &str, message: &str, files: &[CommittedFile]) -> String {
    let mut seed = format!("{timestamp}\n{message}\n");
    for f in files {
        seed.push_str(&format!("{}:{}\n", f.path, f.content.as_deref().unwrap_or("-")));
    }
    hash_bytes(seed.as_bytes())[..12].to_string()
}

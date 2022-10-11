use crate::error::OxenError;
use crate::index::{CommitReader, RefReader};
use crate::model::{Commit, LocalRepository};

pub fn get_by_id(repo: &LocalRepository, commit_id: &str) -> Result<Option<Commit>, OxenError> {
    let reader = CommitReader::new(repo)?;
    reader.get_commit_by_id(commit_id)
}

pub fn get_by_id_or_branch(
    repo: &LocalRepository,
    branch_or_commit: &str,
) -> Result<Option<Commit>, OxenError> {
    let ref_reader = RefReader::new(repo)?;
    let commit_id = match ref_reader.get_commit_id_for_branch(branch_or_commit)? {
        Some(branch_commit_id) => branch_commit_id,
        None => String::from(branch_or_commit),
    };
    let reader = CommitReader::new(repo)?;
    reader.get_commit_by_id(commit_id)
}

pub fn get_head_commit(repo: &LocalRepository) -> Result<Commit, OxenError> {
    let committer = CommitReader::new(repo)?;
    committer.head_commit()
}

pub fn get_parents(repo: &LocalRepository, commit: &Commit) -> Result<Vec<Commit>, OxenError> {
    let committer = CommitReader::new(repo)?;
    let mut commits: Vec<Commit> = vec![];
    for commit_id in commit.parent_ids.iter() {
        if let Some(commit) = committer.get_commit_by_id(commit_id)? {
            commits.push(commit)
        } else {
            return Err(OxenError::commit_db_corrupted(commit_id));
        }
    }
    Ok(commits)
}

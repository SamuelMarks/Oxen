//! goes through the commit entry list and pre-computes the hash to verify everything is synced

use crate::core::index::{commit_validator, CommitEntryReader};
use crate::error::OxenError;
use crate::model::{Commit, LocalRepository, NewCommit};
use crate::util;

pub fn compute(repo: &LocalRepository, commit: &Commit) -> Result<(), OxenError> {
    log::debug!("Running compute_and_write_hash");

    // sleep to make sure the commit is fully written to disk
    // Issue was with a lot of text files in this integration test:
    //     "test_remote_ls_return_data_types_just_top_level_dir"
    std::thread::sleep(std::time::Duration::from_millis(100));

    log::debug!("computing entry hash {} -> {}", commit.id, commit.message);
    let commit_entry_reader = CommitEntryReader::new(repo, commit)?;
    let entries = commit_entry_reader.list_entries()?;
    let n_commit = NewCommit::from_commit(commit); // need this to pass in metadata about commit
    let entries_hash = util::hasher::compute_commit_hash(&n_commit, &entries);

    log::debug!("computing content hash {} -> {}", commit.id, commit.message);
    let content_hash = commit_validator::compute_commit_content_hash(repo, commit)?;

    log::debug!(
        "computing comparing entries_hash == content_hash {} == {}",
        entries_hash,
        content_hash
    );

    if content_hash == entries_hash {
        write_is_valid(repo, commit, "true")?;
    } else {
        write_is_valid(repo, commit, "false")?;
    }

    Ok(())
}

pub fn is_valid(repo: &LocalRepository, commit: &Commit) -> Result<bool, OxenError> {
    Ok(read_is_valid(repo, commit)? == "true")
}

fn write_is_valid(repo: &LocalRepository, commit: &Commit, val: &str) -> Result<(), OxenError> {
    let hash_file_path = util::fs::commit_content_is_valid_path(repo, commit);
    util::fs::write_to_path(hash_file_path, val)?;
    Ok(())
}

fn read_is_valid(repo: &LocalRepository, commit: &Commit) -> Result<String, OxenError> {
    let hash_file_path = util::fs::commit_content_is_valid_path(repo, commit);
    log::debug!("Checking validity at hash_file_path {:?}", hash_file_path);
    let value = util::fs::read_from_path(hash_file_path)?;
    Ok(value)
}

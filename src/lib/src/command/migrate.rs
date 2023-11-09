use std::collections::HashMap;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use jwalk::WalkDir;

use crate::constants;
use crate::constants::{HASH_FILE, HISTORY_DIR, TREE_DIR, VERSIONS_DIR, VERSION_FILE_NAME};
use crate::core::index::{CommitEntryReader, CommitReader};
use crate::error::OxenError;
use crate::model::LocalRepository;
use crate::util::fs::version_dir_from_hash;
use crate::util::progress_bar::{oxen_progress_bar, ProgressBarType};
use crate::{api, util};

pub trait Migrate {
    fn up(&self, path: &Path, all: bool) -> Result<(), OxenError>;
    fn down(&self, path: &Path, all: bool) -> Result<(), OxenError>;
    fn is_needed(&self, repo: &LocalRepository) -> Result<bool, OxenError>;

    fn name(&self) -> &'static str;
}

pub struct UpdateVersionFilesMigration;
impl UpdateVersionFilesMigration {}
pub struct CreateMerkleTreesMigration;
impl CreateMerkleTreesMigration {}

impl Migrate for CreateMerkleTreesMigration {
    fn name(&self) -> &'static str {
        "create_merkle_trees"
    }
    fn up(&self, path: &Path, all: bool) -> Result<(), OxenError> {
        if all {
            create_merkle_trees_for_all_repos_up(path)?;
        } else {
            let repo = LocalRepository::new(path)?;
            create_merkle_trees_up(&repo)?;
        }
        Ok(())
    }

    fn down(&self, path: &Path, all: bool) -> Result<(), OxenError> {
        if all {
            create_merkle_trees_for_all_repos_down(path)?;
        } else {
            let repo = LocalRepository::new(path)?;
            create_merkle_trees_down(&repo)?;
        }
        Ok(())
    }
    fn is_needed(&self, repo: &LocalRepository) -> Result<bool, OxenError> {
        let history_dir = repo
            .path
            .join(constants::OXEN_HIDDEN_DIR)
            .join(constants::HISTORY_DIR);
        // Every top-level directory in history dir should have a subdirectory "tree"
        for entry in WalkDir::new(history_dir).min_depth(1).max_depth(1) {
            let entry = entry?;
            if entry.file_type().is_dir() {
                let path = entry.path();
                let tree_dir = path.join(constants::TREE_DIR);
                if !tree_dir.exists() {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
}

impl Migrate for UpdateVersionFilesMigration {
    fn name(&self) -> &'static str {
        "update_version_files"
    }
    fn up(&self, path: &Path, all: bool) -> Result<(), OxenError> {
        if all {
            update_version_files_for_all_repos_up(path)?;
        } else {
            let repo = LocalRepository::new(path)?;
            update_version_files_up(&repo)?;
        }
        Ok(())
    }

    fn down(&self, path: &Path, all: bool) -> Result<(), OxenError> {
        if all {
            update_version_files_for_all_repos_down(path)?;
        } else {
            println!("Running down migration");
            let repo = LocalRepository::new(path)?;
            update_version_files_down(&repo)?;
        }
        Ok(())
    }

    fn is_needed(&self, repo: &LocalRepository) -> Result<bool, OxenError> {
        let versions_dir = repo
            .path
            .join(constants::OXEN_HIDDEN_DIR)
            .join(constants::VERSIONS_DIR);
        if !versions_dir.exists() {
            return Ok(false);
        }
        for entry in WalkDir::new(&versions_dir) {
            let entry = entry?;
            if entry.file_type().is_file() {
                let path = entry.path();
                let filename = match path.file_name() {
                    Some(filename) => filename.to_string_lossy().to_string(),
                    None => continue,
                };

                if filename.starts_with(constants::HASH_FILE) {
                    continue;
                }

                if filename.starts_with(constants::VERSION_FILE_NAME) {
                    return Ok(false);
                }
                return Ok(true);
            }
        }
        Ok(false)
    }
}

// TODO: if / when we add a third, this can probably be wrapped up into a fn
// that takes in a migration and applies it to all repos in a namespace

pub fn create_merkle_trees_for_all_repos_up(path: &Path) -> Result<(), OxenError> {
    println!("🐂 Collecting namespaces to migrate...");
    let namespaces = api::local::repositories::list_namespaces(path)?;
    let bar = oxen_progress_bar(namespaces.len() as u64, ProgressBarType::Counter);
    println!("🐂 Migrating {} namespaces", namespaces.len());
    for namespace in namespaces {
        let namespace_path = path.join(namespace);
        // Show the canonical namespace path
        log::debug!(
            "This is the namespace path we're walking: {:?}",
            namespace_path.canonicalize()?
        );
        let repos = api::local::repositories::list_repos_in_namespace(&namespace_path);
        for repo in repos {
            match create_merkle_trees_up(&repo) {
                Ok(_) => {}
                Err(err) => {
                    log::error!(
                        "Could not migrate merkle trees for repo {:?}\nErr: {}",
                        repo.path.canonicalize(),
                        err
                    )
                }
            }
        }
        bar.inc(1);
    }
    Ok(())
}

pub fn create_merkle_trees_for_all_repos_down(path: &Path) -> Result<(), OxenError> {
    let namespaces = api::local::repositories::list_namespaces(path)?;
    for namespace in namespaces {
        let namespace_path = path.join(namespace);
        let repos = api::local::repositories::list_repos_in_namespace(&namespace_path);
        for repo in repos {
            match create_merkle_trees_down(&repo) {
                Ok(_) => {}
                Err(err) => {
                    log::error!(
                        "Could not down-migrate merkle trees for repo {:?}\nErr: {}",
                        repo.path,
                        err
                    )
                }
            }
        }
    }
    Ok(())
}

pub fn create_merkle_trees_up(repo: &LocalRepository) -> Result<(), OxenError> {
    // Get all commits in repo, then construct merkle tree for each commit
    let reader = CommitReader::new(repo)?;
    let all_commits = reader.list_all()?;
    let bar = oxen_progress_bar(all_commits.len() as u64, ProgressBarType::Counter);
    for commit in all_commits {
        match api::local::commits::construct_commit_merkle_tree(repo, &commit) {
            Ok(_) => {}
            Err(err) => {
                log::error!(
                    "Could not construct merkle tree for commit {:?}\nErr: {}",
                    commit.id,
                    err
                )
            }
        }
        bar.inc(1);
    }
    Ok(())
}

pub fn create_merkle_trees_down(repo: &LocalRepository) -> Result<(), OxenError> {
    let hidden_dir = util::fs::oxen_hidden_dir(&repo.path);
    let history_dir = hidden_dir.join(HISTORY_DIR);

    for entry in WalkDir::new(&history_dir) {
        match entry {
            Ok(val) => {
                let path = val.path();
                if path.is_dir() && path.ends_with(TREE_DIR) {
                    std::fs::remove_dir_all(path)?;
                }
            }
            Err(err) => {
                log::error!("Error walking directory {:?}\nErr: {}", history_dir, err);
            }
        }
    }
    Ok(())
}

pub fn update_version_files_for_all_repos_up(path: &Path) -> Result<(), OxenError> {
    println!("🐂 Collecting namespaces to migrate...");
    let namespaces = api::local::repositories::list_namespaces(path)?;
    let bar = oxen_progress_bar(namespaces.len() as u64, ProgressBarType::Counter);
    println!("🐂 Migrating {} namespaces", namespaces.len());
    for namespace in namespaces {
        let namespace_path = path.join(namespace);
        // Show the canonical namespace path
        log::debug!(
            "This is the namespace path we're walking: {:?}",
            namespace_path.canonicalize()?
        );
        let repos = api::local::repositories::list_repos_in_namespace(&namespace_path);
        for repo in repos {
            match update_version_files_up(&repo) {
                Ok(_) => {}
                Err(err) => {
                    log::error!(
                        "Could not migrate version files for repo {:?}\nErr: {}",
                        repo.path.canonicalize(),
                        err
                    )
                }
            }
        }
        bar.inc(1);
    }

    Ok(())
}

pub fn update_version_files_up(repo: &LocalRepository) -> Result<(), OxenError> {
    let hidden_dir = util::fs::oxen_hidden_dir(&repo.path);
    let versions_dir = hidden_dir.join(VERSIONS_DIR);

    for entry in WalkDir::new(&versions_dir) {
        match entry {
            Ok(val) => {
                let path = val.path();
                // Rename all files except for server-computed HASH
                if let Some(file_name) = path.file_name() {
                    if path.is_file() && file_name != HASH_FILE {
                        let new_path = util::fs::replace_file_name_keep_extension(
                            &path,
                            VERSION_FILE_NAME.to_owned(),
                        );
                        log::debug!("Renaming {:?} to {:?}", path, new_path);
                        std::fs::rename(path, new_path)?;
                    }
                } else {
                    log::debug!("No filename found for path {:?}", path);
                }
            }
            Err(err) => {
                log::error!("Error walking directory {:?}\nErr: {}", versions_dir, err);
            }
        }
    }

    Ok(())
}

pub fn update_version_files_down(repo: &LocalRepository) -> Result<(), OxenError> {
    // Hash map of entry hash (string) to path to write (commit id + extension)
    // (hash, extension) -> Vec<CommitId>

    // List all commits in the order they were created
    let reader = CommitReader::new(repo)?;
    let mut all_commits = reader.list_all()?;
    // Sort by timestamp from oldest to newest
    all_commits.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));

    // let mut entry_hash_to_commit_ids: HashMap<(String, String), Vec<String>> = HashMap::new();
    let mut entry_hash_and_path_to_first_commit_id: HashMap<(String, PathBuf), String> =
        HashMap::new();
    // Get all commits for repo

    // Collect the FIRST occurrence of a unique hash + full entry path combination.
    for commit in all_commits {
        let commit_entry_reader = CommitEntryReader::new(repo, &commit)?;
        let entries = commit_entry_reader.list_entries()?;
        for entry in entries {
            let entry_hash = entry.hash.clone().to_owned();
            let entry_path = entry.path.clone().to_owned();
            let commit_id = commit.id.to_owned();
            let key = (entry_hash, entry_path);
            // If key is in entry_hash_and_path_to_first_commit_id, do nothing.
            // Otherwise, add the key and commit_id to the map
            entry_hash_and_path_to_first_commit_id
                .entry(key)
                .or_insert(commit_id);
        }
    }

    // Iterate over these, copying the new-format data.extension file to commit_id.extension for all
    // commit ids, then delete new file
    for ((hash, path), commit_id) in entry_hash_and_path_to_first_commit_id.iter() {
        let version_dir = version_dir_from_hash(&repo.path, hash.to_string());
        let extension = util::fs::file_extension(path);
        let new_filename = if extension.is_empty() {
            version_dir.join(VERSION_FILE_NAME)
        } else {
            version_dir.join(format!("{}.{}", VERSION_FILE_NAME, extension))
        };

        if new_filename.exists() {
            let old_filename = version_dir.join(format!("{}.{}", commit_id, extension));
            std::fs::copy(new_filename.clone(), old_filename)?;
        } else {
            log::error!("Could not find version file {:?}", new_filename);
        }
    }

    // Now that all have been copied, iterate through and delete the new-format files
    let mut seen_files = HashSet::<PathBuf>::new();
    for ((hash, path), _commit_id) in entry_hash_and_path_to_first_commit_id.iter() {
        let version_dir = version_dir_from_hash(&repo.path, hash.to_string());
        let extension = util::fs::file_extension(path);
        let new_filename = if extension.is_empty() {
            version_dir.join(VERSION_FILE_NAME)
        } else {
            version_dir.join(format!("{}.{}", VERSION_FILE_NAME, extension))
        };

        if !seen_files.contains(&new_filename) {
            if new_filename.exists() {
                // Delete new file
                seen_files.insert(new_filename.clone());
                std::fs::remove_file(new_filename)?;
            } else {
                log::error!("Could not find version file {:?}", new_filename);
            }
        }
    }

    Ok(())
}

pub fn update_version_files_for_all_repos_down(path: &Path) -> Result<(), OxenError> {
    let namespaces = api::local::repositories::list_namespaces(path)?;
    let bar = oxen_progress_bar(namespaces.len() as u64, ProgressBarType::Counter);
    println!("🐂 Migrating {} namespaces", namespaces.len());
    for namespace in namespaces {
        let namespace_path = path.join(namespace);
        let repos = api::local::repositories::list_repos_in_namespace(&namespace_path);
        for repo in repos {
            match update_version_files_down(&repo) {
                Ok(_) => {}
                Err(err) => {
                    log::error!(
                        "Could not down-migrate version files for repo {:?}\nErr: {}",
                        repo.path,
                        err
                    )
                }
            }
        }
        bar.inc(1);
    }

    Ok(())
}

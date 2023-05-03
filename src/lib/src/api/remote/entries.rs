use crate::api::remote::client;
use crate::constants::AVG_CHUNK_SIZE;
use crate::error::OxenError;
use crate::model::{DirEntry, LocalRepository, RemoteRepository};
use crate::view::EntryMetaDataResponse;
use crate::{api, constants};
use crate::{current_function, util};

use async_compression::futures::bufread::GzipDecoder;
use async_std::prelude::*;
use async_tar::Archive;
use flate2::write::GzEncoder;
use flate2::Compression;
use futures_util::TryStreamExt;
use indicatif::ProgressBar;
use std::fs;
use std::io::prelude::*;
use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;

/// Get a entry from the remote repository given a path and a branch or commit id
pub async fn get_entry(
    remote_repo: &RemoteRepository,
    path: impl AsRef<Path>,
    committish: impl AsRef<str>,
) -> Result<DirEntry, OxenError> {
    let path = path.as_ref().to_string_lossy();
    let committish = committish.as_ref();
    let uri = format!("/entry/{}/{}", committish, path);
    let url = api::endpoint::url_from_repo(remote_repo, &uri)?;

    let client = client::new_for_url(&url)?;
    let response = client.get(&url).send().await?;
    let body = client::parse_json_body(&url, response).await?;
    log::debug!("{} got body {}", current_function!(), body);
    let parsed: EntryMetaDataResponse = serde_json::from_str(&body)?;

    Ok(parsed.entry)
}

/// Pings the remote server first to see if the entry exists
/// and get the size before downloading
pub async fn download_entry(
    remote_repo: &RemoteRepository,
    remote_path: impl AsRef<Path>,
    local_path: impl AsRef<Path>,
    committish: impl AsRef<str>,
    bar: &Arc<ProgressBar>,
) -> Result<(), OxenError> {
    let entry = get_entry(remote_repo, &remote_path, &committish).await?;
    if entry.size > AVG_CHUNK_SIZE {
        download_large_entry(
            remote_repo,
            &remote_path,
            &local_path,
            &committish,
            entry.size,
            bar,
        )
        .await
    } else {
        download_small_entry(remote_repo, remote_path, local_path, committish).await
    }
}

pub async fn download_small_entry(
    remote_repo: &RemoteRepository,
    remote_path: impl AsRef<Path>,
    dest: impl AsRef<Path>,
    committish: impl AsRef<str>,
) -> Result<(), OxenError> {
    let path = remote_path.as_ref().to_string_lossy();
    let committish = committish.as_ref();
    let uri = format!("/file/{}/{}", committish, path);
    let url = api::endpoint::url_from_repo(remote_repo, &uri)?;

    let client = client::new_for_url(&url)?;
    let response = client.get(&url).send().await?;

    let status = response.status();
    if reqwest::StatusCode::OK == status {
        // Copy to file
        let dest = dest.as_ref();
        let mut dest = { fs::File::create(dest)? };
        let mut content = Cursor::new(response.bytes().await?);
        std::io::copy(&mut content, &mut dest)?;
        Ok(())
    } else {
        let err = format!("Could not download entry status: {status}");
        Err(OxenError::basic_str(err))
    }
}

/// Download a file from the remote repository that is too large
/// to fit in a standard HTTP response body
///
/// Note: Does not do the check for if the file exists remotely before
/// downloading for efficiency reasons
pub async fn download_large_entry(
    remote_repo: &RemoteRepository,
    remote_path: impl AsRef<Path>,
    local_path: impl AsRef<Path>,
    committish: impl AsRef<str>,
    num_bytes: u64,
    bar: &Arc<ProgressBar>,
) -> Result<(), OxenError> {
    // Read chunks
    let chunk_size = AVG_CHUNK_SIZE;
    let total_size = num_bytes;
    let num_chunks = ((total_size / chunk_size) + 1) as usize;
    let mut chunk_size = chunk_size;

    // Write files to /tmp/oxen/HASH/chunk_0..N
    let remote_path = remote_path.as_ref();
    let local_path = local_path.as_ref();
    let hash = util::hasher::hash_str(&format!("{:?}_{:?}", remote_path, local_path));
    let tmp_dir = Path::new("/tmp").join("oxen").join(&hash);

    // TODO: We could probably download chunks in parallel too
    for i in 0..num_chunks {
        // Make sure we read the last size correctly
        let chunk_start = (i as u64) * chunk_size;
        if (chunk_start + chunk_size) > total_size {
            chunk_size = total_size % chunk_size;
        }

        let filename = format!("chunk_{i}");
        let tmp_file = tmp_dir.join(filename);

        try_download_entry_chunk(
            remote_repo,
            &remote_path,
            &tmp_file, // local_path
            &committish,
            chunk_start,
            chunk_size,
        )
        .await?;

        bar.inc(chunk_size);
    }

    // Once all downloaded, recombine file and delete temp dir
    log::debug!("Unpack to {:?}", local_path);

    // Create parent dir if it doesn't exist
    if let Some(parent) = local_path.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent)?;
        }
    }

    let mut combined_file = util::fs::file_create(local_path)?;

    let mut should_cleanup = false;
    for i in 0..num_chunks {
        let filename = format!("chunk_{i}");
        let tmp_file = tmp_dir.join(filename);

        log::debug!("Reading file bytes {:?}", tmp_file);
        match std::fs::File::open(&tmp_file) {
            Ok(mut chunk_file) => {
                let mut buffer: Vec<u8> = Vec::new();
                chunk_file
                    .read_to_end(&mut buffer)
                    .expect("Could not read tmp file to end...");

                match combined_file.write_all(&buffer) {
                    Ok(_) => {
                        log::debug!("Unpack successful! {:?}", local_path);
                        std::fs::remove_file(tmp_file)?;
                    }
                    Err(err) => {
                        log::error!("Could not write all data to disk {:?}", err);
                        should_cleanup = true;
                    }
                }
            }
            Err(err) => {
                log::error!("Could not read chunk file {tmp_file:?}: {err}");
                should_cleanup = true;
            }
        }
    }

    if should_cleanup {
        log::error!("Cleaning up tmp dir {:?}", tmp_dir);
        std::fs::remove_dir_all(tmp_dir)?;
        return Err(OxenError::basic_str("Could not write all data to disk"));
    }

    Ok(())
}

async fn try_download_entry_chunk(
    remote_repo: &RemoteRepository,
    remote_path: impl AsRef<Path>,
    local_path: impl AsRef<Path>,
    committish: impl AsRef<str>,
    chunk_start: u64,
    chunk_size: u64,
) -> Result<(), OxenError> {
    let mut try_num = 0;
    while try_num < constants::NUM_HTTP_RETRIES {
        match download_entry_chunk(
            remote_repo,
            &remote_path,
            &local_path,
            &committish,
            chunk_start,
            chunk_size,
        )
        .await
        {
            Ok(_) => {
                log::debug!("Downloaded chunk {:?}", local_path.as_ref());
                return Ok(());
            }
            Err(err) => {
                log::error!("Error trying to download chunk: {}", err);
                try_num += 1;
                let sleep_time = try_num * try_num;
                std::thread::sleep(std::time::Duration::from_secs(sleep_time));
            }
        }
    }
    Err(OxenError::basic_str("Retry download chunk failed"))
}

/// Downloads a chunk of a file
async fn download_entry_chunk(
    remote_repo: &RemoteRepository,
    remote_path: impl AsRef<Path>,
    local_path: impl AsRef<Path>,
    committish: impl AsRef<str>,
    chunk_start: u64,
    chunk_size: u64,
) -> Result<(), OxenError> {
    let remote_path = remote_path.as_ref();
    let local_path = local_path.as_ref();
    log::debug!(
        "{} {:?} -> {:?}",
        current_function!(),
        remote_path,
        local_path
    );

    let uri = format!(
        "/chunk/{}/{}?chunk_start={}&chunk_size={}",
        committish.as_ref(),
        remote_path.to_string_lossy(),
        chunk_start,
        chunk_size
    );

    let url = api::endpoint::url_from_repo(remote_repo, &uri)?;

    log::debug!("download_entry_chunk {}", url);

    let client = client::new_for_url(&url)?;
    let response = client.get(&url).send().await?;

    if let Some(parent) = local_path.parent() {
        if !parent.exists() {
            log::debug!("Create parent dir {:?}", parent);
            std::fs::create_dir_all(parent)?;
        }
    }

    let status = response.status();
    if reqwest::StatusCode::OK == status {
        // Copy to file
        let mut dest = { fs::File::create(local_path)? };
        let mut content = Cursor::new(response.bytes().await?);
        std::io::copy(&mut content, &mut dest)?;
        Ok(())
    } else {
        let err = format!("Could not download entry status: {status}");
        Err(OxenError::basic_str(err))
    }
}

pub async fn download_data_from_version_paths(
    local_repo: &LocalRepository,
    remote_repo: &RemoteRepository,
    content_ids: &[String],
) -> Result<u64, OxenError> {
    let total_retries = constants::NUM_HTTP_RETRIES;
    let mut num_retries = 0;

    while num_retries < total_retries {
        match try_download_data_from_version_paths(local_repo, remote_repo, content_ids).await {
            Ok(val) => return Ok(val),
            Err(OxenError::Authentication(val)) => return Err(OxenError::Authentication(val)),
            Err(err) => {
                num_retries += 1;
                // Exponentially back off
                let sleep_time = num_retries * num_retries;
                log::warn!(
                    "Could not download content {:?} sleeping {}",
                    err,
                    sleep_time
                );
                std::thread::sleep(std::time::Duration::from_secs(sleep_time));
            }
        }
    }

    let err = format!(
        "Err: Failed to download {} files after {} retries",
        content_ids.len(),
        total_retries
    );
    Err(OxenError::basic_str(err))
}

pub async fn try_download_data_from_version_paths(
    local_repo: &LocalRepository,
    remote_repo: &RemoteRepository,
    content_ids: &[String],
) -> Result<u64, OxenError> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    for content_id in content_ids.iter() {
        let line = format!("{content_id}\n");
        encoder.write_all(line.as_bytes())?;
    }
    let body = encoder.finish()?;
    let url = api::endpoint::url_from_repo(remote_repo, "/versions")?;

    let client = client::new_for_url(&url)?;
    if let Ok(res) = client.get(&url).body(body).send().await {
        if reqwest::StatusCode::UNAUTHORIZED == res.status() {
            let err = "Err: unauthorized request to download data".to_string();
            log::error!("{}", err);
            return Err(OxenError::authentication(err));
        }

        let reader = res
            .bytes_stream()
            .map_err(|e| futures::io::Error::new(futures::io::ErrorKind::Other, e))
            .into_async_read();
        let decoder = GzipDecoder::new(futures::io::BufReader::new(reader));
        let archive = Archive::new(decoder);

        let mut size: u64 = 0;
        // For debug if you want to see each file we are unpacking...
        let mut entries = archive.entries()?;
        while let Some(file) = entries.next().await {
            let mut file = file?;
            let path = file.path()?.to_path_buf();

            let fullpath = local_repo.path.join(&path);
            if let Some(parent) = fullpath.parent() {
                if !parent.exists() {
                    std::fs::create_dir_all(parent)?;
                }
            }

            // log::debug!("Unpacking into path {:?}", path);
            file.unpack_in(&local_repo.path).await?;

            let metadata = std::fs::metadata(&fullpath)?;
            size += metadata.len();
            log::debug!("Unpacking {} bytes {:?}", metadata.len(), path);
        }

        Ok(size)
    } else {
        let err =
            format!("api::entries::download_data_from_version_paths Err request failed: {url}");
        Err(OxenError::basic_str(err))
    }
}

#[cfg(test)]
mod tests {

    use crate::api;
    use crate::error::OxenError;
    use crate::test;

    #[tokio::test]
    async fn test_get_dir_entry() -> Result<(), OxenError> {
        test::run_training_data_fully_sync_remote(|_local_repo, remote_repo| async move {
            let path = "README.md";
            let committish = "main";
            let entry = api::remote::entries::get_entry(&remote_repo, path, committish).await?;

            assert_eq!(entry.filename, "README.md");

            Ok(remote_repo)
        })
        .await
    }
}

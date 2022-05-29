use liboxen::api;
use liboxen::command;
use liboxen::constants::HISTORY_DIR;
use liboxen::error::OxenError;
use liboxen::index::{CommitWriter, RefWriter};
use liboxen::model::{Commit, LocalRepository, RemoteRepository};
use liboxen::util;
use liboxen::view::http::{MSG_RESOURCE_CREATED, MSG_RESOURCE_FOUND, STATUS_SUCCESS};
use liboxen::view::{
    CommitResponse, ListCommitResponse, RemoteRepositoryHeadResponse, StatusMessage,
};

use crate::app_data::OxenAppData;

use actix_web::{web, Error, HttpRequest, HttpResponse};
use flate2::read::GzDecoder;
use futures_util::stream::StreamExt as _;
use std::path::Path;
use tar::Archive;
use flate2::write::GzEncoder;
use flate2::Compression;

// List commits for a repository
pub async fn index(req: HttpRequest) -> HttpResponse {
    let app_data = req.app_data::<OxenAppData>().unwrap();
    let path: Option<&str> = req.match_info().get("name");

    if let Some(path) = path {
        let repo_dir = app_data.path.join(path);
        match p_index(&repo_dir) {
            Ok(response) => HttpResponse::Ok().json(response),
            Err(err) => {
                let msg = format!("api err: {}", err);
                HttpResponse::NotFound().json(StatusMessage::error(&msg))
            }
        }
    } else {
        let msg = "Could not find `name` param...";
        HttpResponse::BadRequest().json(StatusMessage::error(msg))
    }
}

pub async fn stats(req: HttpRequest) -> HttpResponse {
    let app_data = req.app_data::<OxenAppData>().unwrap();

    let name: Option<&str> = req.match_info().get("repo_name");
    let commit_id: Option<&str> = req.match_info().get("commit_id");
    if let (Some(name), Some(commit_id)) = (name, commit_id) {
        match api::local::repositories::get_by_name(&app_data.path, name) {
            Ok(repository) => match api::local::repositories::get_commit_stats_from_id(&repository, &commit_id) {
                Ok(Some(commit)) => HttpResponse::Ok().json(RemoteRepositoryHeadResponse {
                    status: String::from(STATUS_SUCCESS),
                    status_message: String::from(MSG_RESOURCE_CREATED),
                    repository: RemoteRepository::from_local(&repository),
                    head: Some(commit),
                }),
                Ok(None) => {
                    log::debug!("Could not get find commit id: {}", commit_id);
                    HttpResponse::NotFound().json(StatusMessage::resource_not_found())
                }
                Err(err) => {
                    log::error!("Could not get find commit id: {}", err);
                    HttpResponse::InternalServerError().json(StatusMessage::internal_server_error())
                }
            },
            Err(err) => {
                log::error!("Could not find repo: {}", err);
                HttpResponse::InternalServerError().json(StatusMessage::internal_server_error())
            }
        }
    } else {
        log::error!("bad request: {:?}", req);
        let msg = "Could not find `repo_name` or `commit_id` param...";
        HttpResponse::BadRequest().json(StatusMessage::error(msg))
    }
}

pub async fn show(req: HttpRequest) -> HttpResponse {
    let app_data = req.app_data::<OxenAppData>().unwrap();

    let name: Option<&str> = req.match_info().get("repo_name");
    let commit_id: Option<&str> = req.match_info().get("commit_id");
    if let (Some(name), Some(commit_id)) = (name, commit_id) {
        match api::local::repositories::get_by_name(&app_data.path, name) {
            Ok(repository) => match api::local::commits::get_by_id(&repository, commit_id) {
                Ok(Some(commit)) => HttpResponse::Ok().json(CommitResponse {
                    status: String::from(STATUS_SUCCESS),
                    status_message: String::from(MSG_RESOURCE_CREATED),
                    commit,
                }),
                Ok(None) => {
                    log::debug!("commit_id {} does not exist for repo: {}", commit_id, name);
                    HttpResponse::NotFound().json(StatusMessage::resource_not_found())
                }
                Err(err) => {
                    log::debug!("Err getting commit_id {}: {}", commit_id, err);
                    HttpResponse::NotFound().json(StatusMessage::resource_not_found())
                }
            },
            Err(err) => {
                log::debug!("Could not find repo [{}]: {}", name, err);
                HttpResponse::NotFound().json(StatusMessage::resource_not_found())
            }
        }
    } else {
        let msg = "Must supply `repo_name` and `commit_id` params";
        HttpResponse::BadRequest().json(StatusMessage::error(msg))
    }
}

pub async fn parent(req: HttpRequest) -> HttpResponse {
    let app_data = req.app_data::<OxenAppData>().unwrap();

    let name: Option<&str> = req.match_info().get("repo_name");
    let commit_id: Option<&str> = req.match_info().get("commit_id");
    if let (Some(name), Some(commit_id)) = (name, commit_id) {
        match api::local::repositories::get_by_name(&app_data.path, name) {
            Ok(repository) => match p_get_parent(&repository, commit_id) {
                Ok(Some(parent)) => HttpResponse::Ok().json(CommitResponse {
                    status: String::from(STATUS_SUCCESS),
                    status_message: String::from(MSG_RESOURCE_FOUND),
                    commit: parent,
                }),
                Ok(None) => {
                    log::debug!("commit {} has no parent in repo {}", commit_id, name);
                    HttpResponse::NotFound().json(StatusMessage::resource_not_found())
                }
                Err(err) => {
                    log::debug!(
                        "Error finding parent for commit {} in repo {}\nErr: {}",
                        commit_id,
                        name,
                        err
                    );
                    HttpResponse::NotFound().json(StatusMessage::resource_not_found())
                }
            },
            Err(err) => {
                log::debug!("Could not find repo [{}]: {}", name, err);
                HttpResponse::NotFound().json(StatusMessage::resource_not_found())
            }
        }
    } else {
        let msg = "Must supply `repo_name` and `commit_id` params";
        HttpResponse::BadRequest().json(StatusMessage::error(msg))
    }
}

fn p_get_parent(
    repository: &LocalRepository,
    commit_id: &str,
) -> Result<Option<Commit>, OxenError> {
    match api::local::commits::get_by_id(repository, commit_id) {
        Ok(Some(commit)) => api::local::commits::get_parent(repository, &commit),
        Ok(None) => Ok(None),
        Err(err) => Err(err),
    }
}

fn p_index(repo_dir: &Path) -> Result<ListCommitResponse, OxenError> {
    let repo = LocalRepository::new(&repo_dir)?;
    let commits = command::log(&repo)?;
    Ok(ListCommitResponse::success(commits))
}

pub async fn download_commit_db(req: HttpRequest,) -> HttpResponse {
    let app_data = req.app_data::<OxenAppData>().unwrap();

    let name: Option<&str> = req.match_info().get("repo_name");
    let commit_id: Option<&str> = req.match_info().get("commit_id");
    if let (Some(name), Some(commit_id)) = (name, commit_id) {
        match api::local::repositories::get_by_name(&app_data.path, name) {
            Ok(repository) => match api::local::commits::get_by_id(&repository, commit_id) {
                Ok(Some(commit)) => {
                    match compress_commit(&repository, &commit) {
                        Ok(buffer) => {
                            HttpResponse::Ok().body(buffer)
                        },
                        Err(err) => {
                            log::error!("Error compressing commit: [{}] Err: {}", name, err);
                            HttpResponse::InternalServerError().json(StatusMessage::internal_server_error())
                        }
                    }
                },
                Ok(None) => {
                    log::debug!("Could not find commit [{}]", name);
                    HttpResponse::NotFound().json(StatusMessage::resource_not_found())
                }
                Err(err) => {
                    log::error!("Error finding commit: [{}] Err: {}", name, err);
                    HttpResponse::NotFound().json(StatusMessage::resource_not_found())
                }
            },
            Err(err) => {
                log::debug!("Could not find repo [{}]: {}", name, err);
                HttpResponse::NotFound().json(StatusMessage::resource_not_found())
            }
        }
    } else {
        let msg = "Must supply `repo_name` and `commit_id` params";
        HttpResponse::BadRequest().json(StatusMessage::error(msg))
    }
}

fn compress_commit(repository: &LocalRepository, commit: &Commit) -> Result<Vec<u8>, OxenError> {
    // Tar and gzip the commit db directory
    // zip up the rocksdb in history dir, and post to server
    let commit_dir = util::fs::oxen_hidden_dir(&repository.path).join(HISTORY_DIR).join(commit.id.clone());
    // This will be the subdir within the tarball
    let tar_subdir = Path::new("history").join(commit.id.clone());

    println!("Compressing commit {}", commit.id);
    let enc = GzEncoder::new(Vec::new(), Compression::default());
    let mut tar = tar::Builder::new(enc);

    tar.append_dir_all(&tar_subdir, commit_dir)?;
    tar.finish()?;

    let buffer: Vec<u8> = tar.into_inner()?.finish()?;
    Ok(buffer)
}

pub async fn upload(
    req: HttpRequest,
    mut body: web::Payload,   // the actual file body
    data: web::Query<Commit>, // these are the query params -> struct
) -> Result<HttpResponse, Error> {
    let app_data = req.app_data::<OxenAppData>().unwrap();
    // name to the repo, should be in url path so okay to unwap
    let name: &str = req.match_info().get("name").unwrap();
    match api::local::repositories::get_by_name(&app_data.path, name) {
        Ok(repo) => {
            let hidden_dir = util::fs::oxen_hidden_dir(&repo.path);

            // Create Commit from uri params
            let commit = &data.into_inner();
            match create_commit(&repo.path, commit) {
                Ok(_) => {
                    // Get tar.gz bytes for history/COMMIT_ID data
                    let mut bytes = web::BytesMut::new();
                    while let Some(item) = body.next().await {
                        bytes.extend_from_slice(&item?);
                    }

                    // Unpack tarball to our hidden dir
                    let mut archive = Archive::new(GzDecoder::new(&bytes[..]));
                    archive.unpack(hidden_dir)?;

                    Ok(HttpResponse::Ok().json(CommitResponse {
                        status: String::from(STATUS_SUCCESS),
                        status_message: String::from(MSG_RESOURCE_CREATED),
                        commit: commit.to_owned(),
                    }))
                }
                Err(err) => {
                    log::error!("Err create_commit: {}", err);
                    Ok(HttpResponse::InternalServerError()
                        .json(StatusMessage::internal_server_error()))
                }
            }
        }
        Err(err) => {
            log::error!("Err get_by_name: {}", err);
            Ok(HttpResponse::InternalServerError().json(StatusMessage::internal_server_error()))
        }
    }
}

fn create_commit(repo_dir: &Path, commit: &Commit) -> Result<(), OxenError> {
    let repo = LocalRepository::from_dir(repo_dir)?;
    let result = CommitWriter::new(&repo);
    match result {
        Ok(commit_writer) => match commit_writer.add_commit_to_db(commit) {
            Ok(_) => {
                let ref_writer = RefWriter::new(&repo)?;
                ref_writer.set_head_commit_id(&commit.id)?;
            }
            Err(err) => {
                log::error!("Error adding commit to db: {:?}", err);
            }
        },
        Err(err) => {
            log::error!("Error creating commit writer: {:?}", err);
        }
    };
    Ok(())
}

#[cfg(test)]
mod tests {

    use actix_web::body::to_bytes;
    use actix_web::{web, App};
    use chrono::Utc;
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::path::Path;

    use liboxen::command;
    use liboxen::constants::OXEN_HIDDEN_DIR;
    use liboxen::error::OxenError;
    use liboxen::model::Commit;
    use liboxen::util;
    use liboxen::view::{CommitResponse, ListCommitResponse};

    use crate::app_data::OxenAppData;
    use crate::controllers;
    use crate::test;

    #[actix_web::test]
    async fn test_controllers_respository_commits_index_empty() -> Result<(), OxenError> {
        let sync_dir = test::get_sync_dir()?;

        let name = "Testing-Name";
        test::create_local_repo(&sync_dir, name)?;

        let uri = format!("/repositories/{}/commits", name);
        let req = test::request_with_param(&sync_dir, &uri, "name", name);

        let resp = controllers::commits::index(req).await;

        let body = to_bytes(resp.into_body()).await.unwrap();
        let text = std::str::from_utf8(&body).unwrap();
        let list: ListCommitResponse = serde_json::from_str(text)?;
        // Plus the initial commit
        assert_eq!(list.commits.len(), 1);

        // cleanup
        std::fs::remove_dir_all(sync_dir)?;

        Ok(())
    }

    #[actix_web::test]
    async fn test_controllers_respository_list_two_commits() -> Result<(), OxenError> {
        let sync_dir = test::get_sync_dir()?;

        let name = "Testing-Name";
        let repo = test::create_local_repo(&sync_dir, name)?;

        liboxen::test::add_txt_file_to_dir(&repo.path, "hello")?;
        command::commit(&repo, "first commit")?;
        liboxen::test::add_txt_file_to_dir(&repo.path, "world")?;
        command::commit(&repo, "second commit")?;

        let uri = format!("/repositories/{}/commits", name);
        let req = test::request_with_param(&sync_dir, &uri, "name", name);

        let resp = controllers::commits::index(req).await;
        let body = to_bytes(resp.into_body()).await.unwrap();
        let text = std::str::from_utf8(&body).unwrap();
        let list: ListCommitResponse = serde_json::from_str(text)?;
        // Plus the initial commit
        assert_eq!(list.commits.len(), 3);

        // cleanup
        std::fs::remove_dir_all(sync_dir)?;

        Ok(())
    }

    #[actix_web::test]
    async fn test_controllers_commits_upload() -> Result<(), OxenError> {
        let sync_dir = test::get_sync_dir()?;

        let name = "Testing-Name";
        let repo = test::create_local_repo(&sync_dir, name)?;
        let commit = Commit {
            id: format!("{}", uuid::Uuid::new_v4()),
            parent_id: None,
            message: String::from("Hello"),
            author: String::from("Greg"),
            date: Utc::now(),
        };

        // create random tarball to post.. currently no validation that it is a valid commit dir
        let path_to_compress = format!("history/{}", commit.id);
        let commit_dir_name = format!("/tmp/oxen/commit/{}", commit.id);
        let commit_dir = Path::new(&commit_dir_name);
        std::fs::create_dir_all(commit_dir)?;
        // Write a random file to it
        let zipped_filename = "blah.txt";
        let zipped_file_contents = "sup";
        let random_file = commit_dir.join(zipped_filename);
        util::fs::write_to_path(&random_file, zipped_file_contents);

        println!("Compressing commit {}...", commit.id);
        let enc = GzEncoder::new(Vec::new(), Compression::default());
        let mut tar = tar::Builder::new(enc);

        tar.append_dir_all(&path_to_compress, &commit_dir)?;
        tar.finish()?;
        let payload: Vec<u8> = tar.into_inner()?.finish()?;

        let commit_query = Commit::to_uri_encoded(&commit);
        let uri = format!("/repositories/{}/commits?{}", name, commit_query);
        let app = actix_web::test::init_service(
            App::new()
                .app_data(OxenAppData {
                    path: sync_dir.clone(),
                })
                .route(
                    "/repositories/{name}/commits",
                    web::post().to(controllers::commits::upload),
                ),
        )
        .await;

        let req = actix_web::test::TestRequest::post()
            .uri(&uri)
            .set_payload(payload)
            .to_request();

        let resp = actix_web::test::call_service(&app, req).await;
        let bytes = actix_http::body::to_bytes(resp.into_body()).await.unwrap();
        let body = std::str::from_utf8(&bytes).unwrap();
        let resp: CommitResponse = serde_json::from_str(body)?;

        // Make sure commit gets populated
        assert_eq!(resp.commit.id, commit.id);
        assert_eq!(resp.commit.message, commit.message);
        assert_eq!(resp.commit.author, commit.author);
        assert_eq!(resp.commit.parent_id, commit.parent_id);

        // Make sure we unzipped the tar ball
        let uploaded_file = sync_dir
            .join(repo.name)
            .join(OXEN_HIDDEN_DIR)
            .join(path_to_compress)
            .join(zipped_filename);
        assert!(uploaded_file.exists());
        assert_eq!(
            util::fs::read_from_path(&uploaded_file)?,
            zipped_file_contents
        );

        // cleanup
        std::fs::remove_dir_all(sync_dir)?;
        std::fs::remove_dir_all(commit_dir)?;

        Ok(())
    }
}

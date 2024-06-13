use super::controllers;
use actix_web::web;

pub fn config(cfg: &mut web::ServiceConfig) {
    cfg.route("", web::post().to(controllers::repositories::create))
        .route(
            "/{namespace}",
            web::get().to(controllers::repositories::index),
        )
        .service(
            web::resource("/{namespace}/{repo_name}")
                // we give the resource a name here so it can be used with HttpRequest.url_for
                .name("repo_root")
                .route(web::get().to(controllers::repositories::show))
                .route(web::delete().to(controllers::repositories::delete)),
        )
        .route(
            "/{namespace}/{repo_name}/transfer",
            web::patch().to(controllers::repositories::transfer_namespace),
        )
        .route(
            "/{namespace}/{repo_name}/commits_db",
            web::get().to(controllers::commits::download_commits_db),
        )
        .route(
            "/{namespace}/{repo_name}/objects_db",
            web::get().to(controllers::commits::download_objects_db),
        )
        .service(
            web::scope("/{namespace}/{repo_name}")
                // Commits
                .service(
                    web::scope("/commits")
                        .route("", web::get().to(controllers::commits::index))
                        .route("", web::post().to(controllers::commits::create))
                        .route("/bulk", web::post().to(controllers::commits::create_bulk))
                        .route("/root", web::get().to(controllers::commits::root_commit))
                        .route(
                            "/complete",
                            web::post().to(controllers::commits::complete_bulk),
                        )
                        .route(
                            "/{commit_id}/db_status",
                            web::get().to(controllers::commits::commits_db_status),
                        )
                        .route(
                            "/{commit_id}/entries_status",
                            web::get().to(controllers::commits::entries_status),
                        )
                        .route("/all", web::get().to(controllers::commits::list_all))
                        .route(
                            "/{commit_id}/latest_synced",
                            web::get().to(controllers::commits::latest_synced),
                        )
                        .route("/{commit_id}", web::get().to(controllers::commits::show))
                        .route(
                            "/{commit_id}/data",
                            web::post().to(controllers::commits::upload),
                        )
                        .route(
                            "/{commit_id}/can_push",
                            web::get().to(controllers::commits::can_push),
                        )
                        .route(
                            "/{commit_id}/complete",
                            web::post().to(controllers::commits::complete),
                        )
                        .route(
                            "/{commit_id}/upload_chunk",
                            web::post().to(controllers::commits::upload_chunk),
                        )
                        .route(
                            "/{commit_or_branch:.*}/history",
                            web::get().to(controllers::commits::commit_history),
                        )
                        .route(
                            "/{commit_or_branch:.*}/parents",
                            web::get().to(controllers::commits::parents),
                        )
                        .route(
                            "/{commit_or_branch:.*}/is_synced",
                            web::get().to(controllers::commits::is_synced),
                        )
                        .route(
                            "/{commit_or_branch:.*}/commit_db",
                            web::get().to(controllers::commits::download_commit_entries_db),
                        ),
                )
                .service(
                    web::scope("/revisions")
                        .route("/{resource:.*}", web::get().to(controllers::revisions::get)),
                )
                // Branches
                .service(
                    web::scope("/branches")
                        .route("", web::get().to(controllers::branches::index))
                        .route(
                            "",
                            web::post().to(controllers::branches::create_from_or_get),
                        )
                        .route(
                            "/{branch_name:.*}/lock",
                            web::post().to(controllers::branches::lock),
                        )
                        .route(
                            "/{branch_name:.*}/versions/{path:.*}",
                            web::get().to(controllers::branches::list_entry_versions),
                        )
                        .route(
                            "/{branch_name}/latest_synced_commit",
                            web::get().to(controllers::branches::latest_synced_commit),
                        )
                        .route(
                            "/{branch_name:.*}/lock",
                            web::get().to(controllers::branches::is_locked),
                        )
                        .route(
                            "/{branch_name:.*}/unlock",
                            web::post().to(controllers::branches::unlock),
                        )
                        .route(
                            "/{branch_name:.*}/merge",
                            web::put().to(controllers::branches::maybe_create_merge),
                        )
                        .route(
                            "/{branch_name:.*}",
                            web::get().to(controllers::branches::show),
                        )
                        .route(
                            "/{branch_name:.*}",
                            web::delete().to(controllers::branches::delete),
                        )
                        .route(
                            "/{branch_name:.*}",
                            web::put().to(controllers::branches::update),
                        ),
                )
                // Compare
                .service(
                    web::scope("/compare")
                        .route(
                            "/commits/{base_head:.*}",
                            web::get().to(controllers::diff::commits),
                        )
                        .route(
                            "/dir_tree/{base_head:.*}",
                            web::get().to(controllers::diff::dir_tree),
                        )
                        .route(
                            "/entries/{base_head:.*}/dir/{dir:.*}",
                            web::get().to(controllers::diff::dir_entries),
                        )
                        .route(
                            "/entries/{base_head:.*}",
                            web::get().to(controllers::diff::entries),
                        )
                        .route(
                            "/file/{base_head:.*}",
                            web::get().to(controllers::diff::file),
                        )
                        .route(
                            "/data_frame/{compare_id}/{path}/{base_head:.*}",
                            web::get().to(controllers::diff::get_derived_df),
                        )
                        .route(
                            "/data_frame/{compare_id}",
                            web::post().to(controllers::diff::get_df_diff),
                        )
                        .route(
                            "/data_frame/{compare_id}",
                            web::put().to(controllers::diff::update_df_diff),
                        )
                        .route(
                            "/data_frame",
                            web::post().to(controllers::diff::create_df_diff),
                        )
                        .route(
                            "/data_frame/{compare_id}",
                            web::delete().to(controllers::diff::delete_df_diff),
                        ),
                )
                // Merge
                .route(
                    "/merge/{base_head:.*}",
                    web::get().to(controllers::merger::show),
                )
                .route(
                    "/merge/{base_head:.*}",
                    web::post().to(controllers::merger::merge),
                )
                // Staging
                .service(
                    web::scope("/workspace/{identifier}")
                        .route(
                            "/status/{resource:.*}",
                            web::get().to(controllers::workspace::status_dir),
                        )
                        .route(
                            "/entries/{resource:.*}",
                            web::post().to(controllers::workspace::add_file),
                        )
                        .route(
                            "/entries/{resource:.*}",
                            web::delete().to(controllers::workspace::delete_file),
                        )
                        .route(
                            "/file/{resource:.*}",
                            web::get().to(controllers::workspace::get_file),
                        )
                        .route(
                            "/file/{resource:.*}",
                            web::post().to(controllers::workspace::add_file),
                        )
                        .route(
                            "/file/{resource:.*}",
                            web::delete().to(controllers::workspace::delete_file),
                        )
                        .route(
                            "/diff/{resource:.*}",
                            web::get().to(controllers::workspace::diff_file),
                        )
                        .route(
                            "/modifications/{resource:.*}",
                            web::delete().to(controllers::workspace::clear_modifications),
                        )
                        .route(
                            "/commit/{resource:.*}",
                            web::post().to(controllers::workspace::commit),
                        )
                        // staging/data_frame
                        // GET /workspace/data_frame/branch/main
                        // List all data frames on a branch
                        //   GET /workspace/{workspace_id}/data_frame/resource/{branch:.*}
                        // List a specific data frame on a branch
                        //   GET /workspace/{workspace_id}/data_frame/resource/{resource:.*}
                        //   GET /workspace/{workspace_id}/data_frame/resource/main/path/to/df.parquet
                        //     { "is_editable": true }
                        //   PUT /workspace/{workspace_id}/data_frame/resource/main/path/to/df.parquet
                        //     { "is_indexed": true }
                        // Get the diff for a data frame on a branch
                        //   GET /workspace/{workspace_id}/data_frame/diff/main/path/to/df.parquet
                        // CRUD operations on a row
                        //   GET /workspace/{workspace_id}/data_frame/rows/resource/{resource:.*}
                        //   GET /workspace/{workspace_id}/data_frame/rows/resource/main/path/to/df.parquet
                        //   PUT /workspace/{workspace_id}/data_frame/rows/resource/main/path/to/df.parquet
                        //   POST /workspace/{workspace_id}/data_frame/rows/resource/main/path/to/df.parquet
                        //   POST /workspace/{workspace_id}/data_frame/rows/restore/main/path/to/df.parquet
                        .service(
                            web::scope("/data_frame")
                                // TODO: Get rid of "list_editable" and "is_editable" in favor of a more RESTFUL /data_frame API
                                .route(
                                    "/branch/{branch:.*}",
                                    web::get()
                                        .to(controllers::workspace::data_frame::get_by_branch),
                                )
                                .route(
                                    "/resource/{resource:.*}",
                                    web::get()
                                        .to(controllers::workspace::data_frame::get_by_resource),
                                )
                                // TODO: name conflict with resource:.*
                                .route(
                                    "/diff/{resource:.*}",
                                    web::get().to(controllers::workspace::data_frame::diff),
                                )
                                .route(
                                    "/resource/{resource:.*}",
                                    web::put().to(controllers::workspace::data_frame::post),
                                )
                                // staging/data_frame/rows
                                // TODO: This conflicts with any branch named "row", should it just be /staging/rows? or /staging/data_frame_rows?
                                .service(
                                    web::scope("/rows")
                                        // TODO: Refactor. This means we can't have a branch called "restore"
                                        // maybe we put it in /staging/restore_row
                                        .route(
                                            "/{row_id}/restore/{resource:.*}",
                                            web::post().to(
                                                controllers::workspace::data_frame::row::restore,
                                            ),
                                        )
                                        .route(
                                            "/{resource:.*}",
                                            web::post().to(
                                                controllers::workspace::data_frame::row::create,
                                            ),
                                        )
                                        .route(
                                            "/{row_id}/{resource:.*}",
                                            web::put().to(
                                                controllers::workspace::data_frame::row::update,
                                            ),
                                        )
                                        .route(
                                            "/{row_id}/{resource:.*}",
                                            web::delete().to(
                                                controllers::workspace::data_frame::row::delete,
                                            ),
                                        )
                                        .route(
                                            "/{row_id}/{resource:.*}",
                                            web::get()
                                                .to(controllers::workspace::data_frame::row::get),
                                        ),
                                ),
                        ),
                )
                // Dir
                .route("/dir/{resource:.*}", web::get().to(controllers::dir::get))
                // File
                .route("/file/{resource:.*}", web::get().to(controllers::file::get))
                // Chunk
                .route(
                    "/chunk/{resource:.*}",
                    web::get().to(controllers::entries::download_chunk),
                )
                // Metadata
                .service(
                    web::scope("/meta")
                        .route(
                            "/agg/dir/{resource:.*}",
                            web::get().to(controllers::metadata::agg_dir),
                        )
                        .route(
                            "/dir/{resource:.*}",
                            web::get().to(controllers::metadata::dir),
                        )
                        .route(
                            "/images/{resource:.*}",
                            web::get().to(controllers::metadata::images),
                        )
                        .route("/{resource:.*}", web::get().to(controllers::metadata::file)),
                )
                // DataFrame
                .route(
                    "/data_frame/index/{resource:.*}",
                    web::post().to(controllers::data_frames::index),
                )
                .route(
                    "/data_frame/{resource:.*}",
                    web::get().to(controllers::data_frames::get),
                )
                // Lines
                .route(
                    "/lines/{resource:.*}",
                    web::get().to(controllers::entries::list_lines_in_file),
                )
                // Versions
                .route(
                    "/versions",
                    web::get().to(controllers::entries::download_data_from_version_paths),
                )
                // Schemas
                .route(
                    "/schemas/hash/{hash}",
                    web::get().to(controllers::schemas::get_by_hash),
                )
                .route(
                    "/schemas/{resource:.*}",
                    web::get().to(controllers::schemas::list_or_get),
                )
                // Tabular
                .route(
                    "/tabular/{commit_or_branch:.*}",
                    web::get().to(controllers::entries::list_tabular),
                )
                // Stats
                .route("/stats", web::get().to(controllers::repositories::stats))
                // Action Callbacks
                .route(
                    "/action/completed/{action}",
                    web::get().to(controllers::action::completed),
                )
                .route(
                    "/action/started/{action}",
                    web::get().to(controllers::action::started),
                )
                .route(
                    "/action/completed/{action}",
                    web::post().to(controllers::action::completed),
                )
                .route(
                    "/action/started/{action}",
                    web::post().to(controllers::action::started),
                ),
        );
}

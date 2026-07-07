pub mod branch;
pub mod commands;
pub mod commit;
pub mod delta;
pub mod error;
pub mod gc;
pub mod kra;
pub mod raster;
pub mod repo;
pub mod scan;
pub mod tiles;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Cached diff rasters are served straight from `.kvc/cache/` over this scheme — no base64,
    // no multi-MB IPC strings, browser-cacheable (see raster::raster_url / commands::serve_raster).
    raster::enable_img_protocol();
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .register_asynchronous_uri_scheme_protocol("kvcimg", |_ctx, request, responder| {
            let uri = request.uri().clone();
            // File reads happen off the webview thread; each response is one cached PNG.
            tauri::async_runtime::spawn_blocking(move || {
                responder.respond(commands::serve_raster(&uri));
            });
        })
        .invoke_handler(tauri::generate_handler![
            commands::init_repository,
            commands::is_repository,
            commands::open_repository,
            commands::delete_repository,
            commands::scan_repository,
            commands::commit_snapshot,
            commands::list_commits,
            commands::list_branches,
            commands::create_branch,
            commands::switch_branch,
            commands::merge_branch,
            commands::delete_branch,
            commands::layer_diff,
            commands::restore_file,
            commands::rollback_to_commit,
            commands::undo_last_commit,
            commands::commit_diff,
            commands::commit_layers,
            commands::working_diff,
            commands::working_layers,
            commands::cleanup_repository,
            commands::get_repo_config,
            commands::set_repo_config,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

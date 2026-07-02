pub mod commands;
pub mod commit;
pub mod delta;
pub mod error;
pub mod kra;
pub mod raster;
pub mod repo;
pub mod scan;
pub mod tiles;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            commands::init_repository,
            commands::is_repository,
            commands::open_repository,
            commands::delete_repository,
            commands::scan_repository,
            commands::commit_snapshot,
            commands::list_commits,
            commands::layer_diff,
            commands::restore_file,
            commands::rollback_to_commit,
            commands::undo_last_commit,
            commands::commit_diff,
            commands::commit_layers,
            commands::working_diff,
            commands::working_layers,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

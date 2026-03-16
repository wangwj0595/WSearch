mod commands;
mod models;
mod services;

use commands::{
    clear_search_history, copy_path, get_search_config, get_search_history, open_file,
    reveal_in_explorer, save_search_config, search_files,
};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 初始化日志
    env_logger::init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![
            search_files,
            get_search_config,
            save_search_config,
            get_search_history,
            clear_search_history,
            open_file,
            reveal_in_explorer,
            copy_path,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

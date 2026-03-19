mod commands;
mod models;
mod services;

use commands::{
    cancel_search, clear_search_history, copy_path, delete_file, delete_files, get_current_results, get_search_config,
    get_search_history, get_window_config, open_file, reveal_in_explorer, save_search_config,
    save_window_config, search_files, SearchState,
};
use services::ConfigStore;
use tauri::LogicalSize;
use tauri::Manager;

/// 检查窗口位置是否有效（过滤最小化时的无效坐标 -32000）
fn is_window_position_valid(x: i32, y: i32) -> bool {
    // Windows 最小化时位置为 -32000，这是无效位置
    x > -30000 && y > -30000
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 初始化日志
    env_logger::init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .manage(SearchState::default())
        .manage(ConfigStore::new())
        .setup(|app| {
            // 读取窗口配置并恢复窗口大小
            let config_store = app.state::<ConfigStore>();
            let window_config = config_store.get_window_config();

            if let Some(window) = app.get_webview_window("main") {
                // 如果保存了最大化状态，恢复最大化
                if window_config.is_maximized {
                    window.maximize().ok();
                } else {
                    // 设置窗口大小
                    if window_config.width > 0 && window_config.height > 0 {
                        window.set_size(LogicalSize::new(
                            window_config.width as f64,
                            window_config.height as f64,
                        )).ok();
                    }

                    // 设置窗口位置（仅当位置有效时）
                    if is_window_position_valid(window_config.x, window_config.y) {
                        use tauri::LogicalPosition;
                        window.set_position(LogicalPosition::new(
                            window_config.x as f64,
                            window_config.y as f64,
                        )).ok();
                    }
                }

                // 设置完成后显示窗口
                window.show().ok();
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            search_files,
            get_current_results,
            get_search_config,
            save_search_config,
            get_search_history,
            clear_search_history,
            cancel_search,
            open_file,
            reveal_in_explorer,
            copy_path,
            delete_file,
            delete_files,
            save_window_config,
            get_window_config,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

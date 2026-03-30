mod commands;
mod models;
mod services;

use commands::{
    cancel_search, clear_search_history, copy_path, delete_file, delete_files, get_current_results, get_recent_usn,
    get_search_config, get_search_history, get_window_config, open_file, refresh_index, rename_file, reveal_in_explorer, save_search_config,
    save_window_config, search_files, SearchState,
};
use services::{init_cache, start_incremental_service, stop_incremental_service, save_usn_state, ConfigStore};
use tauri::LogicalSize;
use tauri::Manager;

/// 检查窗口位置是否有效（过滤最小化时的无效坐标 -32000）
fn is_window_position_valid(x: i32, y: i32) -> bool {
    // Windows 最小化时位置为 -32000，这是无效位置
    x > -30000 && y > -30000
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 初始化日志（强制设置为 info 级别）
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_secs()
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .manage(SearchState::default())
        .manage(ConfigStore::new())
        .on_window_event(|_window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                log::info!("窗口关闭，保存数据");

                // 停止增量服务（这会停止后台线程并保存 USN 状态）
                services::stop_incremental_service();

                // 等待一小段时间让后台线程完成保存
                std::thread::sleep(std::time::Duration::from_millis(500));

                // 同步保存索引缓存
                let cache_manager = services::get_cache_manager();
                cache_manager.flush();
                log::info!("索引缓存已保存");
            }
        })
        .setup(|app| {
            // 初始化索引缓存（加载已有缓存或准备重建）
            init_cache();

            // 读取窗口配置并恢复窗口大小
            let config_store = app.state::<ConfigStore>();
            let window_config = config_store.get_window_config();

            // 获取搜索路径并启动增量更新服务
            let search_config = config_store.load_config().search_config;
            if !search_config.search_paths.is_empty() && cfg!(windows) {
                // 获取需要监控的卷
                let volumes: Vec<String> = search_config.search_paths
                    .iter()
                    .filter_map(|p| {
                        let path = std::path::Path::new(p);
                        path.components().next().and_then(|c| {
                            c.as_os_str().to_str().map(|s| format!("{}\\", s))
                        })
                    })
                    .collect();

                if !volumes.is_empty() {
                    // 检查是否有缓存，有缓存才启动增量更新服务
                    let cache_manager = services::get_cache_manager();
                    let has_any_cache = !cache_manager.get_indexed_volumes().is_empty();

                    if has_any_cache {
                        log::info!("有缓存存在，启动增量更新服务，监控卷: {:?}", volumes);
                        let _ = start_incremental_service(volumes);
                    } else {
                        log::info!("没有缓存存在，跳过启动增量更新服务，等待用户手动触发索引重建");
                    }
                }
            }

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
            rename_file,
            save_window_config,
            get_window_config,
            get_recent_usn,
            refresh_index,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

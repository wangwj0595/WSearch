use crate::models::{SearchConfig, SearchResult};
use crate::services::config_store::ConfigStore;
use crate::services::file_scanner::FileScanner;
use crate::services::trigger_incremental_update;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::thread;
use std::time::Instant;
use tauri::{AppHandle, Emitter, State};

/// 搜索结果缓冲状态
pub struct SearchState {
    pub results: Arc<Mutex<Vec<SearchResult>>>,
    pub is_searching: Arc<Mutex<bool>>,
    pub is_cancelled: Arc<AtomicBool>,
}

impl Default for SearchState {
    fn default() -> Self {
        Self {
            results: Arc::new(Mutex::new(Vec::new())),
            is_searching: Arc::new(Mutex::new(false)),
            is_cancelled: Arc::new(AtomicBool::new(false)),
        }
    }
}

/// 搜索文件（专用线程 + Channel 实时版本）
#[tauri::command]
pub async fn search_files(
    query: String,
    search_paths: Vec<String>,
    exclude_paths: Vec<String>,
    file_types: Vec<String>,
    search_content: bool,
    case_sensitive: bool,
    search_directories: bool,
    use_mft: bool,
    max_results: usize,
    min_size: u64,
    max_size: u64,
    app_handle: AppHandle,
    search_state: State<'_, SearchState>,
) -> Result<(), String> {
    if query.is_empty() {
        return Err("搜索关键词不能为空".to_string());
    }

    if search_paths.is_empty() {
        return Err("请至少添加一个搜索目录".to_string());
    }

    // 设置搜索状态，重置取消标志
    {
        let mut is_searching = search_state.is_searching.lock().unwrap();
        *is_searching = true;
    }
    search_state.is_cancelled.store(false, Ordering::SeqCst);
    {
        let mut results = search_state.results.lock().unwrap();
        results.clear();
    }

    // 在搜索前先触发一次增量更新，确保索引是最新的
    // let _ = trigger_incremental_update();

    // 发送搜索开始事件
    let _ = app_handle.emit("search_started", ());

    // 使用默认配置（presets 和 active_preset_id 不影响搜索）
    let mut config = SearchConfig::default();
    config.search_paths = search_paths;
    config.exclude_paths = exclude_paths;
    config.file_types = file_types;
    config.search_content = search_content;
    config.case_sensitive = case_sensitive;
    config.search_directories = search_directories;
    config.use_mft = use_mft;
    config.max_results = max_results;
    config.min_size = min_size;
    config.max_size = max_size;

    let scanner = FileScanner::new(config);
    let start_time = Instant::now();

    // 使用专用线程 + Channel 搜索
    let (result_rx, progress_rx, _is_cancelled) = scanner.search_with_channel(query.clone());

    // 使用 Arc 共享状态
    let results_arc = search_state.results.clone();
    let is_searching_arc = search_state.is_searching.clone();

    // 在独立线程中监听结果并发送到前端
    let app_handle_for_results = app_handle.clone();

    let result_thread = thread::spawn(move || {
        // 收集所有结果
        let mut all_results = Vec::new();

        // 监听结果 channel，找到就立即发送到前端
        while let Ok(item) = result_rx.recv() {
            let result = item.result;
            // 更新状态
            {
                let mut results = results_arc.lock().unwrap();
                results.push(result.clone());
            }
            all_results.push(result.clone());

            // 立即发送单个结果到前端（真实时）
            let _ = app_handle_for_results.emit("search_result_batch", vec![result]);
        }

        all_results
    });

    // 监听进度 channel，定期发送进度到前端（需要 clone app_handle）
    let app_handle_for_progress = app_handle.clone();
    thread::spawn(move || {
        while let Ok(progress) = progress_rx.recv() {
            let _ = app_handle_for_progress.emit("search_progress", progress);
        }
    });

    // 等待结果收集完成
    let all_results = result_thread.join().unwrap_or_default();

    // 保存搜索历史
    let store = ConfigStore::new();
    let _ = store.add_search_history(query, all_results.len());

    // 发送搜索完成事件
    let elapsed = start_time.elapsed().as_secs();
    #[derive(serde::Serialize, Clone)]
    struct SearchCompletedEvent {
        result_count: usize,
        elapsed_time: u64,
    }
    let event_data = SearchCompletedEvent {
        result_count: all_results.len(),
        elapsed_time: elapsed,
    };
    let _ = app_handle.emit("search_completed", event_data);

    // 重置搜索状态
    {
        let mut is_searching = is_searching_arc.lock().unwrap();
        *is_searching = false;
    }

    Ok(())
}

/// 获取当前搜索结果
#[tauri::command]
pub fn get_current_results(search_state: State<'_, SearchState>) -> Vec<SearchResult> {
    let results = search_state.results.lock().unwrap();
    results.clone()
}

/// 获取搜索配置
#[tauri::command]
pub fn get_search_config() -> Result<SearchConfig, String> {
    let store = ConfigStore::new();
    Ok(store.load_config().search_config)
}

/// 保存搜索配置
#[tauri::command]
pub fn save_search_config(config: SearchConfig) -> Result<(), String> {
    let store = ConfigStore::new();
    store.save_search_config(config)
}

/// 获取搜索历史
#[tauri::command]
pub fn get_search_history() -> Result<Vec<crate::models::SearchHistory>, String> {
    let store = ConfigStore::new();
    Ok(store.get_search_history())
}

/// 清除搜索历史
#[tauri::command]
pub fn clear_search_history() -> Result<(), String> {
    let store = ConfigStore::new();
    let mut config = store.load_config();
    config.search_history.clear();
    store.save_config(&config)
}

/// 取消搜索
#[tauri::command]
pub fn cancel_search(search_state: State<'_, SearchState>) -> Result<(), String> {
    search_state.is_cancelled.store(true, Ordering::SeqCst);
    let mut is_searching = search_state.is_searching.lock().unwrap();
    *is_searching = false;
    Ok(())
}

/// 更新索引（强制重建指定卷的索引）
#[tauri::command]
pub async fn refresh_index(volume: String) -> Result<String, String> {
    use crate::services::file_scanner::FileScanner;
    use crate::services::index_cache::get_cache_manager;
    use crate::services::mft_reader;
    use crate::services::is_running_as_admin;
    use crate::services::usn_monitor;

    // 检查管理员权限
    if !is_running_as_admin() {
        return Err("需要管理员权限才能更新索引".to_string());
    }

    // 检查 NTFS 卷
    if !mft_reader::is_ntfs_volume(&volume) {
        return Err(format!("卷 {} 不是 NTFS 格式", volume));
    }

    log::info!("开始更新卷 {} 的索引", volume);

    // 重建索引前先停止 USN Monitor，避免竞争条件
    log::info!("停止 USN Monitor");
    usn_monitor::stop_incremental_service();

    // 等待一小段时间让后台线程完成
    std::thread::sleep(std::time::Duration::from_millis(500));

    // 读取 MFT
    let mft_entries = mft_reader::scan_volume_files(&volume);

    if mft_entries.is_empty() {
        // 恢复 USN Monitor
        let _ = usn_monitor::start_incremental_service(vec![volume.clone()]);
        return Err("无法读取 MFT 数据".to_string());
    }

    // 获取缓存管理器并更新索引
    let cache_manager = get_cache_manager();

    // 检查是否已有该卷的索引
    if cache_manager.has_path(&volume) {
        // 已有索引，完全替换
        cache_manager.update_from_mft(mft_entries, &volume);
    } else {
        // 新卷，增量添加
        cache_manager.add_volume_from_mft(mft_entries, &volume);
    }

    log::info!("卷 {} 索引更新完成，重启 USN Monitor", volume);

    // 重建索引后重新启动 USN Monitor
    // 注意：start_incremental_service 会自动加载之前保存的 USN 状态
    if let Err(e) = usn_monitor::start_incremental_service(vec![volume.clone()]) {
        log::warn!("重启 USN Monitor 失败: {}", e);
    }

    log::info!("卷 {} 索引更新完成", volume);
    Ok(format!("索引更新成功，共 {} 个文件", cache_manager.file_count()))
}

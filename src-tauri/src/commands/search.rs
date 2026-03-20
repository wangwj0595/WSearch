use crate::models::{SearchConfig, SearchResult};
use crate::services::config_store::ConfigStore;
use crate::services::file_scanner::FileScanner;
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

    // 发送搜索开始事件
    let _ = app_handle.emit("search_started", ());

    let config = SearchConfig {
        search_paths,
        exclude_paths,
        file_types,
        search_content,
        case_sensitive,
        search_directories,
        use_mft,
        max_results,
        sidebar_width: 280,
        collapsed_panels: Vec::new(),
    };

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

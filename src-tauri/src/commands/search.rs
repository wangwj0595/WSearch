use crate::models::{SearchConfig, SearchResult};
use crate::services::{ConfigStore, FileScanner};
use futures_util::StreamExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, State};

/// 搜索结果缓冲状态
pub struct SearchState {
    pub results: Mutex<Vec<SearchResult>>,
    pub is_searching: Mutex<bool>,
    pub is_cancelled: Arc<AtomicBool>,
}

impl Default for SearchState {
    fn default() -> Self {
        Self {
            results: Mutex::new(Vec::new()),
            is_searching: Mutex::new(false),
            is_cancelled: Arc::new(AtomicBool::new(false)),
        }
    }
}

/// 搜索文件（异步流式并行版本）
#[tauri::command]
pub async fn search_files(
    query: String,
    search_paths: Vec<String>,
    exclude_paths: Vec<String>,
    file_types: Vec<String>,
    search_content: bool,
    case_sensitive: bool,
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
        max_results,
        sidebar_width: 280,
    };

    let mut scanner = FileScanner::new(config);
    let scanner_for_time = scanner.clone();

    // 克隆取消标志的 Arc 引用
    let is_cancelled = search_state.is_cancelled.clone();

    // 使用异步流式并行搜索
    let mut stream = scanner.search_streaming(query.clone(), is_cancelled).await;

    // 收集所有结果
    let mut all_results = Vec::new();

    // 遍历流，处理每个批次
    while let Some(batch_results) = stream.next().await {
        if !batch_results.is_empty() {
            // 更新状态
            {
                let mut results = search_state.results.lock().unwrap();
                results.extend(batch_results.clone());
            }
            all_results.extend(batch_results.clone());
            // 发送批次结果到前端
            let _ = app_handle.emit("search_result_batch", batch_results);
        }
    }

    // 保存搜索历史
    let store = ConfigStore::new();
    let _ = store.add_search_history(query, all_results.len());

    // 发送搜索完成事件
    let elapsed = scanner_for_time.get_elapsed_time();
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
        let mut is_searching = search_state.is_searching.lock().unwrap();
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

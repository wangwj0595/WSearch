use crate::models::{SearchConfig, SearchResult};
use crate::services::{ConfigStore, FileScanner};

/// 搜索文件
#[tauri::command]
pub async fn search_files(
    query: String,
    search_paths: Vec<String>,
    exclude_paths: Vec<String>,
    file_types: Vec<String>,
    search_content: bool,
    case_sensitive: bool,
    max_results: usize,
) -> Result<Vec<SearchResult>, String> {
    if query.is_empty() {
        return Err("搜索关键词不能为空".to_string());
    }

    if search_paths.is_empty() {
        return Err("请至少添加一个搜索目录".to_string());
    }

    let config = SearchConfig {
        search_paths,
        exclude_paths,
        file_types,
        search_content,
        case_sensitive,
        max_results,
    };

    let scanner = FileScanner::new(config);
    let results = scanner.search(&query);

    // 保存搜索历史
    let store = ConfigStore::new();
    let _ = store.add_search_history(query, results.len());

    Ok(results)
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

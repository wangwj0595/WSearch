use serde::{Deserialize, Serialize};

/// 搜索结果项
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub name: String,
    pub path: String,
    pub is_directory: bool,
    pub size: u64,
    pub modified_time: String,
    pub match_content: Option<String>,
}

/// 搜索配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchConfig {
    pub search_paths: Vec<String>,
    pub exclude_paths: Vec<String>,
    pub file_types: Vec<String>,
    pub search_content: bool,
    pub case_sensitive: bool,
    pub search_directories: bool,
    pub use_mft: bool,
    pub max_results: usize,
    pub sidebar_width: u32,
    pub collapsed_panels: Vec<String>,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            search_paths: Vec::new(),
            exclude_paths: vec![
                "node_modules".to_string(),
                ".git".to_string(),
                "target".to_string(),
                "dist".to_string(),
                "build".to_string(),
            ],
            file_types: Vec::new(),
            search_content: false,
            case_sensitive: false,
            search_directories: true,
            use_mft: false,
            max_results: 1000,
            sidebar_width: 280,
            collapsed_panels: Vec::new(),
        }
    }
}

/// 搜索进度
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchProgress {
    pub scanned_files: u64,
    pub found_results: u64,
    pub is_complete: bool,
    pub current_path: String,
}

impl Default for SearchProgress {
    fn default() -> Self {
        Self {
            scanned_files: 0,
            found_results: 0,
            is_complete: false,
            current_path: String::new(),
        }
    }
}

/// 搜索历史记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHistory {
    pub query: String,
    pub timestamp: String,
    pub result_count: usize,
}

/// 窗口配置
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WindowConfig {
    pub width: u32,
    pub height: u32,
    pub x: i32,
    pub y: i32,
    pub is_maximized: bool,
}

/// 应用配置（包含搜索配置和历史）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
    pub search_config: SearchConfig,
    pub search_history: Vec<SearchHistory>,
    pub window_config: WindowConfig,
}

/// 搜索预设
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchPreset {
    pub name: String,
    pub config: SearchConfig,
}

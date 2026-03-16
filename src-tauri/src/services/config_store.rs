use crate::models::{AppConfig, SearchConfig};
use dirs;
use std::fs;
use std::path::PathBuf;

/// 配置存储服务
pub struct ConfigStore {
    config_path: PathBuf,
}

impl ConfigStore {
    pub fn new() -> Self {
        let config_dir = dirs_config_dir();
        let config_path = config_dir.join("wsearch_config.json");

        Self { config_path }
    }

    /// 加载配置
    pub fn load_config(&self) -> AppConfig {
        if !self.config_path.exists() {
            return AppConfig::default();
        }

        match fs::read_to_string(&self.config_path) {
            Ok(content) => {
                serde_json::from_str(&content).unwrap_or_default()
            }
            Err(_) => AppConfig::default(),
        }
    }

    /// 保存配置
    pub fn save_config(&self, config: &AppConfig) -> Result<(), String> {
        // 确保目录存在
        if let Some(parent) = self.config_path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }

        let json = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
        fs::write(&self.config_path, json).map_err(|e| e.to_string())?;

        Ok(())
    }

    /// 保存搜索配置
    pub fn save_search_config(&self, search_config: SearchConfig) -> Result<(), String> {
        let mut config = self.load_config();
        config.search_config = search_config;
        self.save_config(&config)
    }

    /// 添加搜索历史
    pub fn add_search_history(&self, query: String, result_count: usize) -> Result<(), String> {
        let mut config = self.load_config();

        // 避免重复历史
        if let Some(last) = config.search_history.first() {
            if last.query == query {
                return Ok(());
            }
        }

        // 添加新历史（保留最多20条）
        let history = crate::models::SearchHistory {
            query,
            timestamp: chrono_now(),
            result_count,
        };

        config.search_history.insert(0, history);

        if config.search_history.len() > 20 {
            config.search_history.truncate(20);
        }

        self.save_config(&config)
    }

    /// 获取搜索历史
    pub fn get_search_history(&self) -> Vec<crate::models::SearchHistory> {
        self.load_config().search_history
    }
}

impl Default for ConfigStore {
    fn default() -> Self {
        Self::new()
    }
}

/// 获取配置目录
fn dirs_config_dir() -> PathBuf {
    if cfg!(windows) {
        std::env::var("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("WSearch")
    } else if cfg!(target_os = "macos") {
        dirs::home_dir()
            .map(|p| p.join("Library/Application Support/WSearch"))
            .unwrap_or_else(|| PathBuf::from("."))
    } else {
        dirs::config_dir()
            .map(|p| p.join("wsearch"))
            .unwrap_or_else(|| PathBuf::from("."))
    }
}

/// 获取当前时间字符串
fn chrono_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();

    let total_minutes = secs / 60;
    let hours = (total_minutes % 1440) / 60;
    let minutes = total_minutes % 60;
    let seconds = secs % 60;

    let days_since_epoch = secs / 86400;
    let year = 1970 + days_since_epoch / 365;
    let day_of_year = days_since_epoch % 365;
    let month = (day_of_year / 30) + 1;
    let day = (day_of_year % 30) + 1;

    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", year, month, day, hours, minutes, seconds)
}
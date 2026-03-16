use crate::models::{SearchConfig, SearchResult};
use rayon::prelude::*;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use walkdir::WalkDir;

/// 文件扫描器
pub struct FileScanner {
    config: SearchConfig,
    scanned_count: Arc<AtomicU64>,
    found_count: Arc<AtomicU64>,
}

impl FileScanner {
    pub fn new(config: SearchConfig) -> Self {
        Self {
            config,
            scanned_count: Arc::new(AtomicU64::new(0)),
            found_count: Arc::new(AtomicU64::new(0)),
        }
    }

    /// 执行搜索
    pub fn search(&self, query: &str) -> Vec<SearchResult> {
        let query_lower = if self.config.case_sensitive {
            query.to_string()
        } else {
            query.to_lowercase()
        };

        let mut results = Vec::new();

        // 并行搜索多个目录
        let search_results: Vec<Vec<SearchResult>> = self
            .config
            .search_paths
            .par_iter()
            .map(|path| {
                self.search_in_path(path, &query_lower)
            })
            .collect();

        for mut result_list in search_results {
            results.append(&mut result_list);
        }

        // 按找到数量限制结果
        if results.len() > self.config.max_results {
            results.truncate(self.config.max_results);
        }

        results
    }

    /// 在指定路径中搜索
    fn search_in_path(&self, path: &str, query: &str) -> Vec<SearchResult> {
        let mut results = Vec::new();
        let path = Path::new(path);

        if !path.exists() {
            return results;
        }

        for entry in WalkDir::new(path)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| !self.should_exclude(e.path()))
        {
            if results.len() >= self.config.max_results {
                break;
            }

            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };

            // 更新扫描计数
            self.scanned_count.fetch_add(1, Ordering::Relaxed);

            let file_name = entry.file_name().to_string_lossy().to_string();
            let file_path = entry.path();

            // 检查是否匹配搜索条件
            let mut is_match = false;
            let mut match_content = None;

            // 文件名搜索
            let name_to_check = if self.config.case_sensitive {
                file_name.clone()
            } else {
                file_name.to_lowercase()
            };

            if name_to_check.contains(query) {
                is_match = true;
            }

            // 内容搜索
            if !is_match && self.config.search_content && entry.file_type().is_file() {
                if let Ok(content) = self.read_file_content(file_path) {
                    let content_to_check = if self.config.case_sensitive {
                        content.clone()
                    } else {
                        content.to_lowercase()
                    };
                    if let Some(pos) = content_to_check.find(query) {
                        is_match = true;
                        // 提取匹配内容的前后文
                        let start = pos.saturating_sub(50);
                        let end = (pos + query.len() + 50).min(content.len());
                        match_content = Some(content[start..end].to_string());
                    }
                }
            }

            // 文件类型过滤
            if is_match && !self.config.file_types.is_empty() {
                if let Some(ext) = entry.path().extension() {
                    let ext_str = ext.to_string_lossy().to_lowercase();
                    is_match = self.config.file_types.iter().any(|t| {
                        let t_lower = t.to_lowercase();
                        t_lower == ext_str || t_lower == format!(".{}", ext_str)
                    });
                } else {
                    is_match = false;
                }
            }

            if is_match {
                let metadata = entry.metadata().ok();
                let size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);
                let modified_time = metadata
                    .and_then(|m| m.modified().ok())
                    .map(|t| {
                        let datetime: chrono_lite::DateTime = t.into();
                        datetime.format("%Y-%m-%d %H:%M:%S").to_string()
                    })
                    .unwrap_or_default();

                results.push(SearchResult {
                    name: file_name,
                    path: file_path.to_string_lossy().to_string(),
                    is_directory: entry.file_type().is_dir(),
                    size,
                    modified_time,
                    match_content,
                });

                self.found_count.fetch_add(1, Ordering::Relaxed);
            }
        }

        results
    }

    /// 检查路径是否应该被排除
    fn should_exclude(&self, path: &Path) -> bool {
        let path_str = path.to_string_lossy();

        for exclude in &self.config.exclude_paths {
            // 精确匹配目录名
            if let Some(file_name) = path.file_name() {
                if file_name == exclude {
                    return true;
                }
            }
            // 检查路径是否包含排除的目录
            if path_str.contains(&format!("/{}", exclude)) || path_str.contains(&format!("\\{}", exclude)) {
                return true;
            }
        }

        false
    }

    /// 读取文件内容（用于内容搜索）
    fn read_file_content(&self, path: &Path) -> Result<String, std::io::Error> {
        // 限制读取文件大小（最大 1MB）
        let metadata = fs::metadata(path)?;
        if metadata.len() > 1024 * 1024 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "File too large",
            ));
        }
        fs::read_to_string(path)
    }

    /// 获取扫描计数
    pub fn get_scanned_count(&self) -> u64 {
        self.scanned_count.load(Ordering::Relaxed)
    }

    /// 获取找到的结果数
    pub fn get_found_count(&self) -> u64 {
        self.found_count.load(Ordering::Relaxed)
    }
}

// 简化版 datetime
mod chrono_lite {
    use std::time::{SystemTime, UNIX_EPOCH};

    pub struct DateTime(SystemTime);

    impl From<SystemTime> for DateTime {
        fn from(time: SystemTime) -> Self {
            DateTime(time)
        }
    }

    impl DateTime {
        pub fn format(&self, format: &str) -> String {
            let duration = self.0.duration_since(UNIX_EPOCH).unwrap_or_default();
            let secs = duration.as_secs();

            // 简化的时间计算
            let total_minutes = secs / 60;
            let hours = (total_minutes % 1440) / 60;
            let minutes = total_minutes % 60;
            let seconds = secs % 60;

            // 计算日期
            let days_since_epoch = secs / 86400;
            let year = 1970 + days_since_epoch / 365;
            let day_of_year = days_since_epoch % 365;
            let month = (day_of_year / 30) + 1;
            let day = (day_of_year % 30) + 1;

            format
                .replace("%Y", &format!("{:04}", year))
                .replace("%m", &format!("{:02}", month))
                .replace("%d", &format!("{:02}", day))
                .replace("%H", &format!("{:02}", hours))
                .replace("%M", &format!("{:02}", minutes))
                .replace("%S", &format!("{:02}", seconds))
        }
    }
}

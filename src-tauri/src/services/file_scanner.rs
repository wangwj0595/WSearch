use crate::models::{SearchConfig, SearchResult};
use rayon::prelude::*;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;
use walkdir::WalkDir;

/// 搜索进度信息
#[derive(Debug, Clone, serde::Serialize)]
pub struct SearchProgress {
    pub scanned_count: u64,
    pub found_count: u64,
    pub current_path: String,
    pub elapsed_time: u64,
    pub estimated_remaining: u64,
}

/// 文件扫描器
pub struct FileScanner {
    config: SearchConfig,
    scanned_count: Arc<AtomicU64>,
    found_count: Arc<AtomicU64>,
    start_time: Option<Instant>,
}

impl FileScanner {
    pub fn new(config: SearchConfig) -> Self {
        Self {
            config,
            scanned_count: Arc::new(AtomicU64::new(0)),
            found_count: Arc::new(AtomicU64::new(0)),
            start_time: None,
        }
    }

    /// 执行搜索（带回调，每5条触发一次）
    /// 使用串行搜索确保线程安全
    pub fn search_stream<F, P>(&mut self, query: &str, is_cancelled: Arc<AtomicBool>, callback: F, progress_callback: P) -> Vec<SearchResult>
    where
        F: Fn(Vec<SearchResult>) + Send + Sync,
        P: Fn(SearchProgress) + Send + Sync,
    {
        // 记录搜索开始时间
        self.start_time = Some(Instant::now());

        // 按空格分割关键词，支持多关键词 AND 搜索
        let keywords: Vec<String> = query
            .split_whitespace()
            .filter(|s| !s.is_empty())
            .map(|s| {
                if self.config.case_sensitive {
                    s.to_string()
                } else {
                    s.to_lowercase()
                }
            })
            .collect();

        // 如果没有有效关键词，返回空结果
        if keywords.is_empty() {
            return Vec::new();
        }

        let mut results = Vec::new();
        let mut batch: Vec<SearchResult> = Vec::new();
        let batch_size = 5;

        // 串行搜索多个目录（确保每5条按顺序触发回调）
        for path in &self.config.search_paths {
            // 检查是否已取消
            if is_cancelled.load(Ordering::SeqCst) {
                break;
            }

            // 开始搜索目录时立即发送进度
            self.send_progress(&progress_callback, path);

            let path_results = self.search_in_path_stream(path, &keywords, &mut batch, batch_size, &callback, &is_cancelled, &progress_callback);
            results.extend(path_results);

            // 发送当前目录搜索完成的进度
            self.send_progress(&progress_callback, path);

            // 检查是否已取消
            if is_cancelled.load(Ordering::SeqCst) {
                break;
            }
        }

        // 发送剩余的批次
        if !batch.is_empty() {
            let remaining: Vec<SearchResult> = batch.drain(..).collect();
            if !remaining.is_empty() {
                callback(remaining);
            }
        }

        // 按找到数量限制结果
        if results.len() > self.config.max_results {
            results.truncate(self.config.max_results);
        }

        results
    }

    /// 在指定路径中搜索（带流式回调）
    fn search_in_path_stream<F, P>(
        &self,
        path: &str,
        keywords: &[String],
        batch: &mut Vec<SearchResult>,
        batch_size: usize,
        callback: &F,
        is_cancelled: &Arc<AtomicBool>,
        progress_callback: &P,
    ) -> Vec<SearchResult>
    where
        F: Fn(Vec<SearchResult>) + Send + Sync,
        P: Fn(SearchProgress) + Send + Sync,
    {
        let mut results = Vec::new();
        let path = Path::new(path);
        let mut scanned_since_last_progress = 0u64;
        let mut current_file_path = String::new();
        const PROGRESS_INTERVAL: u64 = 100;

        if !path.exists() {
            return results;
        }

        for entry in WalkDir::new(path)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| !self.should_exclude(e.path()))
        {
            // 检查是否已取消
            if is_cancelled.load(Ordering::SeqCst) {
                break;
            }

            if results.len() >= self.config.max_results {
                break;
            }

            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };

            // 更新扫描计数
            self.scanned_count.fetch_add(1, Ordering::Relaxed);
            scanned_since_last_progress += 1;

            let file_name = entry.file_name().to_string_lossy().to_string();
            let file_path = entry.path();

            // 更新当前正在扫描的文件路径（叶子目录/文件）
            current_file_path = file_path.to_string_lossy().to_string();

            // 每扫描100个文件发送一次进度更新（使用当前文件路径）
            if scanned_since_last_progress >= PROGRESS_INTERVAL {
                self.send_progress(progress_callback, &current_file_path);
                scanned_since_last_progress = 0;
            }

            // 检查是否匹配搜索条件（多关键词 AND 搜索）
            let mut is_match = false;
            let mut match_content = None;

            // 文件名搜索 - 必须包含所有关键词
            let name_to_check = if self.config.case_sensitive {
                file_name.clone()
            } else {
                file_name.to_lowercase()
            };

            let all_keywords_match = keywords.iter().all(|keyword| name_to_check.contains(keyword));
            if all_keywords_match {
                is_match = true;
            }

            // 内容搜索 - 也必须包含所有关键词
            if !is_match && self.config.search_content && entry.file_type().is_file() {
                if let Ok(content) = self.read_file_content(file_path) {
                    let content_to_check = if self.config.case_sensitive {
                        content.clone()
                    } else {
                        content.to_lowercase()
                    };

                    // 检查是否包含所有关键词
                    let content_contains_all = keywords.iter().all(|keyword| content_to_check.contains(keyword));
                    if content_contains_all {
                        is_match = true;
                        // 提取第一个匹配关键词的前后文
                        if let Some(first_keyword) = keywords.first() {
                            if let Some(pos) = content_to_check.find(first_keyword) {
                                let start = pos.saturating_sub(50);
                                let end = (pos + first_keyword.len() + 50).min(content.len());
                                match_content = Some(content[start..end].to_string());
                            }
                        }
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

                // 创建两个独立的 result 实例
                let result_for_batch = SearchResult {
                    name: file_name.clone(),
                    path: file_path.to_string_lossy().to_string(),
                    is_directory: entry.file_type().is_dir(),
                    size,
                    modified_time: modified_time.clone(),
                    match_content: match_content.clone(),
                };

                let result_for_list = SearchResult {
                    name: file_name,
                    path: file_path.to_string_lossy().to_string(),
                    is_directory: entry.file_type().is_dir(),
                    size,
                    modified_time,
                    match_content,
                };

                // 添加到批次
                batch.push(result_for_batch);
                // 添加到结果列表
                results.push(result_for_list);

                // 每5条触发一次回调
                if batch.len() >= batch_size {
                    let batch_to_send: Vec<SearchResult> = batch.drain(..).collect();
                    callback(batch_to_send);
                }

                self.found_count.fetch_add(1, Ordering::Relaxed);
            }
        }

        results
    }

    /// 执行搜索（非流式版本）
    pub fn search(&self, query: &str) -> Vec<SearchResult> {
        // 按空格分割关键词，支持多关键词 AND 搜索
        let keywords: Vec<String> = query
            .split_whitespace()
            .filter(|s| !s.is_empty())
            .map(|s| {
                if self.config.case_sensitive {
                    s.to_string()
                } else {
                    s.to_lowercase()
                }
            })
            .collect();

        // 如果没有有效关键词，返回空结果
        if keywords.is_empty() {
            return Vec::new();
        }

        let mut results = Vec::new();

        // 并行搜索多个目录
        let search_results: Vec<Vec<SearchResult>> = self
            .config
            .search_paths
            .par_iter()
            .map(|path| {
                self.search_in_path(path, &keywords)
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

    /// 在指定路径中搜索（非流式版本）
    fn search_in_path(&self, path: &str, keywords: &[String]) -> Vec<SearchResult> {
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

            // 检查是否匹配搜索条件（多关键词 AND 搜索）
            let mut is_match = false;
            let mut match_content = None;

            // 文件名搜索 - 必须包含所有关键词
            let name_to_check = if self.config.case_sensitive {
                file_name.clone()
            } else {
                file_name.to_lowercase()
            };

            let all_keywords_match = keywords.iter().all(|keyword| name_to_check.contains(keyword));
            if all_keywords_match {
                is_match = true;
            }

            // 内容搜索 - 也必须包含所有关键词
            if !is_match && self.config.search_content && entry.file_type().is_file() {
                if let Ok(content) = self.read_file_content(file_path) {
                    let content_to_check = if self.config.case_sensitive {
                        content.clone()
                    } else {
                        content.to_lowercase()
                    };

                    let content_contains_all = keywords.iter().all(|keyword| content_to_check.contains(keyword));
                    if content_contains_all {
                        is_match = true;
                        // 提取第一个匹配关键词的前后文
                        if let Some(first_keyword) = keywords.first() {
                            if let Some(pos) = content_to_check.find(first_keyword) {
                                let start = pos.saturating_sub(50);
                                let end = (pos + first_keyword.len() + 50).min(content.len());
                                match_content = Some(content[start..end].to_string());
                            }
                        }
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
                if file_name.to_string_lossy() == **exclude {
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

    /// 获取搜索花费的时间（秒）
    pub fn get_elapsed_time(&self) -> u64 {
        self.start_time
            .map(|start| start.elapsed().as_secs())
            .unwrap_or(0)
    }

    /// 发送搜索进度
    fn send_progress<P>(&self, progress_callback: &P, current_path: &str)
    where
        P: Fn(SearchProgress) + Send + Sync,
    {
        let scanned_count = self.scanned_count.load(Ordering::Relaxed);
        let found_count = self.found_count.load(Ordering::Relaxed);

        let elapsed_time = self.start_time
            .map(|start| start.elapsed().as_secs())
            .unwrap_or(0);

        // 估算剩余时间：基于当前扫描速度
        let estimated_remaining = if scanned_count > 0 && elapsed_time > 0 {
            let scanned_per_sec = scanned_count as f64 / elapsed_time as f64;
            // 假设总共需要扫描 10 倍已扫描的数量（粗略估算）
            let total_estimate = (scanned_count as f64 * 10.0) as u64;
            let remaining = total_estimate.saturating_sub(scanned_count);
            (remaining as f64 / scanned_per_sec.max(1.0)) as u64
        } else {
            0
        };

        let progress = SearchProgress {
            scanned_count,
            found_count,
            current_path: current_path.to_string(),
            elapsed_time,
            estimated_remaining,
        };

        progress_callback(progress);
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
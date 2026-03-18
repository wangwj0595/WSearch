use crate::models::{SearchConfig, SearchResult};
use futures_util::stream;
use rayon::prelude::*;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
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
#[derive(Clone)]
pub struct FileScanner {
    config: SearchConfig,
    start_time: Option<Instant>,
}

impl FileScanner {
    pub fn new(config: SearchConfig) -> Self {
        Self {
            config,
            start_time: None,
        }
    }

    /// 异步流式并行搜索 - 使用 tokio + rayon
    /// 返回一个异步流，每次 yield 一个结果
    pub async fn search_streaming(
        self,
        query: String,
        is_cancelled: Arc<AtomicBool>,
    ) -> impl futures_core::Stream<Item = Vec<SearchResult>> + Send {
        // 记录开始时间
        let start_time = Instant::now();

        // 解析关键词
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

        // 在阻塞线程池中执行并行搜索，返回结果列表
        let results: Arc<Mutex<Vec<SearchResult>>> = Arc::new(Mutex::new(Vec::new()));
        let results_clone = results.clone();
        let cancelled = is_cancelled.clone();
        let config = self.config.clone();

        // 使用 tokio::task::spawn_blocking 在线程池中运行 rayon 并行搜索
        let _ = tokio::task::spawn_blocking(move || {
            // 并行遍历所有搜索目录
            config.search_paths.par_iter().for_each(|search_path| {
                if cancelled.load(Ordering::SeqCst) {
                    return;
                }

                let path = Path::new(search_path);
                if !path.exists() {
                    return;
                }

                // 收集目录条目
                let entries: Vec<_> = WalkDir::new(path)
                    .follow_links(false)
                    .into_iter()
                    .filter_entry(|e| !should_exclude(&config.exclude_paths, e.path()))
                    .filter_map(|e| e.ok())
                    .collect();

                // 并行处理条目
                entries.par_iter().for_each(|entry| {
                    if cancelled.load(Ordering::SeqCst) {
                        return;
                    }

                    // 检查数量限制
                    {
                        let r = results_clone.lock().unwrap();
                        if r.len() >= config.max_results {
                            return;
                        }
                    }

                    let file_name = entry.file_name().to_string_lossy().to_string();
                    let file_path = entry.path();
                    let mut is_match = false;
                    let mut match_content = None;

                    // 检查文件名匹配
                    let name_check = if config.case_sensitive {
                        file_name.clone()
                    } else {
                        file_name.to_lowercase()
                    };

                    if keywords.iter().all(|k| name_check.contains(k)) {
                        is_match = true;
                    }

                    // 检查文件内容
                    if !is_match && config.search_content && entry.file_type().is_file() {
                        if let Ok(content) = std::fs::read_to_string(file_path) {
                            let content_check = if config.case_sensitive {
                                content.clone()
                            } else {
                                content.to_lowercase()
                            };

                            if keywords.iter().all(|k| content_check.contains(k)) {
                                is_match = true;
                                if let Some(first_kw) = keywords.first() {
                                    if let Some(pos) = content_check.find(first_kw) {
                                        let start = pos.saturating_sub(50);
                                        let end = (pos + first_kw.len() + 50).min(content.len());
                                        match_content = Some(content[start..end].to_string());
                                    }
                                }
                            }
                        }
                    }

                    // 检查文件类型过滤
                    if is_match && !config.file_types.is_empty() {
                        if let Some(ext) = entry.path().extension() {
                            let ext_str = ext.to_string_lossy().to_lowercase();
                            is_match = config.file_types.iter().any(|t| {
                                let t_low = t.to_lowercase();
                                t_low == ext_str || t_low == format!(".{}", ext_str)
                            });
                        } else {
                            is_match = false;
                        }
                    }

                    if is_match {
                        let metadata = entry.metadata().ok();
                        let size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);
                        let modified = metadata
                            .and_then(|m| m.modified().ok())
                            .map(format_time)
                            .unwrap_or_default();

                        let result = SearchResult {
                            name: file_name,
                            path: file_path.to_string_lossy().to_string(),
                            is_directory: entry.file_type().is_dir(),
                            size,
                            modified_time: modified,
                            match_content,
                        };

                        let mut r = results_clone.lock().unwrap();
                        r.push(result);
                    }
                });
            });
        }).await;

        // 收集结果并按批次返回
        let all_results = results.lock().unwrap().clone();
        let batch_size = 5;

        // 创建批次
        let batches: Vec<Vec<SearchResult>> = all_results
            .chunks(batch_size)
            .map(|chunk| chunk.to_vec())
            .collect();

        // 存储开始时间供 get_elapsed_time 使用
        let _ = start_time;

        stream::iter(batches)
    }

    /// 获取搜索花费的时间
    pub fn get_elapsed_time(&self) -> u64 {
        self.start_time.map(|t| t.elapsed().as_secs()).unwrap_or(0)
    }
}

/// 检查路径是否应该排除
fn should_exclude(exclude_paths: &[String], path: &Path) -> bool {
    let path_str = path.to_string_lossy();

    for exclude in exclude_paths {
        if let Some(name) = path.file_name() {
            if name.to_string_lossy() == **exclude {
                return true;
            }
        }
        if path_str.contains(&format!("/{}", exclude)) || path_str.contains(&format!("\\{}", exclude)) {
            return true;
        }
    }
    false
}

/// 格式化时间
fn format_time(time: std::time::SystemTime) -> String {
    let dur = time.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let secs = dur.as_secs();

    let mins = secs / 60;
    let hours = (mins % 1440) / 60;
    let minutes = mins % 60;
    let seconds = secs % 60;

    let days = secs / 86400;
    let year = 1970 + days / 365;
    let day_of_year = days % 365;
    let month = (day_of_year / 30) + 1;
    let day = (day_of_year % 30) + 1;

    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", year, month, day, hours, minutes, seconds)
}

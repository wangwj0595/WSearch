use crate::models::{SearchConfig, SearchResult};
use crate::services::mft_reader;
use rayon::prelude::*;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender, Receiver};
use std::sync::Mutex;
use std::sync::Arc;
use std::thread;
use std::time::Instant;
use walkdir::WalkDir;

// MFT 索引缓存（全局单例）
lazy_static::lazy_static! {
    static ref MFT_INDEX: Mutex<Option<MftIndex>> = Mutex::new(None);
}

// MFT 索引结构
#[derive(Clone)]
struct MftIndex {
    entries: Vec<MftEntry>,
    last_update: std::time::Instant,
}

#[derive(Clone)]
struct MftEntry {
    name: String,
    path: String,
    size: u64,
    modified_time: String,
    is_directory: bool,
}

/// 搜索进度信息
#[derive(Debug, Clone, serde::Serialize)]
pub struct SearchProgress {
    pub scanned_count: u64,
    pub found_count: u64,
    pub current_path: String,
    pub elapsed_time: u64,
    pub estimated_remaining: u64,
}

/// 搜索结果项（包含额外信息用于流式发送）
#[derive(Debug, Clone)]
pub struct SearchResultItem {
    pub result: SearchResult,
    pub scanned_count: u64,
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

    /// 克隆方法
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            start_time: self.start_time,
        }
    }

    /// 使用专用线程 + Channel 的实时搜索
    /// 返回 (结果Receiver, 进度Receiver, 取消标志)
    pub fn search_with_channel(
        self,
        query: String,
    ) -> (Receiver<SearchResultItem>, Receiver<SearchProgress>, Arc<AtomicBool>) {
        let (result_tx, result_rx) = mpsc::channel();
        let (progress_tx, progress_rx) = mpsc::channel();
        let is_cancelled = Arc::new(AtomicBool::new(false));
        let cancelled = is_cancelled.clone();

        let config = self.config.clone();

        // 启动专用搜索线程
        thread::spawn(move || {
            Self::search_worker(
                config,
                query,
                result_tx,
                progress_tx,
                cancelled,
            );
        });

        (result_rx, progress_rx, is_cancelled)
    }

    /// 搜索工作线程函数
    fn search_worker(
        config: SearchConfig,
        query: String,
        result_tx: Sender<SearchResultItem>,
        progress_tx: Sender<SearchProgress>,
        is_cancelled: Arc<AtomicBool>,
    ) {
        // 根据配置选择搜索方式
        if config.use_mft {
            Self::search_worker_mft(config, query, result_tx, progress_tx, is_cancelled);
        } else {
            Self::search_worker_walkdir(config, query, result_tx, progress_tx, is_cancelled);
        }
    }

    /// WalkDir 搜索（传统方式）
    fn search_worker_walkdir(
        config: SearchConfig,
        query: String,
        result_tx: Sender<SearchResultItem>,
        progress_tx: Sender<SearchProgress>,
        is_cancelled: Arc<AtomicBool>,
    ) {
        let start_time = Instant::now();
        let scanned_count = AtomicU64::new(0);
        let found_count = AtomicU64::new(0);

        // 解析关键词
        let keywords: Vec<String> = query
            .split_whitespace()
            .filter(|s| !s.is_empty())
            .map(|s| {
                if config.case_sensitive {
                    s.to_string()
                } else {
                    s.to_lowercase()
                }
            })
            .collect();

        // 遍历所有搜索目录
        for search_path in &config.search_paths {
            if is_cancelled.load(Ordering::SeqCst) {
                break;
            }

            let path = Path::new(search_path);
            if !path.exists() {
                continue;
            }

            // 使用 rayon 并行处理 WalkDir 条目
            let entries: Vec<_> = WalkDir::new(path)
                .follow_links(false)
                .into_iter()
                .filter_entry(|e| !should_exclude(&config.exclude_paths, e.path()))
                .filter_map(|e| e.ok())
                .collect();

            // 并行处理条目
            entries.par_iter().for_each(|entry| {
                if is_cancelled.load(Ordering::SeqCst) {
                    return;
                }

                // 原子递增已扫描计数
                let scanned = scanned_count.fetch_add(1, Ordering::SeqCst);

                // 每扫描100个文件发送一次进度
                if scanned % 100 == 0 {
                    let elapsed = start_time.elapsed().as_secs();
                    let rate = if elapsed > 0 { scanned / elapsed } else { 1 };
                    let remaining = if rate > 0 { (scanned / rate).saturating_sub(elapsed) } else { 0 };

                    let progress = SearchProgress {
                        scanned_count: scanned,
                        found_count: found_count.load(Ordering::SeqCst),
                        current_path: entry.path().to_string_lossy().to_string(),
                        elapsed_time: elapsed,
                        estimated_remaining: remaining,
                    };
                    let _ = progress_tx.send(progress);
                }

                // 检查数量限制
                let max_results_u64 = config.max_results as u64;
                if found_count.load(Ordering::SeqCst) >= max_results_u64 {
                    return;
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

                // 如果不搜索目录，则跳过目录
                if is_match && !config.search_directories && entry.file_type().is_dir() {
                    is_match = false;
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

                    // 找到结果，立即发送到 channel（真实时）
                    let item = SearchResultItem {
                        result,
                        scanned_count: scanned,
                    };
                    let _ = result_tx.send(item);
                    found_count.fetch_add(1, Ordering::SeqCst);
                }
            });
        }

        // 发送最终进度
        let elapsed = start_time.elapsed().as_secs();
        let progress = SearchProgress {
            scanned_count: scanned_count.load(Ordering::SeqCst),
            found_count: found_count.load(Ordering::SeqCst),
            current_path: String::new(),
            elapsed_time: elapsed,
            estimated_remaining: 0,
        };
        let _ = progress_tx.send(progress);
    }

    /// MFT 搜索（快速方式）
    #[cfg(windows)]
    fn search_worker_mft(
        config: SearchConfig,
        query: String,
        result_tx: Sender<SearchResultItem>,
        progress_tx: Sender<SearchProgress>,
        is_cancelled: Arc<AtomicBool>,
    ) {
        let start_time = Instant::now();
        let found_count = AtomicU64::new(0);

        // 发送初始进度
        let _ = progress_tx.send(SearchProgress {
            scanned_count: 0,
            found_count: 0,
            current_path: "正在建立 MFT 索引...".to_string(),
            elapsed_time: 0,
            estimated_remaining: 0,
        });

        // 获取或更新 MFT 索引
        let entries = Self::get_or_build_mft_index(&config.search_paths);

        if entries.is_empty() {
            // 索引为空，回退到 WalkDir
            return Self::search_worker_walkdir(config, query, result_tx, progress_tx, is_cancelled);
        }

        // 解析关键词
        let keywords: Vec<String> = query
            .split_whitespace()
            .filter(|s| !s.is_empty())
            .map(|s| {
                if config.case_sensitive {
                    s.to_string()
                } else {
                    s.to_lowercase()
                }
            })
            .collect();

        let max_results = config.max_results as u64;
        let scanned_total = AtomicU64::new(0);
        let total_entries = entries.len() as u64;

        // 在内存中搜索（非常快）
        for entry in entries {
            if is_cancelled.load(Ordering::SeqCst) {
                break;
            }

            if found_count.load(Ordering::SeqCst) >= max_results {
                break;
            }

            let scanned = scanned_total.fetch_add(1, Ordering::SeqCst);

            // 每扫描1000个文件发送一次进度
            if scanned % 1000 == 0 {
                let elapsed = start_time.elapsed().as_secs();
                let progress = SearchProgress {
                    scanned_count: scanned,
                    found_count: found_count.load(Ordering::SeqCst),
                    current_path: entry.path.clone(),
                    elapsed_time: elapsed,
                    estimated_remaining: 0,
                };
                let _ = progress_tx.send(progress);
            }

            // 检查文件名匹配
            let name_check = if config.case_sensitive {
                entry.name.clone()
            } else {
                entry.name.to_lowercase()
            };

            let mut is_match = keywords.iter().all(|k| name_check.contains(k));

            // 检查文件类型过滤
            if is_match && !config.file_types.is_empty() {
                if let Some(ext) = entry.path.rsplit('.').next() {
                    let ext_str = ext.to_lowercase();
                    is_match = config.file_types.iter().any(|t| {
                        let t_low = t.to_lowercase();
                        t_low == ext_str || t_low == format!(".{}", ext_str)
                    });
                } else {
                    is_match = false;
                }
            }

            // 如果不搜索目录，则跳过目录
            if is_match && !config.search_directories && entry.is_directory {
                continue;
            }

            if is_match {
                let result = SearchResult {
                    name: entry.name,
                    path: entry.path,
                    is_directory: entry.is_directory,
                    size: entry.size,
                    modified_time: entry.modified_time,
                    match_content: None,
                };

                let item = SearchResultItem {
                    result,
                    scanned_count: scanned,
                };
                let _ = result_tx.send(item);
                found_count.fetch_add(1, Ordering::SeqCst);
            }
        }

        // 发送最终进度
        let elapsed = start_time.elapsed().as_secs();
        let progress = SearchProgress {
            scanned_count: scanned_total.load(Ordering::SeqCst),
            found_count: found_count.load(Ordering::SeqCst),
            current_path: String::new(),
            elapsed_time: elapsed,
            estimated_remaining: 0,
        };
        let _ = progress_tx.send(progress);
    }

    /// 非 Windows 平台回退到 WalkDir
    #[cfg(not(windows))]
    fn search_worker_mft(
        config: SearchConfig,
        query: String,
        result_tx: Sender<SearchResultItem>,
        progress_tx: Sender<SearchProgress>,
        is_cancelled: Arc<AtomicBool>,
    ) {
        // 非 Windows 平台直接使用 WalkDir
        Self::search_worker_walkdir(config, query, result_tx, progress_tx, is_cancelled);
    }

    /// 获取或构建 MFT 索引
    #[cfg(windows)]
    fn get_or_build_mft_index(search_paths: &[String]) -> Vec<MftEntry> {
        // 检查缓存是否有效（5分钟内有效）
        {
            let index_guard = MFT_INDEX.lock().unwrap();
            if let Some(ref index) = *index_guard {
                if index.last_update.elapsed().as_secs() < 300 {
                    return index.entries.clone();
                }
            }
        }

        // 使用真正的 NTFS MFT/USN Journal 读取
        let mut entries = Vec::new();

        for search_path in search_paths {
            // 获取卷根路径（例如 D:\）
            let volume_root = Self::get_volume_root(search_path);

            if !volume_root.is_empty() && mft_reader::is_ntfs_volume(&volume_root) {
                log::info!("正在使用 NTFS MFT 扫描卷: {}", volume_root);

                // 使用真正的 MFT 读取
                let mft_entries = mft_reader::scan_volume_files(&volume_root);

                for mft_entry in mft_entries {
                    // 过滤出在搜索路径下的文件
                    if mft_entry.path.starts_with(search_path) || search_path.len() <= 3 {
                        entries.push(MftEntry {
                            name: mft_entry.name,
                            path: mft_entry.path,
                            size: mft_entry.size,
                            modified_time: mft_entry.modified_time,
                            is_directory: mft_entry.is_directory,
                        });
                    }
                }
            } else {
                // 非 NTFS 卷或无法识别，使用 WalkDir 回退
                log::info!("卷 {} 不是 NTFS 或无法识别，使用 WalkDir", volume_root);
                Self::build_index_walkdir(search_path, &mut entries);
            }
        }

        // 更新缓存
        {
            let mut index_guard = MFT_INDEX.lock().unwrap();
            *index_guard = Some(MftIndex {
                entries: entries.clone(),
                last_update: std::time::Instant::now(),
            });
        }

        log::info!("MFT 索引构建完成，共 {} 个文件", entries.len());
        entries
    }

    /// 获取路径的卷根路径
    #[cfg(windows)]
    fn get_volume_root(path: &str) -> String {
        let p = Path::new(path);
        if let Some(root) = p.components().next() {
            if let Some(drive) = root.as_os_str().to_str() {
                return format!("{}\\", drive);
            }
        }
        String::new()
    }

    /// 使用 WalkDir 构建索引（回退方案）
    #[cfg(windows)]
    fn build_index_walkdir(search_path: &str, entries: &mut Vec<MftEntry>) {
        let path = Path::new(search_path);
        if !path.exists() {
            return;
        }

        for entry in WalkDir::new(path)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let metadata = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };

            let name = entry.file_name().to_string_lossy().to_string();
            let path_str = entry.path().to_string_lossy().to_string();
            let is_dir = entry.file_type().is_dir();
            let size = metadata.len();
            let modified = metadata
                .modified()
                .map(format_time)
                .unwrap_or_default();

            entries.push(MftEntry {
                name,
                path: path_str,
                size,
                modified_time: modified,
                is_directory: is_dir,
            });
        }
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

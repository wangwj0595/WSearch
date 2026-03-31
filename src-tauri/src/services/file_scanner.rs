use crate::models::{SearchConfig, SearchResult};
use crate::services::mft_reader;
use crate::services::index_cache::{self, get_cache_manager, SearchResultEntry as CacheSearchResult};
use crate::services::usn_monitor;
use crate::services::is_running_as_admin;
use rayon::prelude::*;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender, Receiver};
use std::sync::Mutex;
use std::sync::Arc;
use std::thread;
use std::time::Instant;
use walkdir::WalkDir;
use aho_corasick::AhoCorasick;

// MFT 条目结构（用于内存索引）
#[derive(Clone)]
struct MftEntry {
    name: String,
    path: String,
    size: u64,
    modified_time: String,
    is_directory: bool,
}

// 高效搜索器（使用 Aho-Corasick 多模式匹配）
struct FastSearcher {
    matcher: Option<AhoCorasick>,
    keywords: Vec<String>,
}

impl FastSearcher {
    fn new(query: &str, case_sensitive: bool) -> Self {
        let keywords: Vec<String> = query
            .split_whitespace()
            .filter(|s| !s.is_empty())
            .map(|s| {
                if case_sensitive {
                    s.to_string()
                } else {
                    s.to_lowercase()
                }
            })
            .collect();

        if keywords.is_empty() {
            return Self {
                matcher: None,
                keywords: Vec::new(),
            };
        }

        // 构建 Aho-Corasick 自动机
        let patterns: Vec<&str> = keywords.iter().map(|s| s.as_str()).collect();
        let matcher = AhoCorasick::new(&patterns).ok();

        Self { matcher, keywords }
    }

    fn is_match(&self, text: &str) -> bool {
        if self.keywords.is_empty() {
            return false;
        }

        let text = text;  // 使用引用避免复制

        if let Some(ref matcher) = self.matcher {
            // 使用 Aho-Corasick 进行多模式匹配
            let mut found_count = 0;
            for _ in matcher.find_iter(text) {
                found_count += 1;
                if found_count == self.keywords.len() {
                    return true;
                }
            }
            false
        } else {
            // 回退到普通匹配
            self.keywords.iter().all(|k| text.contains(k))
        }
    }
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

                // 检查文件大小过滤
                if is_match {
                    let metadata = entry.metadata().ok();
                    let size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);

                    // 大小范围过滤（目录不参与大小过滤）
                    if !entry.file_type().is_dir() {
                        if config.min_size > 0 && size < config.min_size {
                            is_match = false;
                        }
                        if config.max_size > 0 && size > config.max_size {
                            is_match = false;
                        }
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
        let cache_manager = get_cache_manager();
        let mut entries = Vec::new();

        // 首先尝试加载缓存（如果还没有加载或之前加载失败）
        if !cache_manager.is_valid() {
            log::info!("缓存无效，尝试加载缓存文件...");
            if !cache_manager.load_cache() {
                log::info!("缓存加载失败，将重建索引");
            }
        }

        // 检查是否有新卷需要建立索引
        let volumes_to_index = cache_manager.get_volumes_to_index(&config.search_paths);

        if !volumes_to_index.is_empty() {
            // 有新卷需要索引，先建立索引
            log::info!("检测到 {} 个新卷需要索引: {:?}", volumes_to_index.len(), volumes_to_index);

            // 发送初始进度
            let _ = progress_tx.send(SearchProgress {
                scanned_count: 0,
                found_count: 0,
                current_path: "正在为新卷建立索引...".to_string(),
                elapsed_time: 0,
                estimated_remaining: 0,
            });

            // 调用 get_or_build_mft_index 来增量添加新卷
            entries = Self::get_or_build_mft_index(&config.search_paths);
        }

        // 检查缓存是否有效
        if cache_manager.is_valid() {
            // 启动增量更新服务（如果尚未启动）
            // Self::start_incremental_service_if_needed(&config.search_paths);

            // 检查是否有新记录
            if usn_monitor::has_new_records() {
                // 有新记录：直接使用缓存搜索（已包含最新变化）
                log::info!("检测到新记录，直接搜索");
            } else {
                // 没有新记录：触发一次增量更新，同步最新变化
                log::info!("没有新记录，触发增量更新...");

                // 发送初始进度
                let _ = progress_tx.send(SearchProgress {
                    scanned_count: 0,
                    found_count: 0,
                    current_path: "正在同步文件变化...".to_string(),
                    elapsed_time: 0,
                    estimated_remaining: 0,
                });

                // 手动触发一次增量更新
                let _ = usn_monitor::trigger_incremental_update();

                log::info!("增量更新完成，使用缓存索引搜索，文件数: {}", cache_manager.file_count());
            }

            // 发送搜索进度
            let _ = progress_tx.send(SearchProgress {
                scanned_count: 0,
                found_count: 0,
                current_path: "正在从缓存搜索...".to_string(),
                elapsed_time: 0,
                estimated_remaining: 0,
            });

            // 使用缓存搜索（传入搜索路径进行过滤）
            let results = cache_manager.search(&query, config.case_sensitive, config.max_results, &config.search_paths);

            for (idx, entry) in results.into_iter().enumerate() {
                if is_cancelled.load(Ordering::SeqCst) {
                    break;
                }

                // 检查文件类型过滤
                let mut is_match = true;
                if !config.file_types.is_empty() {
                    if let Some(ext) = std::path::Path::new(&entry.path).extension() {
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
                if is_match && !config.search_directories && entry.is_directory {
                    continue;
                }

                // 检查文件大小过滤（目录不参与大小过滤）
                if is_match && !entry.is_directory {
                    if config.min_size > 0 && entry.size < config.min_size {
                        is_match = false;
                    }
                    if config.max_size > 0 && entry.size > config.max_size {
                        is_match = false;
                    }
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
                        scanned_count: idx as u64,
                    };
                    let _ = result_tx.send(item);
                    found_count.fetch_add(1, Ordering::SeqCst);
                }
            }

            // 发送最终进度
            let elapsed = start_time.elapsed().as_millis() as u64;
            let progress = SearchProgress {
                scanned_count: cache_manager.file_count(),
                found_count: found_count.load(Ordering::SeqCst),
                current_path: String::new(),
                elapsed_time: elapsed,
                estimated_remaining: 0,
            };
            let _ = progress_tx.send(progress);

            return;
        }

        if entries.is_empty() {
            // 索引为空，回退到 WalkDir
            return Self::search_worker_walkdir(config, query, result_tx, progress_tx, is_cancelled);
        }

        // 解析关键词
        // let keywords: Vec<String> = query
        //     .split_whitespace()
        //     .filter(|s| !s.is_empty())
        //     .map(|s| {
        //         if config.case_sensitive {
        //             s.to_string()
        //         } else {
        //             s.to_lowercase()
        //         }
        //     })
        //     .collect();

        // let max_results = config.max_results as u64;
        // let scanned_total = AtomicU64::new(0);
        // let total_entries = entries.len() as u64;

        // // 在内存中搜索（非常快）
        // for entry in entries {
        //     if is_cancelled.load(Ordering::SeqCst) {
        //         break;
        //     }

        //     if found_count.load(Ordering::SeqCst) >= max_results {
        //         break;
        //     }

        //     let scanned = scanned_total.fetch_add(1, Ordering::SeqCst);

        //     // 每扫描1000个文件发送一次进度
        //     if scanned % 1000 == 0 {
        //         let elapsed = start_time.elapsed().as_secs();
        //         let progress = SearchProgress {
        //             scanned_count: scanned,
        //             found_count: found_count.load(Ordering::SeqCst),
        //             current_path: entry.path.clone(),
        //             elapsed_time: elapsed,
        //             estimated_remaining: 0,
        //         };
        //         let _ = progress_tx.send(progress);
        //     }

        //     // 检查文件名匹配
        //     let name_check = if config.case_sensitive {
        //         entry.name.clone()
        //     } else {
        //         entry.name.to_lowercase()
        //     };

        //     let mut is_match = keywords.iter().all(|k| name_check.contains(k));

        //     // 检查文件类型过滤
        //     if is_match && !config.file_types.is_empty() {
        //         if let Some(ext) = entry.path.rsplit('.').next() {
        //             let ext_str = ext.to_lowercase();
        //             is_match = config.file_types.iter().any(|t| {
        //                 let t_low = t.to_lowercase();
        //                 t_low == ext_str || t_low == format!(".{}", ext_str)
        //             });
        //         } else {
        //             is_match = false;
        //         }
        //     }

        //     // 如果不搜索目录，则跳过目录
        //     if is_match && !config.search_directories && entry.is_directory {
        //         continue;
        //     }

        //     // 检查文件大小过滤（目录不参与大小过滤）
        //     if is_match && !entry.is_directory {
        //         if config.min_size > 0 && entry.size < config.min_size {
        //             is_match = false;
        //         }
        //         if config.max_size > 0 && entry.size > config.max_size {
        //             is_match = false;
        //         }
        //     }

        //     if is_match {
        //         let result = SearchResult {
        //             name: entry.name,
        //             path: entry.path,
        //             is_directory: entry.is_directory,
        //             size: entry.size,
        //             modified_time: entry.modified_time,
        //             match_content: None,
        //         };

        //         let item = SearchResultItem {
        //             result,
        //             scanned_count: scanned,
        //         };
        //         let _ = result_tx.send(item);
        //         found_count.fetch_add(1, Ordering::SeqCst);
        //     }
        // }

        // // 发送最终进度
        // let elapsed = start_time.elapsed().as_secs();
        // let progress = SearchProgress {
        //     scanned_count: scanned_total.load(Ordering::SeqCst),
        //     found_count: found_count.load(Ordering::SeqCst),
        //     current_path: String::new(),
        //     elapsed_time: elapsed,
        //     estimated_remaining: 0,
        // };
        // let _ = progress_tx.send(progress);

        // 索引构建完成后启动增量更新服务
        // Self::start_incremental_service_if_needed(&config.search_paths);
    }

    /// 启动增量更新服务（如果尚未启动）
    #[cfg(windows)]
    fn start_incremental_service_if_needed(search_paths: &[String]) {
        // 检查服务是否已经在运行，避免重复启动
        if usn_monitor::is_incremental_service_running() {
            log::debug!("增量更新服务已在运行，跳过启动");
            return;
        }

        // 获取需要监控的卷列表
        let volumes: Vec<String> = search_paths.iter()
            .filter_map(|p| {
                let path = Path::new(p);
                if let Some(root) = path.components().next() {
                    if let Some(drive) = root.as_os_str().to_str() {
                        if drive.len() >= 2 && drive.chars().nth(1) == Some(':') {
                            return Some(format!("{}:\\", drive.chars().next().unwrap()));
                        }
                    }
                }
                None
            })
            .collect();

        if volumes.is_empty() {
            log::debug!("没有可用的卷来启动增量更新服务");
            return;
        }

        log::info!("启动增量更新服务，卷: {:?}", volumes);

        // 尝试启动服务，如果已经启动会返回错误但不影响运行
        match usn_monitor::start_incremental_service(volumes.clone()) {
            Ok(statuses) => {
                for status in statuses {
                    if status.is_running {
                        log::info!("增量更新服务已在卷 {} 上运行", status.volume);
                    } else if let Some(err) = status.error_message {
                        log::warn!("卷 {} 增量更新服务启动失败: {}", status.volume, err);
                    }
                }
            }
            Err(e) => {
                log::debug!("增量更新服务启动: {}", e);
            }
        }
    }

    #[cfg(not(windows))]
    fn start_incremental_service_if_needed(_search_paths: &[String]) {
        // 非 Windows 平台不做任何操作
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

    /// 获取或构建 MFT 索引（使用新的缓存系统）
    #[cfg(windows)]
    fn get_or_build_mft_index(search_paths: &[String]) -> Vec<MftEntry> {
        let cache_manager = get_cache_manager();

        // 首先尝试加载缓存（如果还没有加载）
        if !cache_manager.is_valid() {
            log::info!("缓存无效，尝试加载缓存文件...");
            if !cache_manager.load_cache() {
                log::info!("缓存加载失败，需要构建新索引");
            }
        }

        // 检查管理员权限
        if !is_running_as_admin() {
            log::warn!("没有管理员权限，无法使用 MFT 快速搜索，回退到 WalkDir");
            return Vec::new();
        }

        // 获取需要建立索引的卷列表（排除已索引的卷）
        let volumes_to_index = cache_manager.get_volumes_to_index(search_paths);

        // 如果所有卷都已索引，缓存有效，直接返回空
        if cache_manager.is_valid() && volumes_to_index.is_empty() {
            log::info!("所有卷已索引，使用已有缓存，共 {} 个文件", cache_manager.file_count());
            return Vec::new();
        }

        // 增量添加新发现的卷
        // let mut has_new_volume = false;

        for volume_root in &volumes_to_index {
            log::info!("检测到新卷 {}，正在建立索引...", volume_root);

            // 检查 NTFS
            let is_ntfs = mft_reader::is_ntfs_volume(volume_root);

            if is_ntfs {
                // 使用 MFT 读取
                let mft_entries = mft_reader::scan_volume_files(volume_root);

                if !mft_entries.is_empty() {
                    // 增量添加新卷的索引
                    cache_manager.add_volume_from_mft(mft_entries.clone(), volume_root);
                    // has_new_volume = true;
                    log::info!("卷 {} 索引构建完成，共 {} 个文件", volume_root, mft_entries.len());
                } else {
                    log::warn!("卷 {} MFT 读取失败", volume_root);
                }
            } else {
                log::info!("卷 {} 不是 NTFS，跳过", volume_root);
            }
        }

        // 检查缓存是否有效
        if cache_manager.is_valid() {
            log::info!("索引已更新，使用缓存搜索，当前共 {} 个文件", cache_manager.file_count());
            return Vec::new();
        }

        // 缓存仍然无效，需要回退到 WalkDir
        let mut entries = Vec::new();
        for search_path in search_paths {
            Self::build_index_walkdir(search_path, &mut entries);
        }

        log::info!("WalkDir 索引构建完成，共 {} 个文件", entries.len());
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

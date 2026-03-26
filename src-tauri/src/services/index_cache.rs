//! 索引缓存模块
//! 实现类似 Everything 的持久化缓存和内存映射机制

#[cfg(windows)]
use crate::services::usn_monitor;
use crate::services::mft_reader::MftFileEntry;
use memmap2::Mmap;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write, BufWriter};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

/// 缓存版本号
/// v1: JSON 格式（旧版本）
/// v2: MessagePack 格式（新版本，更快）
const CACHE_VERSION: u32 = 2;

/// 索引条目（用于二进制序列化）
#[derive(Debug, Clone)]
struct IndexEntry {
    name: String,
    path: String,
    size: u64,
    modified_time: i64,
    is_directory: bool,
}

/// 索引缓存结构
pub struct IndexCache {
    /// 所有文件条目
    entries: Vec<IndexEntry>,
    /// 按卷分组的索引
    volume_indices: HashMap<String, VolumeIndex>,
    /// 哈希索引：按名称前缀
    name_index: HashMap<String, Vec<usize>>,
    /// 最后更新时间
    last_update: std::time::Instant,
    /// 索引是否有效
    is_valid: Arc<AtomicBool>,
    /// 索引版本
    version: u32,
}

/// 卷索引
struct VolumeIndex {
    entries: Vec<usize>,  // 条目在主列表中的索引
    root_path: String,
}

impl IndexCache {
    /// 创建新的索引缓存
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            volume_indices: HashMap::new(),
            name_index: HashMap::new(),
            last_update: std::time::Instant::now(),
            is_valid: Arc::new(AtomicBool::new(false)),
            version: CACHE_VERSION,
        }
    }

    /// 从 MFT 条目构建索引
    pub fn from_mft_entries(mft_entries: Vec<MftFileEntry>, volume_root: &str) -> Self {
        let mut cache = Self::new();

        for entry in mft_entries {
            let idx = cache.entries.len();
            let modified = entry.modified_time;

            // 添加到主列表
            cache.entries.push(IndexEntry {
                name: entry.name.clone(),
                path: entry.path.clone(),
                size: entry.size,
                modified_time: modified,
                is_directory: entry.is_directory,
            });

            // 按卷分组
            let volume = cache.volume_indices
                .entry(volume_root.to_string())
                .or_insert_with(|| VolumeIndex {
                    entries: Vec::new(),
                    root_path: volume_root.to_string(),
                });
            volume.entries.push(idx);

            // 构建名称前缀索引（前 3 个字符，使用字符边界）
            let name_lower = entry.name.to_lowercase();
            if name_lower.len() >= 3 {
                let prefix: String = name_lower.chars().take(3).collect();
                if prefix.len() == 3 {
                    cache.name_index.entry(prefix).or_default().push(idx);
                }
            }
        }

        cache.last_update = std::time::Instant::now();
        cache.is_valid.store(true, Ordering::SeqCst);
        log::info!("索引缓存构建完成，共 {} 个文件", cache.entries.len());

        cache
    }

    /// 搜索文件（使用哈希索引加速）
    /// search_paths: 可选的搜索路径列表，用于过滤结果
    pub fn search(&self, query: &str, case_sensitive: bool, search_paths: &[String]) -> Vec<&IndexEntry> {
        if query.is_empty() {
            return Vec::new();
        }

        let query_lower = query.to_lowercase();
        let keywords: Vec<&str> = query_lower.split_whitespace().collect();

        if keywords.is_empty() {
            return Vec::new();
        }

        let first_keyword = keywords[0];

        // 优先使用前缀索引（使用字符边界）
        let candidate_indices: Vec<usize> = if first_keyword.len() >= 3 {
            let prefix: String = first_keyword.chars().take(3).collect();
            // 从哈希索引获取候选
            if let Some(indices) = self.name_index.get(&prefix) {
                indices.clone()
            } else {
                // 回退到全表扫描
                (0..self.entries.len()).collect()
            }
        } else {
            // 关键词太短，回退到全表扫描
            (0..self.entries.len()).collect()
        };

        // 在候选中搜索
        let mut results: Vec<&IndexEntry> = candidate_indices
            .iter()
            .filter_map(|&idx| {
                let entry = &self.entries[idx];

                // 路径过滤：如果指定了搜索路径，只返回在搜索路径下的文件
                // MFT 返回的路径格式是 \\.\D:\folder，需要去掉 \\.\ 前缀再匹配
                if !search_paths.is_empty() {
                    let normalized_path = entry.path.trim_start_matches("\\\\.\\");

                    let in_search_path = search_paths.iter().any(|sp| {
                        // 标准化搜索路径
                        let normalized_sp = sp.trim_end_matches('\\');

                        // 检查文件路径是否以搜索路径开头
                        // 需要精确匹配目录边界
                        if normalized_path.starts_with(normalized_sp) {
                            // 检查是否是目录边界（后面是反斜杠或结束）
                            let after_sp = &normalized_path[normalized_sp.len()..];
                            after_sp.is_empty() || after_sp.starts_with('\\')
                        } else {
                            false
                        }
                    });

                    if !in_search_path {
                        return None;
                    }
                }

                let name = if case_sensitive {
                    entry.name.clone()
                } else {
                    entry.name.to_lowercase()
                };

                // 检查所有关键词是否都匹配
                let all_match = keywords.iter().all(|&kw| name.contains(kw));

                if all_match {
                    Some(entry)
                } else {
                    None
                }
            })
            .collect();

        // 按相关性排序（名称以查询开头的优先）
        results.sort_by(|a, b| {
            let a_starts = a.name.to_lowercase().starts_with(first_keyword);
            let b_starts = b.name.to_lowercase().starts_with(first_keyword);

            if a_starts != b_starts {
                b_starts.cmp(&a_starts)
            } else {
                // 然后按名称长度排序（短的优先）
                a.name.len().cmp(&b.name.len())
            }
        });

        results
    }

    /// 获取条目数量
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 检查是否为空
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 检查是否有效
    pub fn is_valid(&self) -> bool {
        self.is_valid.load(Ordering::SeqCst)
    }

    /// 获取最后更新时间
    pub fn last_update_time(&self) -> std::time::Instant {
        self.last_update
    }

    /// 获取指定卷的条目
    pub fn get_volume_entries(&self, volume_root: &str) -> Vec<&IndexEntry> {
        if let Some(volume_idx) = self.volume_indices.get(volume_root) {
            volume_idx.entries
                .iter()
                .filter_map(|&idx| self.entries.get(idx))
                .collect()
        } else {
            Vec::new()
        }
    }

    /// 从路径提取卷根路径（内部方法）
    fn extract_volume(path: &str) -> String {
        let normalized = path.trim_start_matches("\\\\.\\");

        if normalized.len() >= 2 && normalized.chars().nth(1) == Some(':') {
            let drive = normalized.chars().next().unwrap();
            return format!("{}:\\", drive);
        }

        if normalized.starts_with("\\\\") {
            if let Some(end) = normalized[2..].find('\\') {
                return normalized[..2 + end].to_string();
            }
            return normalized.to_string();
        }

        String::new()
    }
}

impl Default for IndexCache {
    fn default() -> Self {
        Self::new()
    }
}

/// 缓存管理器（单例）
pub struct CacheManager {
    /// 内存索引缓存
    index: RwLock<IndexCache>,
    /// 缓存文件路径
    cache_path: PathBuf,
    /// 是否启用内存映射
    use_mmap: bool,
    /// 是否正在构建索引
    is_building: AtomicBool,
    /// 文件计数
    file_count: AtomicU64,
}

impl CacheManager {
    /// 创建缓存管理器
    pub fn new() -> Self {
        let cache_dir = Self::get_cache_dir();

        // 确保缓存目录存在
        if let Err(e) = fs::create_dir_all(&cache_dir) {
            log::warn!("创建缓存目录失败: {}", e);
        }

        let cache_path = cache_dir.join("index.esl");

        Self {
            index: RwLock::new(IndexCache::new()),
            cache_path,
            use_mmap: true,
            is_building: AtomicBool::new(false),
            file_count: AtomicU64::new(0),
        }
    }

    /// 获取缓存目录
    fn get_cache_dir() -> PathBuf {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("WSearch")
            .join("cache")
    }

    /// 加载缓存（尝试内存映射）
    pub fn load_cache(&self) -> bool {
        if !self.cache_path.exists() {
            log::info!("缓存文件不存在，需要重建索引");
            return false;
        }

        log::info!("尝试加载缓存: {:?}", self.cache_path);

        if self.use_mmap {
            // 使用内存映射加载
            self.load_cache_mmap()
        } else {
            // 使用普通文件加载
            self.load_cache_file()
        }
    }

    /// 使用内存映射加载缓存
    fn load_cache_mmap(&self) -> bool {
        let file = match OpenOptions::new().read(true).open(&self.cache_path) {
            Ok(f) => f,
            Err(e) => {
                log::warn!("打开缓存文件失败: {}", e);
                return false;
            }
        };

        let mmap = unsafe {
            match Mmap::map(&file) {
                Ok(m) => m,
                Err(e) => {
                    log::warn!("内存映射失败: {}", e);
                    return self.load_cache_file();
                }
            }
        };

        // 解析二进制格式
        self.parse_cache_data(&mmap)
    }

    /// 使用普通文件加载缓存
    fn load_cache_file(&self) -> bool {
        let mut file = match File::open(&self.cache_path) {
            Ok(f) => f,
            Err(e) => {
                log::warn!("打开缓存文件失败: {}", e);
                return false;
            }
        };

        let mut data = Vec::new();
        if let Err(e) = file.read_to_end(&mut data) {
            log::warn!("读取缓存文件失败: {}", e);
            return false;
        }

        self.parse_cache_data(&data)
    }

    /// 解析缓存数据（支持 v1 JSON 和 v2 MessagePack）
    fn parse_cache_data(&self, data: &[u8]) -> bool {
        // 简单格式: [version:4][count:4][entries...]
        if data.len() < 8 {
            log::warn!("缓存文件太小");
            return false;
        }

        let version = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        let _count = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);

        // 根据版本选择解析方式
        match version {
            1 => self.parse_json_format(&data[8..]),
            2 => self.parse_msgpack_format(&data[8..]),
            _ => {
                log::warn!("未知缓存版本: {}", version);
                false
            }
        }
    }

    /// 解析 JSON 格式（v1 兼容）
    fn parse_json_format(&self, data: &[u8]) -> bool {
        let entries: Vec<MftFileEntry> = match serde_json::from_slice(data) {
            Ok(e) => e,
            Err(e) => {
                log::warn!("解析 JSON 缓存失败: {}", e);
                return false;
            }
        };

        self.build_index_from_entries(&entries)
    }

    /// 解析 MessagePack 格式（v2）
    fn parse_msgpack_format(&self, data: &[u8]) -> bool {
        let entries: Vec<MftFileEntry> = match rmp_serde::from_slice(data) {
            Ok(e) => e,
            Err(e) => {
                log::warn!("解析 MessagePack 缓存失败: {}", e);
                return false;
            }
        };

        self.build_index_from_entries(&entries)
    }

    /// 从条目构建内存索引（公共方法，供两种格式共用）
    fn build_index_from_entries(&self, entries: &[MftFileEntry]) -> bool {
        // 构建内存索引
        let mut index = IndexCache::new();
        for entry in entries {
            let idx = index.entries.len();
            index.entries.push(IndexEntry {
                name: entry.name.clone(),
                path: entry.path.clone(),
                size: entry.size,
                modified_time: entry.modified_time,
                is_directory: entry.is_directory,
            });

            // 名称前缀索引（使用字符边界）
            let name_lower = entry.name.to_lowercase();
            if name_lower.len() >= 3 {
                let prefix: String = name_lower.chars().take(3).collect();
                if prefix.len() == 3 {
                    index.name_index.entry(prefix).or_default().push(idx);
                }
            }

            // 从路径中提取卷根路径并添加到卷索引
            let volume_root = Self::extract_volume_root(&entry.path);
            if !volume_root.is_empty() {
                let volume = index.volume_indices
                    .entry(volume_root.clone())
                    .or_insert_with(|| VolumeIndex {
                        entries: Vec::new(),
                        root_path: volume_root.clone(),
                    });
                volume.entries.push(idx);
            }
        }

        index.is_valid.store(true, Ordering::SeqCst);
        index.last_update = std::time::Instant::now();

        // 替换索引
        let mut guard = self.index.write();
        *guard = index;
        self.file_count.store(entries.len() as u64, Ordering::SeqCst);

        log::info!("缓存加载成功，共 {} 个文件", entries.len());
        true
    }

    /// 从文件路径提取卷根路径
    fn extract_volume_root(path: &str) -> String {
        // 处理路径格式: \\.\D:\folder 或 D:\folder
        let normalized = path.trim_start_matches("\\\\.\\");

        // 检查是否是驱动器路径 (如 D:\)
        if normalized.len() >= 2 && normalized.chars().nth(1) == Some(':') {
            let drive = normalized.chars().next().unwrap();
            return format!("{}:\\", drive);
        }

        // 检查 UNC 路径
        if normalized.starts_with("\\\\") {
            // 提取 UNC 路径的第一部分作为卷标识
            if let Some(end) = normalized[2..].find('\\') {
                return normalized[..2 + end].to_string();
            }
            return normalized.to_string();
        }

        String::new()
    }

    /// 保存缓存到文件（使用 MessagePack 序列化 + BufWriter 缓冲写入）
    pub fn save_cache(&self) -> bool {
        let index = self.index.read();

        if index.is_empty() {
            log::warn!("索引为空，无需保存");
            return false;
        }

        log::info!("保存索引缓存到: {:?}", self.cache_path);

        // 序列化数据（使用 MessagePack，更快更小）
        let entries: Vec<MftFileEntry> = index.entries
            .iter()
            .map(|e| MftFileEntry {
                name: e.name.clone(),
                path: e.path.clone(),
                size: e.size,
                modified_time: e.modified_time,
                is_directory: e.is_directory,
            })
            .collect();

        let msgpack_data = match rmp_serde::to_vec(&entries) {
            Ok(d) => d,
            Err(e) => {
                log::warn!("序列化索引失败: {}", e);
                return false;
            }
        };

        // 构建缓存文件: [version:4][count:4][msgpack_data]
        let mut cache_data = Vec::with_capacity(8 + msgpack_data.len());
        cache_data.extend_from_slice(&CACHE_VERSION.to_le_bytes());
        cache_data.extend_from_slice(&(entries.len() as u32).to_le_bytes());
        cache_data.extend_from_slice(&msgpack_data);

        // 使用 BufWriter 缓冲写入，提升 I/O 性能
        let file = match File::create(&self.cache_path) {
            Ok(f) => f,
            Err(e) => {
                log::warn!("创建缓存文件失败: {}", e);
                return false;
            }
        };

        let mut writer = BufWriter::new(file);
        if let Err(e) = writer.write_all(&cache_data) {
            log::warn!("写入缓存文件失败: {}", e);
            return false;
        }

        // 刷新缓冲区，确保数据写入磁盘
        if let Err(e) = writer.flush() {
            log::warn!("刷新缓存文件失败: {}", e);
            return false;
        }

        log::info!("缓存保存成功");
        true
    }

    /// 更新索引（从 MFT）- 完全替换
    pub fn update_from_mft(&self, mft_entries: Vec<MftFileEntry>, volume_root: &str) {
        self.is_building.store(true, Ordering::SeqCst);

        let new_index = IndexCache::from_mft_entries(mft_entries, volume_root);

        {
            let mut index = self.index.write();
            *index = new_index;
        }

        self.file_count.store(self.index.read().len() as u64, Ordering::SeqCst);

        // 保存到缓存
        self.save_cache();

        // 保存 USN 状态
        #[cfg(windows)]
        self.save_usn_state(volume_root);

        self.is_building.store(false, Ordering::SeqCst);
    }

    /// 增量添加卷索引（当检测到新盘符时调用）
    pub fn add_volume_from_mft(&self, mft_entries: Vec<MftFileEntry>, volume_root: &str) {
        self.is_building.store(true, Ordering::SeqCst);

        // 先保存条目数量，用于日志
        let entry_count = mft_entries.len();

        let mut index = self.index.write();

        // 添加新卷的条目到现有索引
        for entry in &mft_entries {
            let idx = index.entries.len();
            let modified = entry.modified_time;

            // 添加到主列表
            index.entries.push(IndexEntry {
                name: entry.name.clone(),
                path: entry.path.clone(),
                size: entry.size,
                modified_time: modified,
                is_directory: entry.is_directory,
            });

            // 按卷分组
            let volume = index.volume_indices
                .entry(volume_root.to_string())
                .or_insert_with(|| VolumeIndex {
                    entries: Vec::new(),
                    root_path: volume_root.to_string(),
                });
            volume.entries.push(idx);

            // 构建名称前缀索引（前 3 个字符，使用字符边界）
            let name_lower = entry.name.to_lowercase();
            if name_lower.len() >= 3 {
                let prefix: String = name_lower.chars().take(3).collect();
                if prefix.len() == 3 {
                    index.name_index.entry(prefix).or_default().push(idx);
                }
            }
        }

        index.last_update = std::time::Instant::now();
        index.is_valid.store(true, Ordering::SeqCst);

        // 释放写锁
        drop(index);

        self.file_count.store(self.index.read().len() as u64, Ordering::SeqCst);

        // 保存到缓存
        self.save_cache();

        // 保存 USN 状态
        #[cfg(windows)]
        self.save_usn_state(volume_root);

        self.is_building.store(false, Ordering::SeqCst);

        log::info!("增量添加卷 {} 的 {} 个文件到索引", volume_root, entry_count);
    }

    /// 保存 USN 状态（读取 USN Journal 的 next_usn 并保存）
    #[cfg(windows)]
    fn save_usn_state(&self, volume_root: &str) {
        // 提取盘符
        let drive_char = volume_root.chars().next().unwrap_or('C');

        use usn_journal_rs::volume::Volume;
        use usn_journal_rs::journal::UsnJournal;

        // 尝试打开 USN Journal 并获取 next_usn
        match Volume::from_drive_letter(drive_char) {
            Ok(volume) => {
                let journal = UsnJournal::new(&volume);
                match journal.query(true) {
                    Ok(data) => {
                        let next_usn = data.next_usn;
                        log::info!("获取卷 {} 的 next_usn: {}", drive_char, next_usn);

                        // 使用 usn_monitor 保存状态
                        if let Err(e) = usn_monitor::set_last_usn(volume_root, next_usn) {
                            log::warn!("设置 USN 状态失败: {}", e);
                        }

                        // 保存到磁盘
                        if let Err(e) = usn_monitor::save_usn_state() {
                            log::warn!("保存 USN 状态失败: {}", e);
                        }
                    }
                    Err(e) => {
                        log::warn!("查询 USN Journal 失败: {:?}", e);
                    }
                }
            }
            Err(e) => {
                log::warn!("打开卷 {} 失败: {:?}", drive_char, e);
            }
        }
    }

    /// 搜索文件
    pub fn search(&self, query: &str, case_sensitive: bool, max_results: usize, search_paths: &[String]) -> Vec<SearchResultEntry> {
        let index = self.index.read();

        let results = index.search(query, case_sensitive, search_paths);

        // 格式化路径：去掉 \\.\ 前缀
        let format_path = |path: &str| -> String {
            if path.starts_with("\\\\.\\") {
                path[4..].to_string()
            } else {
                path.to_string()
            }
        };

        // 格式化时间戳为可读字符串
        let format_time = |timestamp: i64| -> String {
            if timestamp == 0 {
                return String::new();
            }

            // 将 Unix 时间戳转换为可读格式
            let secs = timestamp as i64;
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
        };

        results
            .into_iter()
            .take(max_results)
            .map(|e| SearchResultEntry {
                name: e.name.clone(),
                path: format_path(&e.path),
                size: e.size,
                modified_time: format_time(e.modified_time),
                is_directory: e.is_directory,
            })
            .collect()
    }

    /// 获取文件计数
    pub fn file_count(&self) -> u64 {
        self.file_count.load(Ordering::SeqCst)
    }

    /// 检查是否正在构建索引
    pub fn is_building(&self) -> bool {
        self.is_building.load(Ordering::SeqCst)
    }

    /// 检查索引是否有效
    pub fn is_valid(&self) -> bool {
        self.index.read().is_valid()
    }

    /// 检查缓存是否包含指定路径的索引
    pub fn has_path(&self, path: &str) -> bool {
        let volume_root = Self::get_volume_root(path);
        let index = self.index.read();
        index.volume_indices.contains_key(&volume_root)
    }

    /// 获取需要建立索引的卷列表（排除已索引的卷）
    pub fn get_volumes_to_index(&self, search_paths: &[String]) -> Vec<String> {
        let mut volumes_to_index = Vec::new();

        for search_path in search_paths {
            let volume_root = Self::get_volume_root(search_path);
            if volume_root.is_empty() {
                continue;
            }

            // 检查是否已经有这个卷的索引
            let index = self.index.read();
            if !index.volume_indices.contains_key(&volume_root) {
                // 避免重复添加
                if !volumes_to_index.contains(&volume_root) {
                    volumes_to_index.push(volume_root);
                }
            }
        }

        volumes_to_index
    }

    /// 获取当前已索引的卷列表
    pub fn get_indexed_volumes(&self) -> Vec<String> {
        let index = self.index.read();
        index.volume_indices.keys().cloned().collect()
    }

    /// 清除缓存
    pub fn clear(&self) {
        let mut index = self.index.write();
        *index = IndexCache::new();
        self.file_count.store(0, Ordering::SeqCst);

        // 删除缓存文件
        if self.cache_path.exists() {
            let _ = fs::remove_file(&self.cache_path);
        }
    }

    /// 获取路径的卷根路径
    fn get_volume_root(path: &str) -> String {
        let p = std::path::Path::new(path);
        if let Some(root) = p.components().next() {
            if let Some(drive) = root.as_os_str().to_str() {
                return format!("{}\\", drive);
            }
        }
        String::new()
    }

    /// 添加单个文件到索引（用于 USN 增量更新）
    pub fn add_file_entry(&self, name: String, path: String, size: u64, is_directory: bool, modified_time: i64) {
        let mut index = self.index.write();

        // 检查文件是否已存在
        if index.entries.iter().any(|e| e.path == path) {
            log::debug!("文件已存在，跳过: {}", path);
            return;
        }

        let idx = index.entries.len();
        let volume_root = Self::extract_volume_root(&path);

        // 添加到主列表
        index.entries.push(IndexEntry {
            name: name.clone(),
            path: path.clone(),
            size,
            modified_time,
            is_directory,
        });

        // 按卷分组
        if !volume_root.is_empty() {
            let volume = index.volume_indices
                .entry(volume_root.clone())
                .or_insert_with(|| VolumeIndex {
                    entries: Vec::new(),
                    root_path: volume_root,
                });
            volume.entries.push(idx);
        }

        // 构建名称前缀索引
        let name_lower = name.to_lowercase();
        if name_lower.len() >= 3 {
            let prefix: String = name_lower.chars().take(3).collect();
            if prefix.len() == 3 {
                index.name_index.entry(prefix).or_default().push(idx);
            }
        }

        index.last_update = std::time::Instant::now();

        // 释放写锁
        drop(index);

        self.file_count.store(self.index.read().len() as u64, Ordering::SeqCst);

        log::debug!("添加文件到索引: {}", path);
    }

    /// 从索引删除文件（用于 USN 增量更新）
    pub fn remove_file_entry(&self, path: &str) {
        let mut index = self.index.write();

        // 标准化路径：去掉 \\.\ 前缀
        let normalized_path = path.trim_start_matches("\\\\.\\");

        // 查找文件索引（尝试两种路径格式）
        let file_idx = index.entries.iter().position(|e| {
            e.path == normalized_path || e.path == path ||
            e.path.trim_start_matches("\\\\.\\") == normalized_path
        });

        if let Some(idx) = file_idx {
            // 获取文件名用于删除前缀索引
            let name = index.entries[idx].name.clone();
            let name_lower = name.to_lowercase();

            // 从主列表中移除
            index.entries.remove(idx);

            // 更新前缀索引：调整所有大于已删除索引的条目
            if name_lower.len() >= 3 {
                let prefix: String = name_lower.chars().take(3).collect();
                if let Some(indices) = index.name_index.get_mut(&prefix) {
                    indices.retain(|&i| i != idx);
                }
                // 调整其他前缀索引
                for indices in index.name_index.values_mut() {
                    for i in indices.iter_mut() {
                        if *i > idx {
                            *i -= 1;
                        }
                    }
                }
            }

            // 更新卷索引
            let volume_root = Self::extract_volume_root(path);
            if !volume_root.is_empty() {
                if let Some(volume) = index.volume_indices.get_mut(&volume_root) {
                    volume.entries.retain(|&i| i != idx);
                }
                // 调整其他卷索引
                for (vol, indices) in index.volume_indices.iter_mut() {
                    if *vol != volume_root {
                        for i in indices.entries.iter_mut() {
                            if *i > idx {
                                *i -= 1;
                            }
                        }
                    }
                }
            }

            index.last_update = std::time::Instant::now();

            // 释放写锁
            drop(index);

            self.file_count.store(self.index.read().len() as u64, Ordering::SeqCst);

            log::debug!("从索引删除文件: {}", path);
        }
    }

    /// 批量保存更改（在适当的时候调用）
    pub fn flush(&self) {
        self.save_cache();
    }
}

impl Default for CacheManager {
    fn default() -> Self {
        Self::new()
    }
}

/// 搜索结果条目（用于返回给前端）
#[derive(Debug, Clone, serde::Serialize)]
pub struct SearchResultEntry {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub modified_time: String,
    pub is_directory: bool,
}

lazy_static::lazy_static! {
    /// 全局缓存管理器实例
    pub static ref CACHE_MANAGER: CacheManager = CacheManager::new();
}

/// 初始化缓存（在应用启动时调用）
pub fn init_cache() {
    log::info!("初始化索引缓存...");

    // 尝试加载已有缓存
    if !CACHE_MANAGER.load_cache() {
        log::info!("没有找到有效缓存，需要在首次搜索时构建索引");
    }
}

/// 获取缓存管理器
pub fn get_cache_manager() -> &'static CacheManager {
    &CACHE_MANAGER
}
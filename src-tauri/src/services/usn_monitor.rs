//! USN Journal 监控模块
//! 使用 usn-journal-rs 监控文件变化，实现增量更新

use crate::services::index_cache::get_cache_manager;
use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use usn_journal_rs::journal::EnumOptions;
use usn_journal_rs::path::PathResolver;

/// USN 状态文件路径
fn get_usn_state_file_path() -> std::path::PathBuf {
    let app_data = dirs::data_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    let wsearch_dir = app_data.join("WSearch");

    // 确保目录存在
    if !wsearch_dir.exists() {
        let _ = fs::create_dir_all(&wsearch_dir);
    }

    wsearch_dir.join("usn_state.json")
}

/// USN Journal 状态
#[derive(Debug, Clone, serde::Serialize)]
pub struct UsnJournalStatus {
    pub is_running: bool,
    pub volume: String,
    pub error_message: Option<String>,
    pub last_update: String,
}

/// USN 记录（用于调试显示）
#[derive(Debug, Clone, serde::Serialize)]
pub struct UsnRecord {
    pub usn: i64,
    pub file_name: String,
    pub full_path: String,
    pub reason: u32,
    pub reason_text: String,
    pub timestamp: String,
}

/// USN Journal 监控器
pub struct UsnMonitor {
    volume: String,
    is_running: AtomicBool,
    last_update: std::time::Instant,
}

impl UsnMonitor {
    /// 创建新的监控器
    pub fn new(volume: &str) -> Self {
        Self {
            volume: volume.to_string(),
            is_running: AtomicBool::new(false),
            last_update: std::time::Instant::now(),
        }
    }

    /// 检查 USN Journal 是否可用
    #[cfg(windows)]
    pub fn check_journal_available(&self) -> Result<bool, String> {
        use usn_journal_rs::volume::Volume;

        log::info!("检查 USN Journal 可用性，卷: {}", self.volume);

        // 提取盘符
        let drive_letter = self.extract_drive_letter(&self.volume);

        if drive_letter.is_none() {
            let msg = "无法从路径提取盘符".to_string();
            log::error!("{}", msg);
            return Err(msg);
        }

        let drive = drive_letter.unwrap();
        log::info!("尝试打开盘符 {} 的 USN Journal", drive);

        // 尝试打开卷
        let volume = match Volume::from_drive_letter(drive) {
            Ok(v) => {
                log::info!("成功打开卷 {}:", drive);
                v
            }
            Err(e) => {
                let msg = format!("无法打开卷 {}: {:?}", drive, e);
                log::error!("{}", msg);
                return Err(msg);
            }
        };

        // 使用 new 打开 USN Journal
        let _journal = usn_journal_rs::journal::UsnJournal::new(&volume);
        log::info!("成功打开 USN Journal");
        Ok(true)
    }

    #[cfg(not(windows))]
    pub fn check_journal_available(&self) -> Result<bool, String> {
        Err("USN Journal 仅在 Windows 上可用".to_string())
    }

    /// 从路径提取盘符
    fn extract_drive_letter(&self, path: &str) -> Option<char> {
        let p = Path::new(path);

        // 处理 D:\ 格式
        if let Some(root) = p.components().next() {
            if let Some(drive) = root.as_os_str().to_str() {
                if drive.len() >= 2 && drive.chars().nth(1) == Some(':') {
                    return drive.chars().next();
                }
            }
        }

        // 直接处理字符串格式
        let path_str = path.trim_end_matches('\\').trim_end_matches('/');
        if path_str.len() >= 2 {
            let c = path_str.chars().nth(0)?;
            if c.is_ascii_alphabetic() {
                return Some(c);
            }
        }

        None
    }

    /// 获取监控状态
    pub fn get_status(&self) -> UsnJournalStatus {
        UsnJournalStatus {
            is_running: self.is_running.load(Ordering::SeqCst),
            volume: self.volume.clone(),
            error_message: None,
            last_update: format!("{:?}", self.last_update.elapsed()),
        }
    }
}

/// 增量更新器
pub struct IncrementalUpdater {
    monitor: Option<UsnMonitor>,
    enabled: Arc<AtomicBool>,
    volumes: Vec<String>,
    // 记录每个卷的上次 USN 位置
    last_usn: HashMap<String, i64>,
    // 无新记录时的检查间隔（默认 1 小时）
    idle_check_interval: Duration,
    // 标记是否有新记录
    has_new_records: bool,
}

impl IncrementalUpdater {
    /// 创建新的增量更新器
    pub fn new() -> Self {
        Self {
            monitor: None,
            enabled: Arc::new(AtomicBool::new(false)),
            volumes: Vec::new(),
            last_usn: HashMap::new(),
            idle_check_interval: Duration::from_secs(3600), // 默认 1 小时
            has_new_records: false,
        }
    }

    /// 初始化（指定要监控的卷）
    pub fn init(&mut self, volume: &str) -> Result<UsnJournalStatus, String> {
        log::info!("初始化增量更新器，卷: {}", volume);

        let monitor = UsnMonitor::new(volume);

        // 检查 USN Journal 是否可用
        match monitor.check_journal_available() {
            Ok(_) => {
                log::info!("USN Journal 可用，启用增量更新");
                self.monitor = Some(monitor);
                self.enabled.store(true, Ordering::SeqCst);
                self.volumes.push(volume.to_string());

                let mut status = self.monitor.as_ref().unwrap().get_status();
                status.error_message = None;
                Ok(status)
            }
            Err(e) => {
                log::error!("USN Journal 不可用: {}", e);
                self.enabled.store(false, Ordering::SeqCst);
                Err(e)
            }
        }
    }

    /// 获取状态
    pub fn get_status(&self) -> Option<UsnJournalStatus> {
        self.monitor.as_ref().map(|m| m.get_status())
    }

    /// 检查是否启用
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    /// 获取启用标志的 Arc 引用
    fn get_enabled(&self) -> Arc<AtomicBool> {
        self.enabled.clone()
    }

    /// 获取监控的卷列表
    fn get_volumes(&self) -> Vec<String> {
        self.volumes.clone()
    }

    /// 手动设置指定卷的 USN 位置
    /// 用于跳过历史记录或重新扫描
    pub fn set_last_usn(&mut self, volume: &str, usn: i64) {
        log::info!("手动设置卷 {} 的 USN 位置为: {}", volume, usn);
        self.last_usn.insert(volume.to_string(), usn);
    }

    /// 获取指定卷的当前 USN 位置
    pub fn get_last_usn(&self, volume: &str) -> Option<i64> {
        self.last_usn.get(volume).copied()
    }

    /// 执行一次增量更新，返回是否有新记录
    #[cfg(windows)]
    pub fn update_once(&mut self) -> Result<bool, String> {
        use windows::Win32::System::Ioctl::{
            USN_REASON_FILE_CREATE, USN_REASON_FILE_DELETE, USN_REASON_RENAME_NEW_NAME, USN_REASON_RENAME_OLD_NAME,
        };

        if !self.enabled.load(Ordering::SeqCst) {
            return Err("增量更新未启用".to_string());
        }

        let monitor = self.monitor.as_ref()
            .ok_or_else(|| "监控器未初始化".to_string())?;

        use usn_journal_rs::volume::Volume;
        use usn_journal_rs::journal::UsnJournal;

        let drive_letter = monitor.extract_drive_letter(&monitor.volume)
            .ok_or_else(|| "无法提取盘符".to_string())?;

        log::debug!("执行增量更新检查，卷: {}", drive_letter);

        // 打开卷
        let volume = Volume::from_drive_letter(drive_letter)
            .map_err(|e| format!("无法打开卷: {:?}", e))?;

        // 打开 USN Journal
        let journal = UsnJournal::new(&volume);

        let cache_manager = get_cache_manager();

        // reason 常量就是 u32 值
        let reason_file_create: u32 = USN_REASON_FILE_CREATE;
        let reason_file_delete: u32 = USN_REASON_FILE_DELETE;
        let reason_rename_new: u32 = USN_REASON_RENAME_NEW_NAME;
        let reason_rename_old: u32 = USN_REASON_RENAME_OLD_NAME;

        // 获取上次读取的 USN 位置
        let last_usn = self.last_usn.get(&monitor.volume).copied().unwrap_or(0i64);

        log::info!("开始读取 USN，日志: {}, 上次位置: {}", drive_letter, last_usn);

        // 使用 iter_with_options 从上次位置开始读取（真正的增量更新）
        let options = EnumOptions {
            start_usn: last_usn,
            reason_mask: reason_file_create | reason_file_delete | reason_rename_new | reason_rename_old,
            only_on_close: false,
            timeout: 0,
            wait_for_more: false,
            buffer_size: 64 * 1024, // 64KB 缓冲区
        };

        let iter_result = journal.iter_with_options(options);

        // 记录本次读取的最大 USN
        let mut max_usn: i64 = last_usn;

        // 每次最多处理 100 条记录，避免长时间阻塞
        let max_records_per_batch = 300;

        // 统计本次处理的条目数
        let mut processed_count = 0;

        match iter_result {
            Ok(iter) => {
                for result in iter {
                    // 每次迭代前检查是否已停止
                    if !self.enabled.load(Ordering::SeqCst) {
                        log::info!("检测到停止信号，退出迭代");
                        break;
                    }

                    let entry_usn: i64;

                    match result {
                        Ok(entry) => {
                            entry_usn = entry.usn;

                            // reason 是 u32
                            let reason_mask = entry.reason;
                            let is_relevant = (reason_mask & (reason_file_create | reason_file_delete | reason_rename_new | reason_rename_old)) != 0;

                            if is_relevant {
                                // 检查是否已达到最大处理数量（只统计实际处理的记录）
                                if processed_count >= max_records_per_batch {
                                    log::info!("已达到最大处理数量 {}，退出迭代", max_records_per_batch);
                                    break;
                                }

                                // 将 OsString 转换为 String
                                let file_name_str = entry.file_name.to_string_lossy().to_string();

                                // 过滤临时文件（不计入处理数量）
                                if is_temp_file(&file_name_str) {
                                    log::debug!("跳过临时文件: {}", file_name_str);
                                    // 仍然更新 max_usn
                                    if entry_usn > max_usn {
                                        max_usn = entry_usn;
                                    }
                                    continue;
                                }

                                // 使用 PathResolver 解析完整路径
                                let mut resolver = PathResolver::new_with_cache(&volume);
                                let full_path = resolver.resolve_path(&entry);

                                let path_str = match &full_path {
                                    Some(p) => p.to_string_lossy().to_string(),
                                    None => format!("{}:\\{}", drive_letter, file_name_str),
                                };

                                // 转换时间戳为可读格式
                                let change_time = chrono::DateTime::<chrono::Local>::from(entry.time)
                                    .format("%Y-%m-%d %H:%M:%S");

                                log::debug!("USN 变化: USN={}, Time={}, Reason={}, FullPath={}",
                                    entry.usn, change_time, reason_mask, path_str);

                                // 根据原因更新缓存
                                if (reason_mask & reason_file_create) != 0 || (reason_mask & reason_rename_new) != 0 {
                                    // 获取文件的实际属性
                                    let (file_size, is_dir, modified_time) = get_file_attributes(&path_str);

                                    cache_manager.add_file_entry(
                                        file_name_str.clone(),
                                        path_str.clone(),
                                        file_size,
                                        is_dir,
                                        modified_time,
                                    );
                                    log::debug!("添加文件到索引: {}, size: {}", file_name_str, file_size);
                                }

                                if (reason_mask & reason_file_delete) != 0 || (reason_mask & reason_rename_old) != 0 {
                                    cache_manager.remove_file_entry(&path_str);
                                    log::debug!("从索引删除文件: {}", file_name_str);
                                }

                                // 只有实际处理了记录才增加计数
                                processed_count += 1;
                            }
                        }
                        Err(e) => {
                            log::warn!("读取 USN 条目失败: {:?}", e);
                            // 仍然更新 max_usn
                            entry_usn = 0;
                        }
                    }

                    // 更新最大 USN
                    if entry_usn > max_usn {
                        max_usn = entry_usn;
                    }
                }
            }
            Err(e) => {
                log::error!("读取 USN Journal 失败: {:?}", e);
                return Err(format!("读取失败: {:?}", e));
            }
        }

        // 更新上次读取的 USN 位置并返回是否有新记录
        if max_usn > last_usn {
            self.last_usn.insert(monitor.volume.clone(), max_usn);
            self.has_new_records = true;
            log::debug!("更新 USN 位置: {} -> {}，处理了 {} 条新记录", monitor.volume, max_usn, processed_count);
        } else {
            self.has_new_records = false;
            log::debug!("无新 USN 记录");
        }

        log::debug!("增量更新检查完成");
        Ok(self.has_new_records)
    }

    #[cfg(not(windows))]
    pub fn update_once(&self) -> Result<bool, String> {
        Err("USN Journal 仅在 Windows 上可用".to_string())
    }
}

impl Default for IncrementalUpdater {
    fn default() -> Self {
        Self::new()
    }
}

// 全局增量更新器
lazy_static::lazy_static! {
    static ref INCREMENTAL_UPDATER: std::sync::Mutex<IncrementalUpdater> =
        std::sync::Mutex::new(IncrementalUpdater::new());
}

/// 后台监控线程是否正在运行
static mut BACKGROUND_MONITOR_RUNNING: bool = false;

/// 启动增量更新服务
pub fn start_incremental_service(volumes: Vec<String>) -> Result<Vec<UsnJournalStatus>, String> {
    log::info!("启动增量更新服务，卷: {:?}", volumes);

    // 先加载之前保存的 USN 状态
    init_usn_state();

    let mut results = Vec::new();

    for volume in volumes {
        let mut updater = INCREMENTAL_UPDATER.lock().map_err(|e| e.to_string())?;
        match updater.init(&volume) {
            Ok(status) => {
                results.push(status);
            }
            Err(e) => {
                log::error!("初始化卷 {} 的增量更新失败: {}", volume, e);
                results.push(UsnJournalStatus {
                    is_running: false,
                    volume,
                    error_message: Some(e),
                    last_update: String::new(),
                });
            }
        }
    }

    // 启动后台监控线程
    start_background_monitor();

    Ok(results)
}

/// 启动后台监控线程
fn start_background_monitor() {
    // 获取 updater 的引用
    let updater = match INCREMENTAL_UPDATER.lock() {
        Ok(u) => u,
        Err(e) => {
            log::error!("获取增量更新器失败: {}", e);
            return;
        }
    };

    let enabled = updater.get_enabled();
    let volumes = updater.get_volumes();

    // 释放锁
    drop(updater);

    // 如果已经有后台线程在运行，不再启动
    unsafe {
        if BACKGROUND_MONITOR_RUNNING {
            log::debug!("后台监控线程已在运行");
            return;
        }
        BACKGROUND_MONITOR_RUNNING = true;
    }

    log::info!("启动后台 USN 监控线程");

    thread::spawn(move || {
        // 高频检查间隔（5毫秒）
        let active_check_interval = Duration::from_millis(5);
        // 空闲检查间隔（1小时）
        let idle_check_interval = Duration::from_secs(3600);
        // 每 5 分钟保存一次
        let save_interval = Duration::from_secs(300);
        let mut last_save = std::time::Instant::now();

        // 当前是否处于空闲状态
        let mut is_idle = false;

        loop {
            if !enabled.load(Ordering::SeqCst) {
                log::info!("增量更新已禁用，停止后台监控");
                // 退出前保存状态
                let _ = save_usn_state();
                break;
            }

            let mut has_new_records = false;

            // 重新获取 updater 并调用 update_once
            if let Ok(mut updater) = INCREMENTAL_UPDATER.lock() {
                if updater.is_enabled() {
                    // 遍历所有监控的卷
                    for volume in &volumes {
                        log::debug!("检查卷 {} 的 USN 变化", volume);

                        match updater.update_once() {
                            Ok(has_new) => {
                                // 增量更新成功
                                if has_new {
                                    log::debug!("卷 {} 有新 USN 记录", volume);
                                    has_new_records = true;
                                }
                            }
                            Err(e) => {
                                log::warn!("卷 {} 增量更新失败: {}", volume, e);
                            }
                        }
                    }
                }
            }

            // 定期保存 USN 状态（每 5 分钟）
            if last_save.elapsed() >= save_interval {
                if let Err(e) = save_usn_state() {
                    log::warn!("定期保存 USN 状态失败: {}", e);
                } else {
                    last_save = std::time::Instant::now();
                }
            }

            // 根据是否有新记录调整检查间隔
            if has_new_records {
                is_idle = false;
                thread::sleep(active_check_interval);
            } else {
                if !is_idle {
                    log::info!("无新 USN 记录，进入空闲模式，1 小时后再次检查");
                    is_idle = true;
                }
                thread::sleep(idle_check_interval);
            }
        }

        unsafe {
            BACKGROUND_MONITOR_RUNNING = false;
        }
        log::info!("后台 USN 监控线程已停止");
    });
}

/// 停止增量更新服务
pub fn stop_incremental_service() {
    log::info!("停止增量更新服务");

    if let Ok(updater) = INCREMENTAL_UPDATER.lock() {
        updater.enabled.store(false, Ordering::SeqCst);
    }

    // 注意：USN 状态会在后台线程检测到停止信号后自动保存
    // 这里不需要重复保存，避免日志中显示两次保存
}

/// 获取增量更新器
pub fn get_incremental_updater() -> &'static std::sync::Mutex<IncrementalUpdater> {
    &INCREMENTAL_UPDATER
}

/// 手动触发一次增量更新（供外部调用）
/// 返回是否有新记录
pub fn trigger_incremental_update() -> bool {
    log::info!("手动触发增量更新");

    let mut has_new_records = false;

    if let Ok(mut updater) = INCREMENTAL_UPDATER.lock() {
        if updater.is_enabled() {
            for volume in updater.get_volumes() {
                log::info!("手动更新卷: {}", volume);
                match updater.update_once() {
                    Ok(has_new) => {
                        if has_new {
                            log::info!("卷 {} 有新记录", volume);
                            has_new_records = true;
                            //to do 需要把增量服务调整为5秒检查检查一次
                        }
                    }
                    Err(e) => {
                        log::error!("手动更新失败: {}", e);
                    }
                }
            }
        } else {
            log::warn!("增量更新未启用");
        }
    }

    // 增量更新后立即保存 USN 状态，确保即使应用异常退出也能从最新位置恢复
    // if let Err(e) = save_usn_state() {
    //     log::warn!("保存 USN 状态失败: {}", e);
    // }

    has_new_records
}

/// 手动设置指定卷的 USN 位置（用于跳过历史记录或重新扫描）
/// volume: 卷名，如 "D:\"
/// usn: 新的 USN 位置，设为 0 可重新扫描全部历史
pub fn set_last_usn(volume: &str, usn: i64) -> Result<(), String> {
    log::info!("手动设置卷 {} 的 USN 位置为: {}", volume, usn);

    let mut updater = INCREMENTAL_UPDATER.lock().map_err(|e| e.to_string())?;
    updater.set_last_usn(volume, usn);
    Ok(())
}

/// 获取指定卷的当前 USN 位置
pub fn get_last_usn(volume: &str) -> Option<i64> {
    if let Ok(updater) = INCREMENTAL_UPDATER.lock() {
        return updater.get_last_usn(volume);
    }
    None
}

/// 检查是否是临时文件
fn is_temp_file(file_name: &str) -> bool {
    let name = file_name.to_lowercase();

    // 1. 过滤以 ~ 开头的文件（Windows 临时文件）
    if name.starts_with('~') {
        return true;
    }

    // 2. 过滤包含 ~RF 的文件（Windows 临时文件标记）
    if name.contains("~rf") {
        return true;
    }

    // 3. 过滤扩展名为 .tmp 的文件
    if name.ends_with(".tmp") {
        return true;
    }

    // 4. 过滤扩展名为 .temp 的文件
    if name.ends_with(".temp") {
        return true;
    }

    // 5. 过滤 ~ 开头的临时文件（另一种格式）
    // if name.starts_with("~$") {
    //     return true;
    // }

    false
}

/// 获取文件的实际属性（大小、是否目录和修改时间）
/// 返回 (file_size, is_directory, modified_time_unix)
fn get_file_attributes(path: &str) -> (u64, bool, i64) {
    // 处理路径格式：去掉 \\.\ 前缀
    let normalized_path = path.trim_start_matches("\\\\.\\");

    let file_path = Path::new(normalized_path);

    match fs::metadata(file_path) {
        Ok(metadata) => {
            let size = metadata.len();
            let is_dir = metadata.is_dir();

            // 获取修改时间
            let modified_time = metadata
                .modified()
                .map(|t| {
                    t.duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs() as i64)
                        .unwrap_or(0)
                })
                .unwrap_or(0);

            (size, is_dir, modified_time)
        }
        Err(e) => {
            log::warn!("获取文件属性失败: {}, 错误: {}", path, e);
            (0, false, 0)
        }
    }
}

// ==================== 持久化保存功能 ====================

/// 保存 USN 状态到磁盘
pub fn save_usn_state() -> Result<(), String> {
    log::info!("开始保存 USN 状态");

    let updater = INCREMENTAL_UPDATER.lock().map_err(|e| e.to_string())?;
    let last_usn = updater.last_usn.clone();
    log::info!("当前 last_usn 数据: {:?}", last_usn);
    drop(updater);

    if last_usn.is_empty() {
        log::warn!("last_usn 为空，跳过保存");
        return Ok(());
    }

    save_usn_state_to_disk(&last_usn)
}

/// 保存 USN 状态到磁盘文件
fn save_usn_state_to_disk(last_usn: &HashMap<String, i64>) -> Result<(), String> {
    let file_path = get_usn_state_file_path();

    let json = serde_json::to_string_pretty(last_usn)
        .map_err(|e| format!("序列化失败: {}", e))?;

    let mut file = fs::File::create(&file_path)
        .map_err(|e| format!("创建文件失败: {}", e))?;

    file.write_all(json.as_bytes())
        .map_err(|e| format!("写入文件失败: {}", e))?;

    log::info!("USN 状态已保存到: {:?}", file_path);
    Ok(())
}

/// 从磁盘加载 USN 状态
pub fn load_usn_state() -> Result<HashMap<String, i64>, String> {
    let file_path = get_usn_state_file_path();

    if !file_path.exists() {
        log::info!("USN 状态文件不存在，返回空状态");
        return Ok(HashMap::new());
    }

    let content = fs::read_to_string(&file_path)
        .map_err(|e| format!("读取文件失败: {}", e))?;

    let last_usn: HashMap<String, i64> = serde_json::from_str(&content)
        .map_err(|e| format!("解析 JSON 失败: {}", e))?;

    log::info!("从 {:?} 加载 USN 状态: {:?}", file_path, last_usn);
    Ok(last_usn)
}

/// 初始化时加载 USN 状态
pub fn init_usn_state() {
    match load_usn_state() {
        Ok(last_usn) => {
            if let Ok(mut updater) = INCREMENTAL_UPDATER.lock() {
                for (volume, usn) in last_usn {
                    updater.last_usn.insert(volume, usn);
                }
                log::info!("USN 状态加载成功");
            }
        }
        Err(e) => {
            log::warn!("加载 USN 状态失败: {}", e);
        }
    }
}

// ==================== 调试功能：获取最近 USN 记录 ====================

/// 将 reason 掩码转换为可读文本
fn reason_to_string(reason: u32) -> String {
    let mut reasons = Vec::new();

    // USN_REASON_* 常量定义
    const USN_REASON_DATA_OVERWRITE: u32 = 0x00000001;
    const USN_REASON_DATA_EXTEND: u32 = 0x00000002;
    const USN_REASON_DATA_TRUNCATION: u32 = 0x00000004;
    const USN_REASON_NAMED_DATA_OVERWRITE: u32 = 0x00000010;
    const USN_REASON_NAMED_DATA_EXTEND: u32 = 0x00000020;
    const USN_REASON_NAMED_DATA_TRUNCATION: u32 = 0x00000040;
    const USN_REASON_FILE_CREATE: u32 = 0x00000100;
    const USN_REASON_FILE_DELETE: u32 = 0x00000200;
    const USN_REASON_EA_CHANGE: u32 = 0x00000400;
    const USN_REASON_SECURITY_CHANGE: u32 = 0x00000800;
    const USN_REASON_RENAME_OLD_NAME: u32 = 0x00001000;
    const USN_REASON_RENAME_NEW_NAME: u32 = 0x00002000;
    const USN_REASON_INDEXABLE_CHANGE: u32 = 0x00004000;
    const USN_REASON_REPARSE_POINT_CHANGE: u32 = 0x00008000;
    const USN_REASON_STREAM_CHANGE: u32 = 0x00010000;
    const USN_REASON_LINK_CHANGE: u32 = 0x00020000;
    const USN_REASON_VALID_BASE_CHANGE: u32 = 0x00040000;
    const USN_REASON_HARD_LINK_CHANGE: u32 = 0x00080000;
    const USN_REASON_EXTERNAL_FLAG_CHANGE: u32 = 0x00100000;
    const USN_REASON_ATTRIBUTE_CHANGE: u32 = 0x00200000;
    const USN_REASON_INTEGRITY_CHANGE: u32 = 0x00400000;
    const USN_REASON_ENCRYPTION_CHANGE: u32 = 0x00800000;
    const USN_REASON_OBJECT_ID_CHANGE: u32 = 0x01000000;
    const USN_REASON_REPARSE_TAG_CHANGE: u32 = 0x02000000;
    const USN_REASON_STREAM_ATTRIBUTE_CHANGE: u32 = 0x04000000;
    const USN_REASON_ONLY_ACCESS_CHECK: u32 = 0x08000000;
    const USN_REASON_USN_SOURCE_CHANGE: u32 = 0x10000000;
    const USN_REASON_USN_TITLE_CHANGE: u32 = 0x20000000;
    const USN_REASON_MOUNTED_ON_GLOBAL_REPARSE_POINT: u32 = 0x40000000;

    if reason & USN_REASON_DATA_OVERWRITE != 0 { reasons.push("DATA_OVERWRITE"); }
    if reason & USN_REASON_DATA_EXTEND != 0 { reasons.push("DATA_EXTEND"); }
    if reason & USN_REASON_DATA_TRUNCATION != 0 { reasons.push("DATA_TRUNCATION"); }
    if reason & USN_REASON_NAMED_DATA_OVERWRITE != 0 { reasons.push("NAMED_DATA_OVERWRITE"); }
    if reason & USN_REASON_NAMED_DATA_EXTEND != 0 { reasons.push("NAMED_DATA_EXTEND"); }
    if reason & USN_REASON_NAMED_DATA_TRUNCATION != 0 { reasons.push("NAMED_DATA_TRUNCATION"); }
    if reason & USN_REASON_FILE_CREATE != 0 { reasons.push("FILE_CREATE"); }
    if reason & USN_REASON_FILE_DELETE != 0 { reasons.push("FILE_DELETE"); }
    if reason & USN_REASON_EA_CHANGE != 0 { reasons.push("EA_CHANGE"); }
    if reason & USN_REASON_SECURITY_CHANGE != 0 { reasons.push("SECURITY_CHANGE"); }
    if reason & USN_REASON_RENAME_OLD_NAME != 0 { reasons.push("RENAME_OLD_NAME"); }
    if reason & USN_REASON_RENAME_NEW_NAME != 0 { reasons.push("RENAME_NEW_NAME"); }
    if reason & USN_REASON_INDEXABLE_CHANGE != 0 { reasons.push("INDEXABLE_CHANGE"); }
    if reason & USN_REASON_REPARSE_POINT_CHANGE != 0 { reasons.push("REPARSE_POINT_CHANGE"); }
    if reason & USN_REASON_STREAM_CHANGE != 0 { reasons.push("STREAM_CHANGE"); }
    if reason & USN_REASON_LINK_CHANGE != 0 { reasons.push("LINK_CHANGE"); }
    if reason & USN_REASON_VALID_BASE_CHANGE != 0 { reasons.push("VALID_BASE_CHANGE"); }
    if reason & USN_REASON_HARD_LINK_CHANGE != 0 { reasons.push("HARD_LINK_CHANGE"); }
    if reason & USN_REASON_EXTERNAL_FLAG_CHANGE != 0 { reasons.push("EXTERNAL_FLAG_CHANGE"); }
    if reason & USN_REASON_ATTRIBUTE_CHANGE != 0 { reasons.push("ATTRIBUTE_CHANGE"); }
    if reason & USN_REASON_INTEGRITY_CHANGE != 0 { reasons.push("INTEGRITY_CHANGE"); }
    if reason & USN_REASON_ENCRYPTION_CHANGE != 0 { reasons.push("ENCRYPTION_CHANGE"); }
    if reason & USN_REASON_OBJECT_ID_CHANGE != 0 { reasons.push("OBJECT_ID_CHANGE"); }
    if reason & USN_REASON_REPARSE_TAG_CHANGE != 0 { reasons.push("REPARSE_TAG_CHANGE"); }
    if reason & USN_REASON_STREAM_ATTRIBUTE_CHANGE != 0 { reasons.push("STREAM_ATTRIBUTE_CHANGE"); }
    if reason & USN_REASON_ONLY_ACCESS_CHECK != 0 { reasons.push("ONLY_ACCESS_CHECK"); }
    if reason & USN_REASON_USN_SOURCE_CHANGE != 0 { reasons.push("USN_SOURCE_CHANGE"); }
    if reason & USN_REASON_USN_TITLE_CHANGE != 0 { reasons.push("USN_TITLE_CHANGE"); }
    if reason & USN_REASON_MOUNTED_ON_GLOBAL_REPARSE_POINT != 0 { reasons.push("MOUNTED_ON_GLOBAL_REPARSE_POINT"); }

    if reasons.is_empty() {
        format!("0x{:08X}", reason)
    } else {
        reasons.join(" | ")
    }
}

/// 获取最近 USN 记录（用于调试）
/// volume: 盘符，如 "D:" 或 "D:\"
/// count: 返回的记录数量，默认 10
#[cfg(windows)]
pub fn get_recent_usn_records(volume: &str, count: usize) -> Result<Vec<UsnRecord>, String> {
    use usn_journal_rs::volume::Volume;
    use usn_journal_rs::journal::UsnJournal;
    use usn_journal_rs::USN_REASON_MASK_ALL;

    log::info!("获取最近 USN 记录，卷: {}, 数量: {}", volume, count);

    // 提取盘符
    let drive_letter = {
        let p = Path::new(volume);
        if let Some(root) = p.components().next() {
            if let Some(drive) = root.as_os_str().to_str() {
                if drive.len() >= 2 && drive.chars().nth(1) == Some(':') {
                    drive.chars().next()
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        }
    };

    let drive_letter = drive_letter.ok_or_else(|| "无效的盘符".to_string())?;

    // 打开卷
    let vol = Volume::from_drive_letter(drive_letter)
        .map_err(|e| format!("无法打开卷: {:?}", e))?;

    // 打开 USN Journal
    let journal = UsnJournal::new(&vol);

    // 查询 Journal 状态，获取下一个要读取的 USN 位置
    let journal_data = journal.query(true)
        .map_err(|e| format!("查询 USN Journal 失败: {:?}", e))?;
    let next_usn = journal_data.next_usn;

    log::info!("Next USN: {}, Max USN: {}", next_usn, journal_data.max_usn);

    // 从 max_usn - 5000 - count 位置开始读取，确保获取到最近的记录
    // 使用 saturating_sub 和 max(0) 防止 start_usn 为负数或超出 Journal 范围
    let start_usn = next_usn;

    let options = EnumOptions {
        start_usn,
        reason_mask: USN_REASON_MASK_ALL,
        only_on_close: false,
        timeout: 0,
        wait_for_more: false,
        buffer_size: 64 * 1024,
    };

    log::info!("开始读取 USN，记录起始位置: {}", start_usn);

    let mut records = Vec::new();
    let mut resolver = PathResolver::new_with_cache(&vol);

    // 使用 iter_with_options 读取
    match journal.iter_with_options(options) {
        Ok(iter) => {
            for result in iter {
            //     match result {
            //         Ok(entry) => {
            //             // let file_name_str = entry.file_name.to_string_lossy().to_string();

            //             // 解析完整路径
            //             // let full_path = resolver.resolve_path(&entry);
            //             // let path_str = match &full_path {
            //             //     Some(p) => p.to_string_lossy().to_string(),
            //             //     None => format!("{}:\\{}", drive_letter, file_name_str),
            //             // };

            //             // 转换时间戳
            //             // let timestamp = chrono::DateTime::<chrono::Local>::from(entry.time)
            //             //     .format("%Y-%m-%d %H:%M:%S%.3f")
            //             //     .to_string();

            //             // records.push(UsnRecord {
            //             //     usn: entry.usn,
            //             //     file_name: file_name_str,
            //             //     full_path: path_str,
            //             //     reason: entry.reason,
            //             //     reason_text: reason_to_string(entry.reason),
            //             //     timestamp,
            //             // });
            //         }
            //         Err(e) => {
            //             log::warn!("读取 USN 条目失败: {:?}", e);
            //         }
            //     }
            }
        }
        Err(e) => {
            log::error!("读取 USN Journal 失败: {:?}", e);
            return Err(format!("读取失败: {:?}", e));
        }
    }

    // 保留最后 count 条记录
    if records.len() > count {
        let split_index = records.len() - count;
        records = records.split_off(split_index);
    }

    log::info!("获取到 {} 条 USN 记录", records.len());
    Ok(records)
}

#[cfg(not(windows))]
pub fn get_recent_usn_records(_volume: &str, _count: usize) -> Result<Vec<UsnRecord>, String> {
    Err("USN Journal 仅在 Windows 上可用".to_string())
}

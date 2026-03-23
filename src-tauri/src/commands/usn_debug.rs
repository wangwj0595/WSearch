//! USN 调试命令
//! 用于获取最近的 USN 记录进行调试

use crate::services::usn_monitor::{get_recent_usn_records, UsnRecord};

/// 获取最近 USN 记录
/// volume: 盘符，如 "D:" 或 "D:\"
/// count: 返回的记录数量，默认 10，最大 100
#[tauri::command]
pub fn get_recent_usn(volume: String, count: Option<u32>) -> Result<Vec<UsnRecord>, String> {
    let count = count.unwrap_or(10).min(100) as usize;

    log::info!("[get_recent_usn] volume: {}, count: {}", volume, count);

    get_recent_usn_records(&volume, count)
}
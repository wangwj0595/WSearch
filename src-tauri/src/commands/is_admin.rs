/// 检查当前是否以管理员权限运行
#[tauri::command]
pub fn is_admin() -> bool {
    crate::services::mft_reader::is_running_as_admin()
}
//! NTFS MFT 读取模块
//! 使用 ntfs-reader crate 直接解析 MFT 表

/// 检查是否有管理员权限
#[cfg(windows)]
pub fn is_running_as_admin() -> bool {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    unsafe {
        let mut token_handle = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token_handle).is_err() {
            return false;
        }

        let mut elevation = TOKEN_ELEVATION::default();
        let mut return_length = 0u32;
        let result = GetTokenInformation(
            token_handle,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut _),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut return_length,
        );

        result.is_ok() && elevation.TokenIsElevated != 0
    }
}

#[cfg(not(windows))]
fn is_running_as_admin() -> bool {
    false
}

/// 将普通路径转换为 Windows 设备路径
/// D:\ -> \\.\D:
/// D:\folder -> \\.\D:
fn to_device_path(path: &str) -> String {
    // 提取盘符（例如 D:\ 或 D: -> D）
    let path = path.trim_end_matches('\\');
    if path.len() >= 2 && path.chars().nth(1) == Some(':') {
        let drive_letter = path.chars().next().unwrap();
        return format!("\\\\.\\{}:", drive_letter);
    }
    path.to_string()
}

/// MFT 文件条目
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MftFileEntry {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub modified_time: i64,
    pub is_directory: bool,
}

/// 扫描指定卷上的所有文件
/// 使用 ntfs-reader 直接读取 MFT 表，极快
#[cfg(windows)]
pub fn scan_volume_files(volume_path: &str) -> Vec<MftFileEntry> {
    use ntfs_reader::file_info::FileInfo;
    use ntfs_reader::mft::Mft;
    use ntfs_reader::volume::Volume;

    // 检查管理员权限
    if !is_running_as_admin() {
        log::warn!("需要管理员权限才能读取 MFT，请以管理员身份运行程序");
        return Vec::new();
    }

    let mut entries = Vec::new();

    // 转换路径格式：D:\ -> \\.\D:
    let device_path = to_device_path(volume_path);
    log::info!("尝试打开设备路径: {}", device_path);

    // 打开卷设备（Windows 下需要管理员权限）
    let volume = match Volume::new(&device_path) {
        Ok(v) => v,
        Err(e) => {
            log::warn!("无法打开卷 {} (设备路径: {}): {:?}", volume_path, device_path, e);
            return entries;
        }
    };

    // 创建 MFT 解析器
    let mft = match Mft::new(volume) {
        Ok(m) => m,
        Err(e) => {
            log::warn!("无法解析 MFT: {:?}", e);
            return entries;
        }
    };

    log::info!("正在读取 MFT，卷: {}", volume_path);

    // 遍历所有 MFT 记录
    let mut count = 0;

    mft.iterate_files(|file| {
        // 获取文件信息
        let info = FileInfo::new(&mft, file);

        // 跳过系统文件
        if info.name.starts_with('$') {
            return;
        }

        count += 1;

        // 获取修改时间（直接从 MFT 获取，无需文件系统调用）
        // ntfs-reader 库从 MFT 解析的时间戳字段是 Option<OffsetDateTime>
        let modified_time = info.modified
            .map(|t| t.unix_timestamp())
            .unwrap_or(0);

        entries.push(MftFileEntry {
            name: info.name.clone(),
            path: info.path.to_string_lossy().to_string(),
            size: info.size,
            modified_time,
            is_directory: info.is_directory,
        });
    });

    log::info!("从 MFT 扫描到 {} 个文件", count);
    entries
}

#[cfg(not(windows))]
pub fn scan_volume_files(_volume_path: &str) -> Vec<MftFileEntry> {
    Vec::new()
}

/// 检查路径是否为 NTFS 卷
#[cfg(windows)]
pub fn is_ntfs_volume(path: &str) -> bool {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::GetVolumeInformationW;

    log::info!("检测文件系统类型: {}", path);

    let wide_path: Vec<u16> = OsStr::new(path)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let mut volume_name = [0u16; 261];
    let mut serial_number: u32 = 0;
    let mut max_component_length: u32 = 0;
    let mut file_system_flags: u32 = 0;
    let mut file_system_name = [0u16; 261];

    let result = unsafe {
        GetVolumeInformationW(
            PCWSTR::from_raw(wide_path.as_ptr()),
            Some(&mut volume_name),
            Some(&mut serial_number),
            Some(&mut max_component_length),
            Some(&mut file_system_flags),
            Some(&mut file_system_name),
        )
    };

    if result.is_ok() {
        let fs_name = String::from_utf16_lossy(&file_system_name);
        // 去除末尾的空字符 \0
        let fs_name_trimmed = fs_name.trim_end_matches('\0').trim();
        return fs_name_trimmed == "NTFS";
    }

    log::warn!("GetVolumeInformationW 失败 for {}", path);
    false
}

#[cfg(not(windows))]
pub fn is_ntfs_volume(_path: &str) -> bool {
    false
}

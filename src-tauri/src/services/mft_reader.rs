//! NTFS MFT 读取模块
//! 使用 ntfs-reader crate 直接解析 MFT 表

/// MFT 文件条目
#[derive(Debug, Clone)]
pub struct MftFileEntry {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub modified_time: String,
    pub is_directory: bool,
}

/// 扫描指定卷上的所有文件
/// 使用 ntfs-reader 直接读取 MFT 表，极快
#[cfg(windows)]
pub fn scan_volume_files(volume_path: &str) -> Vec<MftFileEntry> {
    use ntfs_reader::file_info::FileInfo;
    use ntfs_reader::mft::Mft;
    use ntfs_reader::volume::Volume;

    let mut entries = Vec::new();

    // 打开卷设备（Windows 下需要管理员权限）
    // 格式: "\\\\.\\C:" 或 "C:\\"
    let volume = match Volume::new(volume_path) {
        Ok(v) => v,
        Err(e) => {
            log::warn!("无法打开卷 {}: {:?}", volume_path, e);
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
    mft.iterate_files(|file| {
        // 获取文件信息
        let info = FileInfo::new(&mft, file);

        // 跳过系统文件
        if info.name.starts_with('$') {
            return;
        }

        entries.push(MftFileEntry {
            name: info.name,
            path: info.path.to_string_lossy().to_string(),
            size: info.size,
            modified_time: String::new(),
            is_directory: info.is_directory,
        });
    });

    log::info!("从 MFT 扫描到 {} 个文件", entries.len());
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
        return fs_name == "NTFS";
    }

    false
}

#[cfg(not(windows))]
pub fn is_ntfs_volume(_path: &str) -> bool {
    false
}

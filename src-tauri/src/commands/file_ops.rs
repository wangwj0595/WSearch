use std::process::Command;

/// 打开文件或文件夹
#[tauri::command]
pub fn open_file(path: String) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        Command::new("cmd")
            .args(["/C", "start", "", &path])
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(&path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "linux")]
    {
        Command::new("xdg-open")
            .arg(&path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

/// 在资源管理器中显示文件位置
#[tauri::command]
pub fn reveal_in_explorer(path: String) -> Result<(), String> {
    let path = std::path::Path::new(&path);
    let _dir = if path.is_file() {
        path.parent().unwrap_or(path)
    } else {
        path
    };

    #[cfg(target_os = "windows")]
    {
        Command::new("explorer")
            .args(["/select,", &path.to_string_lossy()])
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .args(["-R", &path.to_string_lossy()])
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "linux")]
    {
        Command::new("xdg-open")
            .arg(_dir)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

/// 复制文件路径到剪贴板
#[tauri::command]
pub fn copy_path(path: String) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        Command::new("cmd")
            .args(["/C", "echo", &path, "|", "clip"])
            .output()
            .map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("pbcopy")
            .arg(&path)
            .output()
            .map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "linux")]
    {
        Command::new("xclip")
            .args(["-selection", "clipboard"])
            .arg(&path)
            .output()
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

/// 删除单个文件或目录
#[tauri::command]
pub fn delete_file(path: String) -> Result<String, String> {
    let path_obj = std::path::Path::new(&path);

    if !path_obj.exists() {
        return Err("文件或目录不存在".to_string());
    }

    let result = if path_obj.is_dir() {
        std::fs::remove_dir_all(&path)
    } else {
        std::fs::remove_file(&path)
    };

    match result {
        Ok(_) => {
            log::info!("删除成功: {}", path);
            Ok(format!("已删除: {}", path))
        }
        Err(e) => {
            log::error!("删除失败: {} - {}", path, e);
            Err(format!("删除失败: {}", e))
        }
    }
}

/// 批量删除文件或目录
#[tauri::command]
pub fn delete_files(paths: Vec<String>) -> Result<Vec<String>, String> {
    let mut success_count = 0;
    let mut failed_paths: Vec<String> = Vec::new();

    for path in &paths {
        let path_obj = std::path::Path::new(path);

        if !path_obj.exists() {
            failed_paths.push(format!("{} - 文件不存在", path));
            continue;
        }

        let result = if path_obj.is_dir() {
            std::fs::remove_dir_all(path)
        } else {
            std::fs::remove_file(path)
        };

        match result {
            Ok(_) => {
                success_count += 1;
                log::info!("删除成功: {}", path);
            }
            Err(e) => {
                failed_paths.push(format!("{} - {}", path, e));
                log::error!("删除失败: {} - {}", path, e);
            }
        }
    }

    if failed_paths.is_empty() {
        Ok(vec![format!("成功删除 {} 个项目", success_count)])
    } else if success_count > 0 {
        let mut messages = vec![format!("成功删除 {} 个项目", success_count)];
        messages.extend(failed_paths);
        Ok(messages)
    } else {
        Err("所有删除操作均失败".to_string())
    }
}

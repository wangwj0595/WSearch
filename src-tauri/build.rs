use std::fs;
use std::path::Path;

fn main() {
    // 读取 package.json 获取版本号
    let package_json_path = Path::new("../package.json");
    let package_json_content = fs::read_to_string(package_json_path)
        .expect("Failed to read package.json");

    let package_json: serde_json::Value = serde_json::from_str(&package_json_content)
        .expect("Failed to parse package.json");

    let version = package_json["version"]
        .as_str()
        .expect("Failed to get version from package.json");

    // 设置 Cargo 目标文件的版本
    println!("cargo:rustc-env=APP_VERSION={}", version);

    // 同时设置 CARGO_PKG_VERSION 环境变量（这是 cargo 默认的）
    println!("cargo:version={}", version);

    tauri_build::build()
}
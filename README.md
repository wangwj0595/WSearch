# WSearch 🔍

> 一个基于 Tauri + React + TypeScript 的桌面搜索应用

## ✨ 特性

- ⚡ 基于 Tauri 构建，性能优异
- 💻 现代化 UI 设计
- 🔧 完整的 TypeScript 类型支持

## 🚀 快速开始

### 环境要求

- Node.js 18+
- Rust 1.70+
- pnpm 8+

### 安装步骤

```bash
# 1. 安装 pnpm（如未安装）
npm install -g pnpm

# 2. 安装项目依赖
pnpm install


```

### Skills

pnpx skills add ant-design/antd-skill
pnpx skills add obra/superpowers

### 运行项目

```bash
# 启动开发服务器
pnpm tauri dev
```

### 构建发布

```bash
# 构建生产版本
pnpm tauri build
```

## ⚠️ 注意事项

当前环境没有安装 Rust/Cargo，无法直接运行 `pnpm tauri dev`。您需要：

### 安装 Visual Studio Build Tools（推荐）

1. 下载 [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/)
2. 运行安装程序，选择"使用 C++ 的桌面开发"
3. 勾选以下组件：
   - MSVC v143 - VS 2022 C++ x64/x86 构建工具
   - Windows 11 SDK（如果需要）
4. 安装完成后重启终端

### 安装 Rust

1. 访问 [Rust 官网](https://rustup.rs/) 下载安装
2. 按照提示完成安装
3. 运行 `pnpm tauri dev` 启动应用

## 🛠️ 推荐 IDE 配置

| 工具 | 说明 |
|------|------|
| [VS Code](https://code.visualstudio.com/) | 代码编辑器 |
| [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) | Tauri 扩展 |
| [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer) | Rust 语言支持 |

## 📁 项目结构

```
WSearch/
├── src/                    # React 前端源码
│   ├── components/         # 组件目录
│   ├── hooks/              # 自定义 Hooks
│   ├── types/              # TypeScript 类型定义
│   └── App.tsx             # 主应用组件
├── src-tauri/              # Tauri 后端源码
│   ├── src/
│   │   ├── commands/       # Tauri 命令
│   │   ├── services/       # 服务层
│   │   └── lib.rs          # 库入口
│   └── tauri.conf.json     # Tauri 配置
└── package.json            # 项目配置
```

## 📄 许可证

MIT License

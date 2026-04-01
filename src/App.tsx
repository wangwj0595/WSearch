import { useState, useEffect, useCallback, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { ConfigProvider, message, Table, Button, Input, InputNumber, Space, Checkbox, List, Typography, Tooltip, Progress, theme, Collapse, Dropdown, Select, Modal, Popconfirm } from "antd";
import { FolderOpenOutlined, FolderOutlined, FileTextOutlined, HistoryOutlined, SettingOutlined, DeleteOutlined, MoreOutlined, SyncOutlined, BugOutlined, SaveOutlined, EditOutlined } from "@ant-design/icons";
import type { SearchResult, SearchConfig, SearchHistory, SearchProgress, SearchCompletedEvent, UsnRecord, SearchPreset } from "./types";
import type { RowSelectMethod } from "antd/es/table/interface";
import { defaultSearchConfig } from "./types";
import { useWindowSize } from "./hooks/useWindowSize";
import "./App.css";

const { Text } = Typography;

function AppContent() {
  // 启用窗口大小记忆
  useWindowSize();

  const [results, setResults] = useState<SearchResult[]>([]);
  const [loading, setLoading] = useState(false);
  const [config, setConfig] = useState<SearchConfig>(defaultSearchConfig);
  const [history, setHistory] = useState<SearchHistory[]>([]);
  const [isResizing, setIsResizing] = useState(false);
  const [sidebarWidth, setSidebarWidth] = useState(config.sidebar_width);
  const [pageSize, setPageSize] = useState(25);
  const [currentPage, setCurrentPage] = useState(1);
  const containerRef = useRef<HTMLDivElement>(null);
  const [tableHeight, setTableHeight] = useState(400);
  const [totalResults, setTotalResults] = useState(0);
  const [searchProgress, setSearchProgress] = useState<SearchProgress | null>(null);
  const [lastSearchTime, setLastSearchTime] = useState<number | null>(null);
  const [searchKeyword, setSearchKeyword] = useState<string>("");
  const [selectedRowKeys, setSelectedRowKeys] = useState<React.Key[]>([]);
  const [lastSelectedRow, setLastSelectedRow] = useState<number | null>(null);
  const [activePanels, setActivePanels] = useState<string[]>(['search', 'exclude', 'options', 'history']);

  // USN 调试相关状态
  const [usnVolume, setUsnVolume] = useState<string>("D:");
  const [usnCount, setUsnCount] = useState<number>(10);
  const [usnRecords, setUsnRecords] = useState<UsnRecord[]>([]);
  const [usnLoading, setUsnLoading] = useState(false);
  const [showDebugPanel, setShowDebugPanel] = useState(false);

  // 索引刷新相关状态
  const [refreshLoading, setRefreshLoading] = useState(false);

  // 文件大小筛选状态
  const [minSizeValue, setMinSizeValue] = useState<number | null>(null);
  const [minSizeUnit, setMinSizeUnit] = useState<'KB' | 'MB' | 'GB'>('KB');
  const [maxSizeValue, setMaxSizeValue] = useState<number | null>(null);
  const [maxSizeUnit, setMaxSizeUnit] = useState<'KB' | 'MB' | 'GB'>('KB');

  // 预设相关状态
  const [presetModalVisible, setPresetModalVisible] = useState(false);
  const [savePresetModalVisible, setSavePresetModalVisible] = useState(false);
  const [presetName, setPresetName] = useState('');
  const [editingPreset, setEditingPreset] = useState<SearchPreset | null>(null);

  // 文件名编辑相关状态
  const [editingFilePath, setEditingFilePath] = useState<string | null>(null);
  const [editingFileName, setEditingFileName] = useState<string>("");

  // 标记是否正在处理折叠面板变化（防止 useEffect 覆盖用户操作）
  // 使用 ref 来避免闭包问题
  const isUpdatingPanelsRef = useRef(false);

  // 从配置加载折叠面板状态
  useEffect(() => {
    // 如果正在处理用户操作，不覆盖
    if (isUpdatingPanelsRef.current) {
      isUpdatingPanelsRef.current = false;
      return;
    }

    if (config.collapsed_panels && config.collapsed_panels.length > 0) {
      // 收起的面板不显示在 activePanels 中
      const allPanels = ['search', 'exclude', 'options', 'history'];
      setActivePanels(allPanels.filter(p => !config.collapsed_panels.includes(p)));
    }
  }, [config.collapsed_panels]);

  // 保存折叠面板状态
  const saveCollapsedPanels = useCallback((expandedPanels: string[]) => {
    // 标记正在处理用户操作，防止 useEffect 覆盖
    isUpdatingPanelsRef.current = true;

    // 立即更新 UI 状态，不等待后端响应
    setActivePanels(expandedPanels);

    const allPanels = ['search', 'exclude', 'options', 'history'];
    const collapsedPanels = allPanels.filter(p => !expandedPanels.includes(p));
    // 直接使用传入的 expandedPanels 计算，避免依赖可能过期的 config 状态
    setConfig(prevConfig => {
      const newConfig = { ...prevConfig, collapsed_panels: collapsedPanels };
      // 在状态更新后异步保存
      invoke("save_search_config", { config: newConfig });
      return newConfig;
    });
  }, []);

  // 加载配置和历史
  useEffect(() => {
    loadConfig();
    loadHistory();

    // 监听搜索结果批次事件（流式显示）
    const unlistenBatch = listen<SearchResult[]>("search_result_batch", (event) => {
      setResults(prev => [...prev, ...event.payload]);
    });

    // 监听搜索完成事件
    const unlistenComplete = listen<SearchCompletedEvent>("search_completed", (event) => {
      setTotalResults(event.payload.result_count);
      // 保存搜索时间
      setLastSearchTime(event.payload.elapsed_time);
      setLoading(false);
      loadHistory();
    });

    // 监听搜索开始事件
    const unlistenStart = listen("search_started", () => {
      setResults([]);
      setTotalResults(0);
      setSearchProgress(null);
      setLastSearchTime(null);
      setSelectedRowKeys([]);
      setLastSelectedRow(null);
      setCurrentPage(1);
    });

    // 监听搜索进度事件
    const unlistenProgress = listen<SearchProgress>("search_progress", (event) => {
      setSearchProgress(event.payload);
    });

    return () => {
      unlistenBatch.then(fn => fn());
      unlistenComplete.then(fn => fn());
      unlistenStart.then(fn => fn());
      unlistenProgress.then(fn => fn());
    };
  }, []);

  // 同步配置中的侧边栏宽度
  useEffect(() => {
    setSidebarWidth(config.sidebar_width);
  }, [config.sidebar_width]);

  // 计算表格高度
  useEffect(() => {
    const updateHeight = () => {
      if (containerRef.current) {
        const containerHeight = containerRef.current.clientHeight;
        // 搜索栏高度 72px + 结果头部 40px + padding 32px = 144px
        const height = containerHeight - 144;
        setTableHeight(Math.max(200, height));
      }
    };

    updateHeight();
    window.addEventListener('resize', updateHeight);
    return () => window.removeEventListener('resize', updateHeight);
  }, [sidebarWidth]);

  // 处理调整大小
  const handleMouseDown = (e: React.MouseEvent) => {
    e.preventDefault();
    setIsResizing(true);
  };

  useEffect(() => {
    if (!isResizing) return;

    const handleMouseMove = (e: MouseEvent) => {
      const newWidth = e.clientX;
      // 限制最小和最大宽度
      const minWidth = 200;
      const maxWidth = 600;
      const clampedWidth = Math.max(minWidth, Math.min(maxWidth, newWidth));
      setSidebarWidth(clampedWidth);
    };

    const handleMouseUp = () => {
      setIsResizing(false);
      // 保存新的宽度到配置
      const newConfig = { ...config, sidebar_width: sidebarWidth };
      setConfig(newConfig);
      invoke("save_search_config", { config: newConfig });
    };

    document.addEventListener("mousemove", handleMouseMove);
    document.addEventListener("mouseup", handleMouseUp);

    return () => {
      document.removeEventListener("mousemove", handleMouseMove);
      document.removeEventListener("mouseup", handleMouseUp);
    };
  }, [isResizing, sidebarWidth, config]);

  const loadConfig = async () => {
    try {
      const savedConfig = await invoke<SearchConfig>("get_search_config");
      if (savedConfig) {
        setConfig(savedConfig);
        // 同步折叠面板状态
        const allPanels = ['search', 'exclude', 'options', 'history'];
        if (savedConfig.collapsed_panels && savedConfig.collapsed_panels.length > 0) {
          setActivePanels(allPanels.filter(p => !savedConfig.collapsed_panels!.includes(p)));
        } else {
          setActivePanels(allPanels);
        }
      }
    } catch (e) {
      console.error("加载配置失败:", e);
    }
  };

  const loadHistory = async () => {
    try {
      const hist = await invoke<SearchHistory[]>("get_search_history");
      setHistory(hist);
    } catch (e) {
      console.error("加载历史失败:", e);
    }
  };

  // 添加搜索目录
  const addSearchPath = async () => {
    const selected = await open({
      directory: true,
      multiple: false,
      title: "选择搜索目录",
    });
    if (selected) {
      const newPaths = [...config.search_paths, selected as string];
      const newConfig = { ...config, search_paths: newPaths };
      setConfig(newConfig);
      await invoke("save_search_config", { config: newConfig });
    }
  };

  // 移除搜索目录
  const removeSearchPath = (path: string) => {
    const newPaths = config.search_paths.filter((p) => p !== path);
    const newConfig = { ...config, search_paths: newPaths };
    setConfig(newConfig);
    invoke("save_search_config", { config: newConfig });
  };

  // 添加排除目录
  const addExcludePath = async () => {
    const selected = await open({
      directory: true,
      multiple: false,
      title: "选择排除目录",
    });
    if (selected) {
      const newPaths = [...config.exclude_paths, selected as string];
      const newConfig = { ...config, exclude_paths: newPaths };
      setConfig(newConfig);
      await invoke("save_search_config", { config: newConfig });
    }
  };

  // 移除排除目录
  const removeExcludePath = (path: string) => {
    const newPaths = config.exclude_paths.filter((p) => p !== path);
    const newConfig = { ...config, exclude_paths: newPaths };
    setConfig(newConfig);
    invoke("save_search_config", { config: newConfig });
  };

  // 执行搜索（流式版本）
  const handleSearch = useCallback(async (searchQuery: string) => {
    if (!searchQuery.trim()) {
      message.warning("请输入搜索关键词");
      return;
    }
    if (config.search_paths.length === 0) {
      message.warning("请至少添加一个搜索目录");
      return;
    }

    setLoading(true);
    setResults([]);
    setTotalResults(0);

    try {
      // 流式搜索：不等待返回结果，结果通过事件分批推送
      await invoke("search_files", {
        query: searchQuery,
        searchPaths: config.search_paths,
        excludePaths: config.exclude_paths,
        fileTypes: config.file_types,
        searchContent: config.search_content,
        caseSensitive: config.case_sensitive,
        searchDirectories: config.search_directories,
        useMft: config.use_mft,
        maxResults: config.max_results,
        minSize: convertToBytes(minSizeValue, minSizeUnit),
        maxSize: convertToBytes(maxSizeValue, maxSizeUnit),
      });
      // 搜索历史会在 search_completed 事件中刷新
    } catch (e) {
      message.error(`搜索失败: ${e}`);
      setLoading(false);
    }
  }, [config, minSizeValue, minSizeUnit, maxSizeValue, maxSizeUnit]);

  // 打开文件
  const handleOpenFile = async (path: string) => {
    try {
      await invoke("open_file", { path });
    } catch (e) {
      message.error(`打开失败: ${e}`);
    }
  };

  // 在资源管理器中显示
  const handleReveal = async (path: string) => {
    try {
      await invoke("reveal_in_explorer", { path });
    } catch (e) {
      message.error(`显示失败: ${e}`);
    }
  };

  // 重命名文件
  const handleRenameFile = async (oldPath: string, newName: string) => {
    // 获取原文件名
    const pathParts = oldPath.replace(/\\/g, '/').split('/');
    const oldName = pathParts[pathParts.length - 1];

    // 如果文件名没有变化，不调用后端
    if (newName.trim() === oldName) {
      setEditingFilePath(null);
      setEditingFileName("");
      return;
    }

    try {
      const result = await invoke<string>("rename_file", {
        oldPath,
        newName: newName.trim()
      });
      message.success(result);

      // 更新本地状态
      setResults(prev => prev.map(item => {
        if (item.path === oldPath) {
          // 计算新路径
          const oldPathObj = oldPath.replace(/\\/g, '/');
          const lastSlash = oldPathObj.lastIndexOf('/');
          const newPath = oldPath.substring(0, lastSlash + 1) + newName.trim();
          return {
            ...item,
            name: newName.trim(),
            path: newPath
          };
        }
        return item;
      }));
    } catch (e) {
      message.error(`重命名失败: ${e}`);
    } finally {
      setEditingFilePath(null);
      setEditingFileName("");
    }
  };

  // 开始编辑文件名
  const startEditFileName = (path: string, name: string) => {
    setEditingFilePath(path);
    setEditingFileName(name);
  };

  // 取消编辑文件名
  // const cancelEditFileName = () => {
  //   setEditingFilePath(null);
  //   setEditingFileName("");
  // };

  // 删除单个文件
  const handleDeleteFile = async (path: string) => {
    try {
      const result = await invoke<string>("delete_file", { path });
      message.success(result);
      // 从结果中移除已删除的文件
      setResults(prev => prev.filter(item => item.path !== path));
      setTotalResults(prev => Math.max(0, prev - 1));
      // 清除选中状态
      setSelectedRowKeys(prev => prev.filter(key => key !== path));
    } catch (e) {
      message.error(`删除失败: ${e}`);
    }
  };

  // 批量删除文件
  const handleBatchDelete = async () => {
    if (selectedRowKeys.length === 0) {
      message.warning("请先选择要删除的文件");
      return;
    }

    try {
      const paths = selectedRowKeys.map(key => key as string);
      const result = await invoke<string[]>("delete_files", { paths });

      if (result.length === 1) {
        message.success(result[0]);
      } else {
        // 显示成功和失败信息
        result.forEach(msg => {
          if (msg.includes("成功")) {
            message.success(msg);
          } else {
            message.warning(msg);
          }
        });
      }

      // 从结果中移除已删除的文件
      setResults(prev => prev.filter(item => !selectedRowKeys.includes(item.path)));
      setTotalResults(prev => Math.max(0, prev - selectedRowKeys.length));
      // 清除选中状态
      setSelectedRowKeys([]);
    } catch (e) {
      message.error(`批量删除失败: ${e}`);
    }
  };

  // 清除历史
  const handleClearHistory = async () => {
    try {
      await invoke("clear_search_history");
      setHistory([]);
      message.success("历史已清除");
    } catch (e) {
      message.error(`清除失败: ${e}`);
    }
  };

  // 转换大小值为字节
  const convertToBytes = (value: number | null, unit: 'KB' | 'MB' | 'GB'): number => {
    if (value === null || value === undefined || value === 0) return 0;
    switch (unit) {
      case 'KB': return value * 1024;
      case 'MB': return value * 1024 * 1024;
      case 'GB': return value * 1024 * 1024 * 1024;
      default: return value;
    }
  };

  // 快速选择大小
  const handleQuickSize = (size: number, unit: 'KB' | 'MB' | 'GB') => {
    setMinSizeValue(size);
    setMinSizeUnit(unit);
  };

  // 格式化显示范围
  const formatSizeRange = (): string => {
    const minBytes = convertToBytes(minSizeValue, minSizeUnit);
    const maxBytes = convertToBytes(maxSizeValue, maxSizeUnit);

    const formatBytes = (bytes: number): string => {
      if (bytes === 0) return '0';
      if (bytes >= 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
      if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
      if (bytes >= 1024) return `${(bytes / 1024).toFixed(1)} KB`;
      return `${bytes} B`;
    };

    const minStr = minBytes > 0 ? formatBytes(minBytes) : '0';
    const maxStr = maxBytes > 0 ? formatBytes(maxBytes) : '无限制';

    return `${minStr} ~ ${maxStr}`;
  };

  // 取消搜索
  const handleCancelSearch = async () => {
    try {
      await invoke("cancel_search");
      setLoading(false);
      message.info("搜索已取消");
    } catch (e) {
      message.error(`取消失败: ${e}`);
    }
  };

  // 分页变化处理
  const handlePaginationChange = (page: number, size: number) => {
    setCurrentPage(page);
    setPageSize(size);
  };

  // 获取最近 USN 记录（调试用）
  const handleGetRecentUsn = async () => {
    if (!usnVolume.trim()) {
      message.warning("请输入盘符");
      return;
    }

    setUsnLoading(true);
    try {
      const records = await invoke<UsnRecord[]>("get_recent_usn", {
        volume: usnVolume,
        count: usnCount,
      });
      setUsnRecords(records);
      message.success(`获取到 ${records.length} 条 USN 记录`);
    } catch (e) {
      message.error(`获取失败: ${e}`);
    } finally {
      setUsnLoading(false);
    }
  };

  // 更新索引
  const handleRefreshIndex = async () => {
    if (config.search_paths.length === 0) {
      message.warning("请先添加搜索目录");
      return;
    }

    setRefreshLoading(true);
    try {
      // 从搜索路径中提取卷
      const volumes = config.search_paths
        .map(p => {
          const path = p.replace(/\\/g, '/');
          const match = path.match(/^([A-Za-z]):/);
          return match ? `${match[1]}:\\` : null;
        })
        .filter((v): v is string => v !== null);

      if (volumes.length === 0) {
        message.warning("无法识别卷");
        return;
      }

      // 去重
      const uniqueVolumes = [...new Set(volumes)];

      // 传递卷列表到后端，后端循环处理
      const result = await invoke<string>("refresh_index", { volumes: uniqueVolumes });
      message.success(result);
    } catch (e) {
      message.error(`更新索引失败: ${e}`);
    } finally {
      setRefreshLoading(false);
    }
  };

  // 生成预设签名（用于去重）
  const getPresetSignature = (paths: string[]): string => {
    return paths.join('|');
  };

  // 检查是否存在相同目录配置的预设
  const isDuplicatePreset = (paths: string[], excludeId?: string): boolean => {
    const signature = getPresetSignature(paths);
    return config.presets.some(p =>
      p.id !== excludeId && getPresetSignature(p.search_paths) === signature
    );
  };

  // 保存为新预设
  const handleSavePreset = async () => {
    if (!presetName.trim()) {
      message.warning("请输入预设名称");
      return;
    }

    if (config.search_paths.length === 0) {
      message.warning("请先添加搜索目录");
      return;
    }

    // 检查同名预设
    if (config.presets.some(p => p.name === presetName.trim())) {
      message.warning("已存在同名预设，请使用其他名称");
      return;
    }

    // 检查目录组合是否重复
    if (isDuplicatePreset(config.search_paths)) {
      message.warning("已存在相同目录配置的预设");
      return;
    }

    const newPreset: SearchPreset = {
      id: crypto.randomUUID(),
      name: presetName.trim(),
      search_paths: [...config.search_paths],
      created_at: Date.now(),
      use_count: 0,
    };

    const newPresets = [...config.presets, newPreset];
    const newConfig = {
      ...config,
      presets: newPresets,
      active_preset_id: newPreset.id,
    };

    setConfig(newConfig);
    await invoke("save_search_config", { config: newConfig });
    message.success(`预设 "${presetName}" 保存成功`);
    setSavePresetModalVisible(false);
    setPresetName('');
  };

  // 应用预设
  const handleApplyPreset = async (preset: SearchPreset) => {
    // 更新使用次数
    const updatedPresets = config.presets.map(p =>
      p.id === preset.id ? { ...p, use_count: p.use_count + 1 } : p
    );

    const newConfig = {
      ...config,
      search_paths: [...preset.search_paths],
      presets: updatedPresets,
      active_preset_id: preset.id,
    };

    setConfig(newConfig);
    await invoke("save_search_config", { config: newConfig });
    message.success(`已切换到预设 "${preset.name}"`);
  };

  // 删除预设
  const handleDeletePreset = async (presetId: string) => {
    const newPresets = config.presets.filter(p => p.id !== presetId);
    const newActiveId = config.active_preset_id === presetId ? null : config.active_preset_id;

    const newConfig = {
      ...config,
      presets: newPresets,
      active_preset_id: newActiveId,
    };

    setConfig(newConfig);
    await invoke("save_search_config", { config: newConfig });
    message.success("预设已删除");
  };

  // 重命名预设
  const handleRenamePreset = async (presetId: string, newName: string) => {
    if (!newName.trim()) {
      message.warning("预设名称不能为空");
      return;
    }

    // 检查同名预设
    if (config.presets.some(p => p.id !== presetId && p.name === newName.trim())) {
      message.warning("已存在同名预设");
      return;
    }

    const newPresets = config.presets.map(p =>
      p.id === presetId ? { ...p, name: newName.trim() } : p
    );

    const newConfig = {
      ...config,
      presets: newPresets,
    };

    setConfig(newConfig);
    await invoke("save_search_config", { config: newConfig });
    message.success("预设已重命名");
    setEditingPreset(null);
  };

  // 创建空白预设
  const handleCreateEmptyPreset = async () => {
    const newPreset: SearchPreset = {
      id: crypto.randomUUID(),
      name: `预设 ${config.presets.length + 1}`,
      search_paths: [],
      created_at: Date.now(),
      use_count: 0,
    };

    const newPresets = [...config.presets, newPreset];
    const newConfig = {
      ...config,
      presets: newPresets,
      active_preset_id: newPreset.id,
    };

    setConfig(newConfig);
    await invoke("save_search_config", { config: newConfig });
    setPresetModalVisible(false);
    message.success("空白预设已创建");
  };

  // 获取当前激活的预设
  // const activePreset = config.presets.find(p => p.id === config.active_preset_id);

  // 表格列定义
  const columns = [
    {
      title: "名称",
      dataIndex: "name",
      key: "name",
      sorter: (a: SearchResult, b: SearchResult) => a.name.localeCompare(b.name),
      render: (name: string, record: SearchResult) => (
        <Space>
          <span>{record.is_directory ? "📁" : "📄"}</span>
          {editingFilePath === record.path ? (
            <Input
              size="small"
              value={editingFileName}
              onChange={(e) => setEditingFileName(e.target.value)}
              onPressEnter={() => handleRenameFile(record.path, editingFileName)}
              onBlur={() => handleRenameFile(record.path, editingFileName)}
              autoFocus
              style={{ width: 150 }}
              onClick={(e) => e.stopPropagation()}
            />
          ) : (
            <a
              onClick={(e) => {
                e.stopPropagation();
                startEditFileName(record.path, name);
              }}
              title="点击修改文件名"
              style={{ cursor: 'pointer' }}
            >
              {name}
            </a>
          )}
        </Space>
      ),
    },
    {
      title: "路径",
      dataIndex: "path",
      key: "path",
      ellipsis: true,
      sorter: (a: SearchResult, b: SearchResult) => a.path.localeCompare(b.path),
      render: (path: string) => (
        <Tooltip title={path}>
          <Text type="secondary" style={{ fontSize: 12 }}>{path}</Text>
        </Tooltip>
      ),
    },
    {
      title: "大小",
      dataIndex: "size",
      key: "size",
      width: 100,
      sorter: (a: SearchResult, b: SearchResult) => a.size - b.size,
      render: (size: number) => size > 0 ? `${(size / 1024).toFixed(1)} KB` : "-",
    },
    {
      title: "修改时间",
      dataIndex: "modified_time",
      key: "modified_time",
      width: 180,
      sorter: (a: SearchResult, b: SearchResult) => a.modified_time.localeCompare(b.modified_time),
      render: (time: string) => <Text type="secondary" style={{ fontSize: 12 }}>{time}</Text>,
    },
    {
      title: "操作",
      key: "action",
      width: 180,
      render: (_: unknown, record: SearchResult) => (
        <Space size="small">
          <Button size="small" onClick={() => handleOpenFile(record.path)}>打开</Button>
          <Button size="small" onClick={() => handleReveal(record.path)}>定位</Button>
          <Button
            size="small"
            danger
            onClick={() => handleDeleteFile(record.path)}
          >
            删除
          </Button>
        </Space>
      ),
    },
  ];

  return (
    <div className="app-container">
      <div className="sidebar" style={{ width: `${sidebarWidth}px`, minWidth: `${sidebarWidth}px` }}>
        <Collapse
          activeKey={activePanels}
          ghost
          expandIconPosition="end"
          onChange={(keys) => {
            saveCollapsedPanels(keys as string[]);
          }}
        >
          <Collapse.Panel
            key="search"
            header={
              <div className="collapse-header">
                <div className="collapse-header-left">
                  <FolderOpenOutlined style={{ color: '#1890ff', fontSize: 14 }} />
                  <Text strong>搜索目录</Text>
                  <Text type="secondary" style={{ fontSize: 12 }}>({config.search_paths.length})</Text>
                </div>
                <Button
                  type="text"
                  size="small"
                  icon={<FolderOutlined />}
                  onClick={(e) => {
                    e.stopPropagation();
                    addSearchPath();
                  }}
                />
              </div>
            }
          >
            {/* 预设选择和操作按钮 */}
            <div style={{ marginBottom: 8, display: 'flex', gap: 8, alignItems: 'center', flexWrap: 'wrap' }}>
              <Select
                size="small"
                style={{ width: 140 }}
                placeholder="选择预设"
                value={config.active_preset_id}
                onChange={(value) => {
                  const preset = config.presets.find(p => p.id === value);
                  if (preset) handleApplyPreset(preset);
                }}
                options={config.presets.map(p => ({
                  value: p.id,
                  label: `${p.name} (${p.use_count}次)`,
                }))}
                allowClear
                onClear={() => {
                  const newConfig = { ...config, active_preset_id: null };
                  setConfig(newConfig);
                  invoke("save_search_config", { config: newConfig });
                }}
              />
              <Button
                size="small"
                icon={<SaveOutlined />}
                onClick={() => setSavePresetModalVisible(true)}
                disabled={config.search_paths.length === 0}
              >
                保存预设
              </Button>
              <Button
                size="small"
                icon={<SettingOutlined />}
                onClick={() => setPresetModalVisible(true)}
              >
                管理
              </Button>
            </div>

            {config.search_paths.length === 0 ? (
              <Text type="secondary" style={{ fontSize: 12, display: 'block', textAlign: 'center', padding: '12px 0' }}>
                点击上方 + 添加搜索目录
              </Text>
            ) : (
              <List
                size="small"
                dataSource={config.search_paths}
                renderItem={(item) => (
                  <List.Item
                    className="path-item"
                    actions={[
                      <Button
                        key="remove"
                        type="text"
                        danger
                        size="small"
                        onClick={() => removeSearchPath(item)}
                      >
                        <DeleteOutlined />
                      </Button>
                    ]}
                  >
                    <Text ellipsis style={{ fontSize: 12 }}>{item}</Text>
                  </List.Item>
                )}
              />
            )}
          </Collapse.Panel>

          <Collapse.Panel
            key="exclude"
            header={
              <div className="collapse-header">
                <div className="collapse-header-left">
                  <FolderOutlined style={{ color: '#ff4d4f', fontSize: 14 }} />
                  <Text strong>排除目录</Text>
                  <Text type="secondary" style={{ fontSize: 12 }}>({config.exclude_paths.length})</Text>
                </div>
                <Button
                  type="text"
                  size="small"
                  icon={<FolderOutlined />}
                  onClick={(e) => {
                    e.stopPropagation();
                    addExcludePath();
                  }}
                />
              </div>
            }
          >
            {config.exclude_paths.length === 0 ? (
              <Text type="secondary" style={{ fontSize: 12, display: 'block', textAlign: 'center', padding: '12px 0' }}>
                点击上方 + 添加排除目录
              </Text>
            ) : (
              <List
                size="small"
                dataSource={config.exclude_paths}
                renderItem={(item) => (
                  <List.Item
                    className="path-item"
                    actions={[
                      <Button
                        key="remove"
                        type="text"
                        danger
                        size="small"
                        onClick={() => removeExcludePath(item)}
                      >
                        <DeleteOutlined />
                      </Button>
                    ]}
                  >
                    <Text ellipsis style={{ fontSize: 12 }}>{item}</Text>
                  </List.Item>
                )}
              />
            )}
          </Collapse.Panel>

          <Collapse.Panel
            key="options"
            header={
              <div className="collapse-header">
                <div className="collapse-header-left">
                  <SettingOutlined style={{ color: '#52c41a', fontSize: 14 }} />
                  <Text strong>搜索选项</Text>
                </div>
              </div>
            }
          >
            <Space direction="vertical" style={{ width: "100%" }} size="middle">
              <Checkbox
                checked={config.search_content}
                onChange={(e) => {
                  const newConfig = { ...config, search_content: e.target.checked };
                  setConfig(newConfig);
                  invoke("save_search_config", { config: newConfig });
                }}
              >
                <FileTextOutlined style={{ marginRight: 6 }} />
                搜索文件内容
              </Checkbox>
              <Checkbox
                checked={config.search_directories}
                onChange={(e) => {
                  const newConfig = { ...config, search_directories: e.target.checked };
                  setConfig(newConfig);
                  invoke("save_search_config", { config: newConfig });
                }}
              >
                <FolderOpenOutlined style={{ marginRight: 6 }} />
                搜索目录
              </Checkbox>
              <Checkbox
                checked={config.case_sensitive}
                onChange={(e) => {
                  const newConfig = { ...config, case_sensitive: e.target.checked };
                  setConfig(newConfig);
                  invoke("save_search_config", { config: newConfig });
                }}
              >
                区分大小写
              </Checkbox>
              <Checkbox
                checked={config.use_mft}
                onChange={(e) => {
                  const newConfig = { ...config, use_mft: e.target.checked };
                  setConfig(newConfig);
                  invoke("save_search_config", { config: newConfig });
                }}
              >
                MFT 快速搜索
              </Checkbox>
              <div className="max-results-wrapper">
                <Text style={{ fontSize: 13 }}>最大结果数:</Text>
                <InputNumber
                  size="small"
                  min={1}
                  max={10000}
                  value={config.max_results}
                  onChange={(value) => {
                    if (value) {
                      const newConfig = { ...config, max_results: value };
                      setConfig(newConfig);
                      invoke("save_search_config", { config: newConfig });
                    }
                  }}
                  style={{ width: 100 }}
                />
              </div>
              {/* 文件大小筛选 */}
              <div style={{ marginTop: 8 }}>
                <Text style={{ fontSize: 13 }}>文件大小:</Text>
              </div>
              <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginTop: 4 }}>
                <InputNumber
                  size="small"
                  placeholder="最小"
                  min={0}
                  value={minSizeValue}
                  onChange={(value) => setMinSizeValue(value)}
                  style={{ width: 70 }}
                />
                <Select
                  size="small"
                  value={minSizeUnit}
                  onChange={(value) => setMinSizeUnit(value)}
                  style={{ width: 65 }}
                  options={[
                    { value: 'KB', label: 'KB' },
                    { value: 'MB', label: 'MB' },
                    { value: 'GB', label: 'GB' },
                  ]}
                />
                <Text style={{ fontSize: 12 }}>~</Text>
                <InputNumber
                  size="small"
                  placeholder="最大"
                  min={0}
                  value={maxSizeValue}
                  onChange={(value) => setMaxSizeValue(value)}
                  style={{ width: 70 }}
                />
                <Select
                  size="small"
                  value={maxSizeUnit}
                  onChange={(value) => setMaxSizeUnit(value)}
                  style={{ width: 65 }}
                  options={[
                    { value: 'KB', label: 'KB' },
                    { value: 'MB', label: 'MB' },
                    { value: 'GB', label: 'GB' },
                  ]}
                />
              </div>
              <div style={{ marginTop: 8 }}>
                <Space wrap size={[4, 4]}>
                  <Button size="small" type="text" onClick={() => handleQuickSize(1, 'KB')}>1KB</Button>
                  <Button size="small" type="text" onClick={() => handleQuickSize(1, 'MB')}>1MB</Button>
                  <Button size="small" type="text" onClick={() => handleQuickSize(10, 'MB')}>10MB</Button>
                  <Button size="small" type="text" onClick={() => handleQuickSize(100, 'MB')}>100MB</Button>
                  <Button size="small" type="text" onClick={() => handleQuickSize(1, 'GB')}>1GB</Button>
                  <Button size="small" type="text" onClick={() => handleQuickSize(10, 'GB')}>10GB</Button>
                </Space>
              </div>
              {(minSizeValue || maxSizeValue) && (
                <Text type="secondary" style={{ fontSize: 11, display: 'block', marginTop: 4 }}>
                  范围: {formatSizeRange()}
                </Text>
              )}
            </Space>
          </Collapse.Panel>

          <Collapse.Panel
            key="history"
            header={
              <div className="collapse-header">
                <div className="collapse-header-left">
                  <HistoryOutlined style={{ color: '#faad14', fontSize: 14 }} />
                  <Text strong>搜索历史</Text>
                  <Text type="secondary" style={{ fontSize: 12 }}>({history.length})</Text>
                </div>
                {history.length > 0 && (
                  <Button
                    type="text"
                    size="small"
                    danger
                    icon={<DeleteOutlined />}
                    onClick={(e) => {
                      e.stopPropagation();
                      handleClearHistory();
                    }}
                  />
                )}
              </div>
            }
          >
            {history.length === 0 ? (
              <Text type="secondary" style={{ fontSize: 12, display: 'block', textAlign: 'center', padding: '12px 0' }}>
                暂无搜索历史
              </Text>
            ) : (
              <List
                size="small"
                dataSource={history.slice(0, 10)}
                renderItem={(item) => (
                  <List.Item
                    className="history-item"
                    onClick={async () => {
                      if (loading) {
                        await handleCancelSearch();
                      }
                      setSearchKeyword(item.query);
                      handleSearch(item.query);
                    }}
                  >
                    <Text ellipsis>{item.query}</Text>
                    <Text type="secondary" style={{ fontSize: 11, marginLeft: 8 }}>({item.result_count})</Text>
                  </List.Item>
                )}
              />
            )}
          </Collapse.Panel>

        </Collapse>
      </div>

      <div
        className={`resizer ${isResizing ? 'active' : ''}`}
        onMouseDown={handleMouseDown}
      />

      <div className="main-content">
        <div className="search-bar">
          <Space.Compact style={{ flex: 1 }}>
            <Input
              placeholder="输入文件名或内容搜索..."
              size="large"
              value={searchKeyword}
              onChange={(e) => setSearchKeyword(e.target.value)}
              onPressEnter={() => {
                if (!loading && searchKeyword.trim()) {
                  handleSearch(searchKeyword);
                }
              }}
              style={{ flex: 1 }}
              disabled={loading}
            />
            <Button
              size="large"
              type={loading ? "default" : "primary"}
              onClick={loading ? handleCancelSearch : () => {
                if (searchKeyword.trim()) {
                  handleSearch(searchKeyword);
                }
              }}
            >
            {loading ? "取消" : "搜索"}
            </Button>
          </Space.Compact>
          <Dropdown
            menu={{
              items: [
                {
                  key: 'refresh',
                  icon: <SyncOutlined spin={refreshLoading} />,
                  label: '更新索引',
                  onClick: handleRefreshIndex,
                  disabled: refreshLoading,
                },
                {
                  key: 'debug',
                  icon: <BugOutlined />,
                  label: 'USN 调试',
                  onClick: () => setShowDebugPanel(!showDebugPanel),
                },
              ],
            }}
            trigger={['click']}
          >
            <Button
              type="text"
              size="large"
              icon={<MoreOutlined />}
              style={{ marginLeft: 8 }}
              title="更多"
            />
          </Dropdown>
        </div>

        {/* USN 调试面板 */}
        {showDebugPanel && (
          <div style={{ padding: '12px 16px', borderBottom: '1px solid #f0f0f0', background: '#fafafa' }}>
            <Space wrap>
              <Text strong>USN 调试:</Text>
              <Input
                placeholder="盘符，如 D:"
                value={usnVolume}
                onChange={(e) => setUsnVolume(e.target.value)}
                style={{ width: 100 }}
                size="small"
              />
              <InputNumber
                placeholder="数量"
                value={usnCount}
                onChange={(value) => setUsnCount(value || 10)}
                min={1}
                max={100}
                style={{ width: 80 }}
                size="small"
              />
              <Button
                size="small"
                type="primary"
                onClick={handleGetRecentUsn}
                loading={usnLoading}
              >
                获取 USN
              </Button>
            </Space>
            {usnRecords.length > 0 && (
              <div style={{ marginTop: 12, maxHeight: 300, overflow: 'auto' }}>
                <Table
                  size="small"
                  dataSource={usnRecords}
                  rowKey="usn"
                  columns={[
                    { title: 'USN', dataIndex: 'usn', key: 'usn', width: 120 },
                    { title: '文件名', dataIndex: 'file_name', key: 'file_name', ellipsis: true },
                    { title: '原因', dataIndex: 'reason_text', key: 'reason_text', width: 180 },
                    { title: '时间', dataIndex: 'timestamp', key: 'timestamp', width: 180 },
                  ]}
                  pagination={false}
                  scroll={{ y: 200 }}
                />
              </div>
            )}
          </div>
        )}

        <div className="results-container" ref={containerRef}>
          {loading && searchProgress && searchProgress.scanned_files > 0 && (
            <div className="results-header" style={{ padding: '8px 16px', borderBottom: '1px solid #f0f0f0' }}>
              <Progress
                percent={Math.min(99, Math.floor((searchProgress.scanned_files / Math.max(1, searchProgress.scanned_files + searchProgress.estimated_remaining * 100)) * 100))}
                size="small"
                showInfo={false}
                status="active"
                strokeColor="#1890ff"
              />
            </div>
          )}
          <Table
            columns={columns}
            dataSource={results}
            rowKey="path"
            size="small"
            rowSelection={{
              selectedRowKeys,
              onChange: (newSelectedRowKeys: React.Key[], selectedRows: SearchResult[], info: { type: RowSelectMethod }) => {
                // 判断是否按住了Shift键
                if (info.type === 'multiple' && lastSelectedRow !== null) {
                  // Shift键多选：计算选中范围
                  const currentIndex = selectedRows.findIndex(row => row.path === newSelectedRowKeys[newSelectedRowKeys.length - 1]);
                  if (currentIndex !== -1 && currentIndex !== lastSelectedRow) {
                    // 获取当前页的数据（考虑分页）
                    const start = Math.min(lastSelectedRow, currentIndex);
                    const end = Math.max(lastSelectedRow, currentIndex);

                    // 获取当前页的所有行的key
                    const currentPageData = results;
                    const rangeKeys = currentPageData.slice(start, end + 1).map(item => item.path);

                    // 合并选中的keys（去重）
                    const mergedKeys = [...new Set([...newSelectedRowKeys, ...rangeKeys])];
                    setSelectedRowKeys(mergedKeys);
                    setLastSelectedRow(currentIndex);
                    return;
                  }
                }

                // 普通选择：更新选中状态和记录最后选中的行索引
                setSelectedRowKeys(newSelectedRowKeys);
                const currentIndex = selectedRows.length > 0
                  ? results.findIndex(row => row.path === newSelectedRowKeys[newSelectedRowKeys.length - 1])
                  : null;
                setLastSelectedRow(currentIndex !== -1 ? currentIndex : null);
              },
            }}
            pagination={{
              current: currentPage,
              pageSize: pageSize,
              showSizeChanger: true,
              pageSizeOptions: ['10', '20', '25', '50', '100'],
              showTotal: (total: number) => `共 ${total} 条`,
              onChange: handlePaginationChange
            }}
            loading={loading}
            scroll={{ y: tableHeight }}
            title={() => (
              <Space>
                <Text>找到 {totalResults > 0 ? totalResults : results.length} 个结果</Text>
                {loading && searchProgress && (
                  <Text type="secondary">
                    已花费 {searchProgress.elapsed_time} 秒
                    {searchProgress.estimated_remaining > 0 && `，预计还有 ${searchProgress.estimated_remaining} 秒`}
                  </Text>
                )}
                {!loading && lastSearchTime && (
                  <Text type="secondary">
                    共花费 {lastSearchTime} 秒
                  </Text>
                )}
                {selectedRowKeys.length > 0 && (
                  <>
                    <Button
                      size="small"
                      type="primary"
                      onClick={() => {
                        const selectedItems = results.filter(item => selectedRowKeys.includes(item.path));
                        selectedItems.forEach(item => handleOpenFile(item.path));
                      }}
                    >
                      打开选中 ({selectedRowKeys.length})
                    </Button>
                    <Button
                      size="small"
                      danger
                      onClick={handleBatchDelete}
                    >
                      批量删除 ({selectedRowKeys.length})
                    </Button>
                  </>
                )}
              </Space>
            )}
          />
          {loading && searchProgress && searchProgress.current_path && (
            <div className="results-footer" style={{ padding: '8px 16px', borderTop: '1px solid #f0f0f0', background: '#fafafa', position: 'sticky', bottom: 0 }}>
              <Text type="secondary">🔍 正在搜索: {searchProgress.current_path}</Text>
            </div>
          )}
        </div>
      </div>

      {/* 保存预设弹窗 */}
      <Modal
        title="保存为预设"
        open={savePresetModalVisible}
        onOk={handleSavePreset}
        onCancel={() => {
          setSavePresetModalVisible(false);
          setPresetName('');
        }}
        okText="保存"
        cancelText="取消"
      >
        <Input
          placeholder="请输入预设名称"
          value={presetName}
          onChange={(e) => setPresetName(e.target.value)}
          onPressEnter={handleSavePreset}
        />
        {config.search_paths.length > 0 && (
          <Text type="secondary" style={{ fontSize: 12, display: 'block', marginTop: 8 }}>
            目录: {config.search_paths.join(', ')}
          </Text>
        )}
      </Modal>

      {/* 预设管理弹窗 */}
      <Modal
        title="预设管理"
        open={presetModalVisible}
        onCancel={() => setPresetModalVisible(false)}
        footer={[
          <Button key="create" icon={<FolderOutlined />} onClick={handleCreateEmptyPreset}>
            新建空白预设
          </Button>,
          <Button key="close" type="primary" onClick={() => setPresetModalVisible(false)}>
            关闭
          </Button>,
        ]}
        width={500}
      >
        {config.presets.length === 0 ? (
          <Text type="secondary" style={{ display: 'block', textAlign: 'center', padding: '24px 0' }}>
            暂无预设，请创建或保存预设
          </Text>
        ) : (
          <List
            size="small"
            dataSource={config.presets}
            renderItem={(preset: SearchPreset) => (
              <List.Item
                actions={[
                  editingPreset?.id === preset.id ? (
                    <Space key="save">
                      <Input
                        size="small"
                        defaultValue={preset.name}
                        onPressEnter={(e) => handleRenamePreset(preset.id, (e.target as HTMLInputElement).value)}
                        style={{ width: 100 }}
                        autoFocus
                        onBlur={(e) => handleRenamePreset(preset.id, e.target.value)}
                      />
                      <Button size="small" onClick={() => setEditingPreset(null)}>取消</Button>
                    </Space>
                  ) : (
                    <Space key="actions">
                      <Button
                        size="small"
                        type={config.active_preset_id === preset.id ? 'primary' : 'default'}
                        onClick={() => handleApplyPreset(preset)}
                      >
                        {config.active_preset_id === preset.id ? '当前' : '应用'}
                      </Button>
                      <Button size="small" icon={<EditOutlined />} onClick={() => setEditingPreset(preset)} />
                      <Popconfirm
                        title="确定删除此预设吗？"
                        onConfirm={() => handleDeletePreset(preset.id)}
                        okText="确定"
                        cancelText="取消"
                      >
                        <Button size="small" danger icon={<DeleteOutlined />} />
                      </Popconfirm>
                    </Space>
                  ),
                ]}
              >
                <div>
                  <Text strong>{preset.name}</Text>
                  <Text type="secondary" style={{ fontSize: 12, marginLeft: 8 }}>
                    使用{preset.use_count}次
                  </Text>
                  {preset.search_paths.length > 0 && (
                    <Text type="secondary" style={{ fontSize: 11, display: 'block' }}>
                      {preset.search_paths.join(', ')}
                    </Text>
                  )}
                </div>
              </List.Item>
            )}
          />
        )}
      </Modal>
    </div>
  );
}

function App() {
  return (
    <ConfigProvider
      theme={{
        algorithm: theme.defaultAlgorithm,
        token: {
          // 可在此配置全局主题 token
          // colorPrimary: '#1890ff',
        },
      }}
    >
      <AppContent />
    </ConfigProvider>
  );
}

export default App;

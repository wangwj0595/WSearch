import { useState, useEffect, useCallback, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { ConfigProvider, message, Table, Button, Input, InputNumber, Space, Checkbox, List, Typography, Tooltip, Progress, theme, Collapse } from "antd";
import { FolderOpenOutlined, FolderOutlined, FileTextOutlined, HistoryOutlined, SettingOutlined, DeleteOutlined } from "@ant-design/icons";
import type { SearchResult, SearchConfig, SearchHistory, SearchProgress, SearchCompletedEvent } from "./types";
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

  // 从配置加载折叠面板状态
  useEffect(() => {
    if (config.collapsed_panels && config.collapsed_panels.length > 0) {
      // 收起的面板不显示在 activePanels 中
      const allPanels = ['search', 'exclude', 'options', 'history'];
      setActivePanels(allPanels.filter(p => !config.collapsed_panels.includes(p)));
    }
  }, [config.collapsed_panels]);

  // 保存折叠面板状态
  const saveCollapsedPanels = useCallback((expandedPanels: string[]) => {
    const allPanels = ['search', 'exclude', 'options', 'history'];
    const collapsedPanels = allPanels.filter(p => !expandedPanels.includes(p));
    const newConfig = { ...config, collapsed_panels: collapsedPanels };
    setConfig(newConfig);
    invoke("save_search_config", { config: newConfig });
  }, [config]);

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
        maxResults: config.max_results,
      });
      // 搜索历史会在 search_completed 事件中刷新
    } catch (e) {
      message.error(`搜索失败: ${e}`);
      setLoading(false);
    }
  }, [config]);

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
          <a onClick={() => handleOpenFile(record.path)}>{name}</a>
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
        </div>

        <div className="results-container" ref={containerRef}>
          {loading && searchProgress && searchProgress.scanned_count > 0 && (
            <div className="results-header" style={{ padding: '8px 16px', borderBottom: '1px solid #f0f0f0' }}>
              <Progress
                percent={Math.min(99, Math.floor((searchProgress.scanned_count / Math.max(1, searchProgress.scanned_count + searchProgress.estimated_remaining * 100)) * 100))}
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

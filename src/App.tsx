import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { message, Table, Button, Input, Space, Checkbox, List, Card, Typography, Tooltip } from "antd";
import type { SearchResult, SearchConfig, SearchHistory } from "./types";
import { defaultSearchConfig } from "./types";
import "./App.css";

const { Search } = Input;
const { Text } = Typography;

function App() {
  const [results, setResults] = useState<SearchResult[]>([]);
  const [loading, setLoading] = useState(false);
  const [config, setConfig] = useState<SearchConfig>(defaultSearchConfig);
  const [history, setHistory] = useState<SearchHistory[]>([]);

  // 加载配置和历史
  useEffect(() => {
    loadConfig();
    loadHistory();
  }, []);

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

  // 执行搜索
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
    try {
      const searchResults = await invoke<SearchResult[]>("search_files", {
        query: searchQuery,
        searchPaths: config.search_paths,
        excludePaths: config.exclude_paths,
        fileTypes: config.file_types,
        searchContent: config.search_content,
        caseSensitive: config.case_sensitive,
        maxResults: config.max_results,
      });
      setResults(searchResults);
      loadHistory(); // 刷新历史
    } catch (e) {
      message.error(`搜索失败: ${e}`);
    } finally {
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

  // 表格列定义
  const columns = [
    {
      title: "名称",
      dataIndex: "name",
      key: "name",
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
      render: (size: number) => size > 0 ? `${(size / 1024).toFixed(1)} KB` : "-",
    },
    {
      title: "修改时间",
      dataIndex: "modified_time",
      key: "modified_time",
      width: 180,
      render: (time: string) => <Text type="secondary" style={{ fontSize: 12 }}>{time}</Text>,
    },
    {
      title: "操作",
      key: "action",
      width: 150,
      render: (_: unknown, record: SearchResult) => (
        <Space size="small">
          <Button size="small" onClick={() => handleOpenFile(record.path)}>打开</Button>
          <Button size="small" onClick={() => handleReveal(record.path)}>定位</Button>
        </Space>
      ),
    },
  ];

  return (
    <div className="app-container">
      <div className="sidebar">
        <Card size="small" title="搜索目录" extra={<Button type="link" size="small" onClick={addSearchPath}>+ 添加</Button>}>
          <List
            size="small"
            dataSource={config.search_paths}
            renderItem={(item) => (
              <List.Item
                actions={[<Button key="remove" type="link" danger size="small" onClick={() => removeSearchPath(item)}>×</Button>]}
              >
                <Text ellipsis style={{ fontSize: 12 }}>📁 {item}</Text>
              </List.Item>
            )}
          />
        </Card>

        <Card size="small" title="排除目录" extra={<Button type="link" size="small" onClick={addExcludePath}>+ 添加</Button>}>
          <List
            size="small"
            dataSource={config.exclude_paths}
            renderItem={(item) => (
              <List.Item
                actions={[<Button key="remove" type="link" danger size="small" onClick={() => removeExcludePath(item)}>×</Button>]}
              >
                <Text ellipsis style={{ fontSize: 12 }}>🚫 {item}</Text>
              </List.Item>
            )}
          />
        </Card>

        <Card size="small" title="搜索选项">
          <Space direction="vertical" style={{ width: "100%" }} size="small">
            <Checkbox
              checked={config.search_content}
              onChange={(e) => {
                const newConfig = { ...config, search_content: e.target.checked };
                setConfig(newConfig);
                invoke("save_search_config", { config: newConfig });
              }}
            >
              搜索文件内容
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
          </Space>
        </Card>

        <Card size="small" title="搜索历史" extra={
          history.length > 0 && <Button type="link" size="small" danger onClick={handleClearHistory}>清除</Button>
        }>
          <List
            size="small"
            dataSource={history.slice(0, 10)}
            renderItem={(item) => (
              <List.Item style={{ cursor: "pointer", padding: "4px 8px" }} onClick={() => handleSearch(item.query)}>
                <Text ellipsis>{item.query}</Text>
                <Text type="secondary" style={{ fontSize: 10 }}>({item.result_count})</Text>
              </List.Item>
            )}
          />
        </Card>
      </div>

      <div className="main-content">
        <div className="search-bar">
          <Search
            placeholder="输入文件名或内容搜索..."
            enterButton="搜索"
            size="large"
            loading={loading}
            onSearch={handleSearch}
            style={{ flex: 1 }}
          />
        </div>

        <div className="results-container">
          <div className="results-header">
            <Text>找到 {results.length} 个结果</Text>
          </div>
          <Table
            columns={columns}
            dataSource={results}
            rowKey="path"
            size="small"
            pagination={{ pageSize: 20 }}
            loading={loading}
          />
        </div>
      </div>
    </div>
  );
}

export default App;

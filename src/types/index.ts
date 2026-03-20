// 搜索结果
export interface SearchResult {
  name: string;
  path: string;
  is_directory: boolean;
  size: number;
  modified_time: string;
  match_content?: string;
}

// 搜索配置
export interface SearchConfig {
  search_paths: string[];
  exclude_paths: string[];
  file_types: string[];
  search_content: boolean;
  case_sensitive: boolean;
  search_directories: boolean;
  use_mft: boolean;
  max_results: number;
  sidebar_width: number;
  collapsed_panels: string[];
}

// 搜索历史
export interface SearchHistory {
  query: string;
  timestamp: string;
  result_count: number;
}

// 搜索进度
export interface SearchProgress {
  scanned_count: number;
  found_count: number;
  current_path: string;
  elapsed_time: number;
  estimated_remaining: number;
}

// 搜索完成事件数据
export interface SearchCompletedEvent {
  result_count: number;
  elapsed_time: number;
}

// 窗口配置
export interface WindowConfig {
  width: number;
  height: number;
  x: number;
  y: number;
  is_maximized: boolean;
}

// 默认配置
export const defaultSearchConfig: SearchConfig = {
  search_paths: [],
  exclude_paths: ['node_modules', '.git', 'target', 'dist', 'build'],
  file_types: [],
  search_content: false,
  case_sensitive: false,
  search_directories: true,
  use_mft: false,
  max_results: 3000,
  sidebar_width: 280,
  collapsed_panels: [],
};

// 默认窗口配置
export const defaultWindowConfig: WindowConfig = {
  width: 800,
  height: 600,
  x: 0,
  y: 0,
  is_maximized: false,
};

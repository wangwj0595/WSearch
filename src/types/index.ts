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
  max_results: number;
}

// 搜索历史
export interface SearchHistory {
  query: string;
  timestamp: string;
  result_count: number;
}

// 默认配置
export const defaultSearchConfig: SearchConfig = {
  search_paths: [],
  exclude_paths: ['node_modules', '.git', 'target', 'dist', 'build'],
  file_types: [],
  search_content: false,
  case_sensitive: false,
  max_results: 1000,
};

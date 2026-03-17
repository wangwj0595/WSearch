import { useEffect, useRef, useCallback } from 'react';
import { getCurrentWindow, LogicalSize, LogicalPosition } from '@tauri-apps/api/window';
import { invoke } from '@tauri-apps/api/core';
import { WindowConfig, defaultWindowConfig } from '../types';

export function useWindowSize() {
  const saveTimeoutRef = useRef<number | null>(null);
  const isInitializedRef = useRef(false);

  // 保存窗口配置到后端
  const saveWindowConfig = useCallback(async (config: WindowConfig) => {
    try {
      await invoke('save_window_config', { windowConfig: config });
    } catch (error) {
      console.error('Failed to save window config:', error);
    }
  }, []);

  // 加载窗口配置
  const loadWindowConfig = useCallback(async (): Promise<WindowConfig> => {
    try {
      const config = await invoke<WindowConfig>('get_window_config');
      return config;
    } catch (error) {
      console.error('Failed to load window config:', error);
      return defaultWindowConfig;
    }
  }, []);

  // 应用窗口大小
  const applyWindowSize = useCallback(async (config: WindowConfig) => {
    try {
      const appWindow = getCurrentWindow();

      // 只在有有效值时设置
      if (config.width > 0 && config.height > 0) {
        await appWindow.setSize(new LogicalSize(config.width, config.height));
      }

      // 如果位置有效（x 和 y 都不为 0），则设置位置
      if (config.x !== 0 || config.y !== 0) {
        await appWindow.setPosition(new LogicalPosition(config.x, config.y));
      }
    } catch (error) {
      console.error('Failed to apply window size:', error);
    }
  }, []);

  // 保存当前的窗口配置（带防抖）
  const saveCurrentWindowSize = useCallback(async () => {
    if (!isInitializedRef.current) return;

    // 清除之前的定时器
    if (saveTimeoutRef.current) {
      clearTimeout(saveTimeoutRef.current);
    }

    // 延迟保存，避免频繁写入
    saveTimeoutRef.current = window.setTimeout(async () => {
      try {
        const appWindow = getCurrentWindow();
        const size = await appWindow.outerSize();
        const position = await appWindow.outerPosition();

        const config: WindowConfig = {
          width: size.width,
          height: size.height,
          x: position.x,
          y: position.y,
        };

        await saveWindowConfig(config);
      } catch (error) {
        console.error('Failed to save window size:', error);
      }
    }, 500);
  }, [saveWindowConfig]);

  useEffect(() => {
    const initWindow = async () => {
      // 加载并应用窗口配置
      const config = await loadWindowConfig();
      await applyWindowSize(config);
      isInitializedRef.current = true;
    };

    initWindow();

    // 监听窗口大小变化
    const appWindow = getCurrentWindow();

    const unlistenResize = appWindow.onResized(() => {
      saveCurrentWindowSize();
    });

    const unlistenMove = appWindow.onMoved(() => {
      saveCurrentWindowSize();
    });

    // 清理
    return () => {
      if (saveTimeoutRef.current) {
        clearTimeout(saveTimeoutRef.current);
      }
      unlistenResize.then(fn => fn());
      unlistenMove.then(fn => fn());
    };
  }, [loadWindowConfig, applyWindowSize, saveCurrentWindowSize]);
}

use crate::models::WindowConfig;
use crate::services::ConfigStore;
use tauri::State;

#[tauri::command]
pub fn save_window_config(
    window_config: WindowConfig,
    config_store: State<'_, ConfigStore>,
) -> Result<(), String> {
    config_store.save_window_config(window_config)
}

#[tauri::command]
pub fn get_window_config(config_store: State<'_, ConfigStore>) -> WindowConfig {
    config_store.get_window_config()
}

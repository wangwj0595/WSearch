pub mod config_store;
pub mod file_scanner;
pub mod index_cache;
pub mod mft_reader;
pub mod usn_monitor;

pub use config_store::ConfigStore;
pub use index_cache::{init_cache, get_cache_manager, CacheManager, SearchResultEntry};
pub use mft_reader::{MftFileEntry, scan_volume_files, is_ntfs_volume, is_running_as_admin};
pub use usn_monitor::{start_incremental_service, stop_incremental_service, get_incremental_updater, IncrementalUpdater, save_usn_state, trigger_incremental_update};

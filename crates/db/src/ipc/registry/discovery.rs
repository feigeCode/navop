use one_core::storage::get_config_dir;
use std::path::PathBuf;

const EXTENSIONS_DIR_NAME: &str = "extensions";
const DATABASE_DRIVERS_DIR_NAME: &str = "database_drivers";

pub(super) fn default_driver_dirs() -> Vec<PathBuf> {
    vec![default_user_driver_dir()]
}

pub(super) fn default_user_driver_dir() -> PathBuf {
    get_config_dir()
        .map(|dir| {
            dir.join(EXTENSIONS_DIR_NAME)
                .join(DATABASE_DRIVERS_DIR_NAME)
        })
        .unwrap_or_else(|_| PathBuf::from(EXTENSIONS_DIR_NAME).join(DATABASE_DRIVERS_DIR_NAME))
}

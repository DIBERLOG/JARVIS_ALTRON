pub mod structs;
pub mod manager;

use crate::{config, APP_CONFIG_DIR};

use log::info;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

pub use manager::SettingsManager;

fn get_db_file_path() -> PathBuf {
    PathBuf::from(format!(
        "{}/{}",
        APP_CONFIG_DIR.get().unwrap().display(),
        config::DB_FILE_NAME
    ))
}

pub fn init_settings() -> structs::Settings {
    let db_file_path = get_db_file_path();

    info!(
        "Loading settings db file located at: {}",
        db_file_path.display()
    );

    if db_file_path.exists() {
        if let Ok(db_file) = File::open(&db_file_path) {
            let reader = BufReader::new(db_file);
            if let Ok(settings) = serde_json::from_reader(reader) {
                info!("Settings loaded.");
                return settings;
            }
        }
    }

    warn!("No settings file found or there was an error parsing it. Creating default struct.");
    structs::Settings::default()
}

/// init settings and return a SettingsManager ready to use
pub fn init() -> SettingsManager {
    let settings = init_settings();
    SettingsManager::new(settings)
}

pub fn save_settings(settings: &structs::Settings) -> Result<(), std::io::Error> {
    let db_file_path = get_db_file_path();

    // Atomic: a crash mid-write must never leave a truncated settings file, and a
    // damaged file must not stop the application on the next start.
    crate::fsutil::write_json_atomic(&db_file_path, settings)?;

    info!("Settings saved to: {:#}", db_file_path.display());
    Ok(())
}

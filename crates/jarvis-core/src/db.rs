pub mod manager;
pub mod structs;

use crate::{config, APP_CONFIG_DIR};

use altron_command_engine::persistence;
use log::info;
use std::io;
use std::path::{Path, PathBuf};

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

    match read_settings_result(&db_file_path){
        Ok(settings)=>{info!("Settings loaded.");settings},
        Err(error) if error.kind()==io::ErrorKind::NotFound=>structs::Settings::default(),
        Err(error)=>panic!("Не удалось открыть настройки; исходные данные не изменены: {error}"),
    }
}

/// init settings and return a SettingsManager ready to use
pub fn init() -> SettingsManager {
    let settings = init_settings();
    SettingsManager::new(settings)
}

pub fn save_settings(settings: &structs::Settings) -> Result<(), std::io::Error> {
    let db_file_path = get_db_file_path();
    save_settings_at(&db_file_path, settings)?;

    info!("Settings saved to: {:#}", db_file_path.display());
    Ok(())
}

fn read_settings_at(path: &Path) -> Option<structs::Settings> {
    read_settings_result(path).ok()
}

fn read_settings_result(path:&Path)->io::Result<structs::Settings>{let value:serde_json::Value=persistence::read(path)?;let plain=if crate::secure_storage::is_sealed(&value){crate::secure_storage::open(&value,"settings").map_err(io::Error::other)?}else{value};serde_json::from_value(plain).map_err(io::Error::other)}

fn save_settings_at(path: &Path, settings: &structs::Settings) -> io::Result<()> {
    persistence::transaction_with_codec(path,settings,|latest|{*latest=settings.clone();Ok(())},decode_settings,encode_settings).map(|_|()).map_err(io::Error::other)
}

fn decode_settings(value:serde_json::Value)->Result<serde_json::Value,String>{if crate::secure_storage::is_sealed(&value){crate::secure_storage::open(&value,"settings")}else{Ok(value)}}
fn encode_settings(value:&serde_json::Value)->Result<serde_json::Value,String>{crate::secure_storage::seal(value,"settings")}

/// Serializes read-modify-write across GUI and assistant processes. Each write
/// starts from the latest disk state, so a stale process cannot reset settings.
pub fn update_settings<F>(
    fallback: &structs::Settings,
    change: F,
) -> Result<structs::Settings, String>
where
    F: FnOnce(&mut structs::Settings) -> Result<(), String>,
{
    let path = get_db_file_path();
    let updated = persistence::transaction_with_codec(&path, fallback, change,decode_settings,encode_settings)?;
    info!("ALTRON kernel: settings transaction committed");
    Ok(updated)
}

pub fn latest_settings() -> Option<structs::Settings> {
    read_settings_at(&get_db_file_path())
}

#[cfg(test)]
mod persistence_tests {
    use super::*;

    #[test]
    fn legacy_settings_replace_existing_file_and_survive_reload() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("app.db");
        let mut settings = structs::Settings::default();
        persistence::replace(&path, &settings).unwrap();
        settings.set("tts_mode", "silero").unwrap();
        persistence::replace(&path, &settings).unwrap();
        assert_eq!(
            read_settings_at(&path).unwrap().get("tts_mode").as_deref(),
            Some("silero")
        );
        settings.set("tts_mode", "xtts").unwrap();
        persistence::replace(&path, &settings).unwrap();
        assert_eq!(
            read_settings_at(&path).unwrap().get("tts_mode").as_deref(),
            Some("xtts")
        );
    }
}

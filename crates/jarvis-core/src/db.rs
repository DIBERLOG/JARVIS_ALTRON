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
    if crate::sync_profile::enabled(){update_settings(settings,|latest|{*latest=settings.clone();Ok(())}).map_err(io::Error::other)?;}
    else{save_settings_at(&db_file_path, settings)?;}

    info!("Settings saved to: {:#}", db_file_path.display());
    Ok(())
}

fn read_settings_at(path: &Path) -> Option<structs::Settings> {
    read_settings_result(path).ok()
}

fn read_settings_result(path:&Path)->io::Result<structs::Settings>{
    if crate::sync_profile::enabled() && crate::APP_CONFIG_DIR.get().is_some_and(|root|path.parent()==Some(root.as_path())){
        return serde_json::from_value(crate::sync_profile::read().map_err(io::Error::other)?).map_err(io::Error::other);
    }
    let value:serde_json::Value=persistence::read(path)?;let plain=if crate::secure_storage::is_sealed(&value){crate::secure_storage::open(&value,"settings").map_err(io::Error::other)?}else{value};serde_json::from_value(plain).map_err(io::Error::other)}

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
    if crate::sync_profile::enabled(){
        let initial=serde_json::to_value(fallback).map_err(|_|"Некорректные настройки")?;
        let result=crate::sync_profile::update(&initial,|value|{
            let mut settings:structs::Settings=serde_json::from_value(value.clone()).map_err(|_|"Повреждены настройки v2")?;
            change(&mut settings)?;
            let typed=serde_json::to_value(settings).map_err(|_|"Некорректные настройки")?;
            value.as_object_mut().ok_or("Некорректный снимок")?.extend(typed.as_object().ok_or("Некорректные настройки")?.clone());Ok(())
        })?;
        return serde_json::from_value(result).map_err(|_|"Повреждены настройки v2".into());
    }
    let path = get_db_file_path();
    let updated = persistence::transaction_with_codec(&path, fallback, change,decode_settings,encode_settings)?;
    info!("ALTRON kernel: settings transaction committed");
    Ok(updated)
}

/// Validated source for opt-in migration, retaining fields unknown to this build.
pub fn migration_snapshot(fallback:&structs::Settings)->Result<serde_json::Value,String>{
    let path=get_db_file_path();let mut value=if path.exists(){decode_settings(persistence::read(&path).map_err(|_|"Не удалось прочитать исходные настройки")?)?}else{serde_json::to_value(fallback).map_err(|_|"Некорректные настройки")?};
    let _:structs::Settings=serde_json::from_value(value.clone()).map_err(|_|"Исходные настройки повреждены")?;
    let vault=crate::sync_profile::root()?.join("password-vault-v1.json");if vault.exists(){let encrypted:serde_json::Value=persistence::read(&vault).map_err(|_|"Повреждён исходный сейф")?;crate::sync_data::validate_vault(&encrypted)?;value["sync_vault"]=encrypted;}Ok(value)
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

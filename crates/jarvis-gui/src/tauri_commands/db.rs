use crate::AppState;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct SettingEntry {
    key: String,
    val: String,
}

#[tauri::command]
pub fn db_read(state: tauri::State<'_, AppState>, key: &str) -> String {
    state.settings.read(key).unwrap_or_default()
}

#[tauri::command]
pub fn db_write(state: tauri::State<'_, AppState>, key: &str, val: &str) -> bool {
    match state.settings.write(key, val) {
        Ok(()) => true,
        Err(e) => {
            log::warn!("db_write('{}') failed: {}", key, e);
            false
        }
    }
}

#[tauri::command]
pub fn db_write_many(state: tauri::State<'_, AppState>, entries: Vec<SettingEntry>) -> Result<(), String> {
    let pairs: Vec<(&str, &str)> = entries.iter()
        .map(|entry| (entry.key.as_str(), entry.val.as_str()))
        .collect();
    state.settings.write_many(&pairs)
}

#[tauri::command]
pub fn center_restore_data(state: tauri::State<'_, AppState>, data: String, city: String) -> Result<(), String> {
    if data.len() > 100 * 1024 * 1024 { return Err("Слишком большой файл данных".into()); }
    let parsed: serde_json::Value = serde_json::from_str(&data).map_err(|_| "Некорректный JSON")?;
    if !["notes", "reminders", "birthdays", "habits"].iter().all(|key| parsed.get(*key).is_some_and(|v|v.is_array())) {
        return Err("Некорректные данные Центра".into());
    }
    let previous_city = jarvis_core::weather_city::get();
    jarvis_core::weather_city::set(&city)?;
    if let Err(error) = state.settings.write_many(&[("center_data", &data), ("center_reminder_announcements", "{}")]) {
        return match jarvis_core::weather_city::set(&previous_city) {
            Ok(()) => Err(error),
            Err(rollback) => Err(format!("{error}. Город не удалось восстановить: {rollback}")),
        };
    }
    Ok(())
}

// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use jarvis_core::{config, db, i18n, voices, DB, SettingsManager};
use tauri::Manager;

#[macro_use]
extern crate simple_log;

mod events;

mod tauri_commands;

#[derive(Clone)]
pub struct AppState {
    pub settings: SettingsManager,
    pub notes: tauri_commands::NotesHandle,
    pub vault: tauri_commands::VaultHandle,
}

fn main() {
    config::init_dirs().expect("Failed to init dirs");
    
    // basic logging setup (simpler for GUI)
    simple_log::quick!("info");

    // init settings
    let manager = db::init();

    // init i18n
    i18n::init(&manager.lock().language);

    // init voices
    if let Err(e) = voices::init(&manager.lock().voice, &manager.lock().language) {
        eprintln!("Failed to init voices: {}", e);
    }

    // init audio backend
    if let Err(e) = jarvis_core::audio::init() {
        eprintln!("Failed to init audio: {:?}", e);
    }

    // set global DB (for core modules that read settings at init time)
    DB.set(manager.arc().clone())
            .expect("DB already initialized");

    // open the encrypted notes storage eagerly; a failure is logged and the
    // notes page can still show the storage gate
    let notes = tauri_commands::NotesHandle::new();
    notes.preload();
    // the password vault shares that key session, and adds its own idle timer
    // (which also cancels clipboard timers) and its own derived working key
    let vault = tauri_commands::VaultHandle::new(notes.clone());

    tauri::Builder::default()
        .manage(AppState {
            settings: manager,
            notes,
            vault,
        })
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .invoke_handler(tauri::generate_handler![
            // audio
            tauri_commands::pv_get_audio_devices,
            tauri_commands::pv_get_audio_device_name,
            tauri_commands::play_sound,

            // db
            tauri_commands::db_read,
            tauri_commands::db_write,

            // etc
            tauri_commands::get_app_version,
            tauri_commands::get_author_name,
            tauri_commands::get_repository_link,
            tauri_commands::get_tg_official_link,
            tauri_commands::get_boosty_link,
            tauri_commands::get_patreon_link,
            tauri_commands::get_feedback_link,

            // fs
            tauri_commands::get_log_file_path,
            tauri_commands::show_in_folder,

            // sys
            tauri_commands::get_current_ram_usage,
            tauri_commands::get_peak_ram_usage,
            tauri_commands::get_cpu_temp,
            tauri_commands::get_cpu_usage,
            tauri_commands::get_jarvis_app_stats,
            tauri_commands::is_jarvis_app_running,
            tauri_commands::run_jarvis_app,

            // vosk
            tauri_commands::list_vosk_models,

            // gliner
            tauri_commands::list_gliner_models,

            // i18n
            tauri_commands::get_translations,
            tauri_commands::translate,
            tauri_commands::get_current_language,
            tauri_commands::set_language,
            tauri_commands::get_supported_languages,

            // commands
            tauri_commands::get_commands_count,
            tauri_commands::get_commands_list,

            // voices
            tauri_commands::list_voices,
            tauri_commands::get_voice,
            tauri_commands::preview_voice,

            // notes (encrypted local storage)
            tauri_commands::notes_status,
            tauri_commands::notes_initialize,
            tauri_commands::notes_unlock_dpapi,
            tauri_commands::notes_unlock_password,
            tauri_commands::notes_lock,
            tauri_commands::notes_import_backup,
            tauri_commands::notes_import_backup_file,
            tauri_commands::notes_export_backup,
            tauri_commands::notes_export_backup_file,
            tauri_commands::notes_list,
            tauri_commands::notes_get,
            tauri_commands::notes_create,
            tauri_commands::notes_update,
            tauri_commands::notes_autosave,
            tauri_commands::notes_set_pinned,
            tauri_commands::notes_trash,
            tauri_commands::notes_restore,
            tauri_commands::notes_purge,
            tauri_commands::notes_folders,
            tauri_commands::notes_create_folder,
            tauri_commands::notes_rename_folder,
            tauri_commands::notes_trash_folder,
            tauri_commands::notes_restore_folder,
            tauri_commands::notes_purge_folder,
            tauri_commands::notes_tags,
            tauri_commands::notes_conflicts,
            tauri_commands::notes_resolve_conflict,

            // password vault (encrypted local storage)
            tauri_commands::vault_status,
            tauri_commands::vault_initialize,
            tauri_commands::vault_unlock_password,
            tauri_commands::vault_unlock_dpapi,
            tauri_commands::vault_lock,
            tauri_commands::vault_import_backup,
            tauri_commands::vault_import_backup_file,
            tauri_commands::vault_export_backup_file,
            tauri_commands::vault_change_master_password,
            tauri_commands::vault_set_idle_timeout,
            tauri_commands::vault_idle_status,
            tauri_commands::vault_touch,
            tauri_commands::vault_list,
            tauri_commands::vault_get,
            tauri_commands::vault_create,
            tauri_commands::vault_update,
            tauri_commands::vault_update_metadata,
            tauri_commands::vault_update_secrets,
            tauri_commands::vault_set_favorite,
            tauri_commands::vault_trash,
            tauri_commands::vault_restore,
            tauri_commands::vault_purge,
            tauri_commands::vault_tags,
            tauri_commands::vault_reveal,
            tauri_commands::vault_copy_username,
            tauri_commands::vault_copy_password,
            tauri_commands::vault_clipboard_status,
            tauri_commands::vault_clipboard_clear,
            tauri_commands::vault_generate_password,
            tauri_commands::vault_generate_and_copy,
            tauri_commands::vault_conflicts,
            tauri_commands::vault_resolve_conflict,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            // Lock the encrypted storage when the application is closing, so the
            // master key and the clipboard timer never outlive the window.
            if matches!(
                event,
                tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit
            ) {
                if let Some(state) = app_handle.try_state::<AppState>() {
                    state.vault.lock_for_exit();
                }
            }
        });
}

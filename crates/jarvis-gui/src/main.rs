// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use jarvis_core::{config, db, i18n, voices, DB, SettingsManager};

#[macro_use]
extern crate simple_log;

mod events;
mod ollama;

mod tauri_commands;
mod window_motion;
mod listening_shortcut;

#[derive(Clone)]
pub struct AppState {
    pub settings: SettingsManager,
}

fn main() {
    if std::env::args().any(|arg| arg == "--release-smoke-test") {
        let root = &*jarvis_core::APP_DIR;
        let core = std::env::current_exe().expect("Executable path unavailable")
            .with_file_name(if cfg!(windows) { "jarvis-app.exe" } else { "jarvis-app" });
        assert!(core.is_file() && root.join("resources/commands").is_dir(), "Incomplete release package");
        return;
    }
    config::init_dirs().expect("Failed to init dirs");
    
    // basic logging setup (simpler for GUI)
    simple_log::quick!("info");

    // init settings
    let manager = db::init();
    ollama::start_in_background();
    jarvis_core::tts::prewarm_silero();

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

    tauri::Builder::default()
        .setup(|app| {
            use tauri::Manager;
            if let Err(error) = tauri_commands::run_jarvis_app() {
                eprintln!("Не удалось автоматически запустить голосовое ядро: {error}");
            }
            if let Some(window) = app.get_webview_window("main") {
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(15));
                    if !window_motion::entrance_requested() && window.is_visible().ok() == Some(false) {
                        let _ = window.center();
                        let _ = window.show();
                    }
                });
            }
            Ok(())
        })
        .manage(AppState { settings: manager })
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().with_handler(|app, _, event| {
            if event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed {
                use tauri::Emitter;
                let _ = app.emit("toggle-listening-shortcut", ());
            }
        }).build())
        .manage(listening_shortcut::ListeningShortcut::default())
        .invoke_handler(tauri::generate_handler![
            window_motion::animate_window_in,
            listening_shortcut::set_listening_shortcut,
            // audio
            tauri_commands::pv_get_audio_devices,
            tauri_commands::pv_get_audio_device_name,
            tauri_commands::play_sound,

            // db
            tauri_commands::db_read,
            tauri_commands::db_write,
            tauri_commands::db_write_many,
            tauri_commands::center_restore_data,

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
            tauri_commands::get_health_status,
            tauri_commands::is_jarvis_app_running,
            tauri_commands::run_jarvis_app,

            // vosk
            tauri_commands::list_vosk_models,
            tauri_commands::center_transcribe_audio,
            tauri_commands::center_punctuate_text,
            tauri_commands::get_jarvis_terminal_log,
            tauri_commands::password_vault_load,
            tauri_commands::password_vault_save,
            tauri_commands::outlook_request,

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

            // chat
            tauri_commands::chat_get_config,
            tauri_commands::chat_send,
            tauri_commands::agent_run,
            tauri_commands::agent_read_attachment,
            tauri_commands::agent_export_docx,
            tauri_commands::agent_chain_step,
            tauri_commands::chat_search_web,
            tauri_commands::chat_get_news,
            tauri_commands::center_get_news,
            tauri_commands::center_translate_news,
            tauri_commands::center_get_weather,
            tauri_commands::center_get_weather_city,
            tauri_commands::chat_is_speaking,
            tauri_commands::chat_stop_speech,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                tauri_commands::stop_owned_jarvis_app();
            }
        });
}

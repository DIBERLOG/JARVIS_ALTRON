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
    pub local_ai: tauri_commands::LocalAiHandle,
    pub memory: tauri_commands::MemoryHandle,
    pub autocorrect: tauri_commands::AutocorrectHandle,
    pub windows_actions: tauri_commands::WindowsActionsHandle,
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

    // the local AI runtime is built from the stored settings; it starts no
    // process until the user asks for one, and it has no handle to the
    // encrypted storages
    let local_ai = tauri_commands::LocalAiHandle::restore(&manager);

    // AI memory reaches the encrypted storage through the shared session, which
    // derives a separate key for it; this handle only tracks summary jobs
    let memory = tauri_commands::MemoryHandle::new();

    // safe Windows actions: one session for the buttons, the voice host, and the
    // model path. It starts no program and touches no window until an action has
    // been checked against the central policy and (for a risky one) confirmed.
    let windows_actions = tauri_commands::WindowsActionsHandle::restore();

    // local spelling: dictionaries on disk, the user's own words in the shared
    // session under their own derived key, and an in-memory undo journal that is
    // cleared whenever the storage locks
    let autocorrect = tauri_commands::AutocorrectHandle::new(manager.clone());
    // Every lock path (idle timeout, explicit lock, exit) goes through the vault
    // handle, so one hook is enough to drop the spelling journals with the keys.
    vault.set_lock_hook({
        let autocorrect = autocorrect.clone();
        std::sync::Arc::new(move || autocorrect.clear_journals())
    });

    tauri::Builder::default()
        .manage(AppState {
            settings: manager,
            notes,
            vault,
            local_ai,
            memory,
            autocorrect,
            windows_actions,
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

            // local AI (managed llama-server runtime)
            tauri_commands::local_ai_get_config,
            tauri_commands::local_ai_validate,
            tauri_commands::local_ai_save_config,
            tauri_commands::local_ai_export_config,
            tauri_commands::local_ai_import_config,
            tauri_commands::local_ai_start,
            tauri_commands::local_ai_stop,
            tauri_commands::local_ai_restart,
            tauri_commands::local_ai_status,
            tauri_commands::local_ai_generate,
            tauri_commands::local_ai_cancel,
            tauri_commands::local_ai_select_server,
            tauri_commands::local_ai_select_model,

            // AI memory (encrypted conversations, summaries, facts)
            tauri_commands::memory_status,
            tauri_commands::memory_get_settings,
            tauri_commands::memory_update_settings,
            tauri_commands::memory_lock,
            tauri_commands::memory_list_conversations,
            tauri_commands::memory_create_conversation,
            tauri_commands::memory_open_conversation,
            tauri_commands::memory_rename_conversation,
            tauri_commands::memory_archive_conversation,
            tauri_commands::memory_delete_conversation,
            tauri_commands::memory_clear_conversation,
            tauri_commands::memory_clear_history,
            tauri_commands::memory_append_user_message,
            tauri_commands::memory_append_assistant_message,
            tauri_commands::memory_delete_message,
            tauri_commands::memory_list_facts,
            tauri_commands::memory_create_fact,
            tauri_commands::memory_update_fact,
            tauri_commands::memory_set_fact_usage,
            tauri_commands::memory_delete_fact,
            tauri_commands::memory_restore_fact,
            tauri_commands::memory_purge_fact,
            tauri_commands::memory_list_candidates,
            tauri_commands::memory_approve_candidate,
            tauri_commands::memory_reject_candidate,
            tauri_commands::memory_build_context,
            tauri_commands::memory_context_budget,
            tauri_commands::memory_summarize,
            tauri_commands::memory_suggest_candidates,
            tauri_commands::memory_export_backup,
            tauri_commands::memory_import_backup,
            tauri_commands::memory_conflicts,
            tauri_commands::memory_resolve_conflict,

            // autocorrect (local spelling, user word list, explicit AI text previews)
            tauri_commands::autocorrect_status,
            tauri_commands::autocorrect_get_settings,
            tauri_commands::autocorrect_update_settings,
            tauri_commands::autocorrect_reload_dictionaries,
            tauri_commands::autocorrect_check,
            tauri_commands::autocorrect_suggest,
            tauri_commands::autocorrect_apply,
            tauri_commands::autocorrect_undo,
            tauri_commands::autocorrect_undo_status,
            tauri_commands::autocorrect_dictionary_list,
            tauri_commands::autocorrect_dictionary_add,
            tauri_commands::autocorrect_dictionary_remove,
            tauri_commands::autocorrect_dictionary_ignore,
            tauri_commands::autocorrect_dictionary_unignore,
            tauri_commands::autocorrect_dictionary_ignored,
            tauri_commands::autocorrect_dictionary_stats,
            tauri_commands::autocorrect_dictionary_import_file,
            tauri_commands::autocorrect_dictionary_export_file,
            tauri_commands::autocorrect_dictionary_export_backup,
            tauri_commands::autocorrect_dictionary_import_backup,
            tauri_commands::autocorrect_improve_text,
            tauri_commands::autocorrect_cancel_improvement,
            tauri_commands::autocorrect_apply_improvement,
            tauri_commands::autocorrect_rule_add,
            tauri_commands::autocorrect_rule_remove,

            // safe Windows actions (typed actions, one policy, one confirmation gate)
            tauri_commands::windows_actions_overview,
            tauri_commands::windows_actions_update_settings,
            tauri_commands::windows_actions_request,
            tauri_commands::windows_actions_pending,
            tauri_commands::windows_actions_confirm,
            tauri_commands::windows_actions_cancel,
            tauri_commands::windows_actions_list_windows,
            tauri_commands::windows_actions_scheduled,
            tauri_commands::windows_actions_take_fired,
            tauri_commands::windows_actions_prune_scheduled,
            tauri_commands::windows_actions_audit_log,
            tauri_commands::windows_actions_clear_audit_log,
            tauri_commands::windows_actions_export_audit_log,
            tauri_commands::windows_actions_add_allowed_application,
            tauri_commands::windows_actions_remove_allowed_application,
            tauri_commands::windows_actions_set_allowed_application_enabled,
            tauri_commands::windows_actions_reaccept_allowed_application,
            tauri_commands::windows_actions_route_voice,
            tauri_commands::windows_actions_tools,
        ])
        .setup(|app| {
            // When a timer or reminder fires the window is poked; the item itself is
            // collected by `windows_actions_take_fired`, so a notification the
            // interface shows can never be the only record of what fired.
            let handle = app.state::<AppState>().windows_actions.clone();
            let app_handle = app.handle().clone();
            handle.install_fired_hook(move |_view| {
                use tauri::Emitter;
                let _ = app_handle.emit("windows-actions-fired", ());
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            // Lock the encrypted storage and stop the managed model server when
            // the application is closing, so neither the master key nor a child
            // process outlives the window.
            if matches!(
                event,
                tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit
            ) {
                if let Some(state) = app_handle.try_state::<AppState>() {
                    // The undo journal holds document text: it is dropped before the
                    // keys are, not after the process ends.
                    state.autocorrect.clear_journals();
                    state.vault.lock_for_exit();
                    state.memory.shutdown();
                    state.local_ai.shutdown();
                    // The scheduler thread is stopped before the process ends, so a
                    // timer cannot fire into a window that is already gone.
                    state.windows_actions.shutdown();
                }
            }
        });
}

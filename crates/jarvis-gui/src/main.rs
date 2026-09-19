// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use jarvis_core::{config, db, i18n, voices, SettingsManager, DB};
use tauri::Manager;

mod desktop;
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
    pub whisper: tauri_commands::WhisperHandle,
    /// The full backup and restore, one operation at a time.
    pub backup: tauri_commands::BackupHandle,
    /// Global voice input: the phrase, the handover, and the clipboard.
    pub voice_input: tauri_commands::VoiceInputHandle,
    /// The conversation route: one question at a time, and no path from an answer
    /// to a command.
    pub conversation: std::sync::Arc<tauri_commands::ConversationRuntime>,
    /// The ordered exit, built once so a normal exit, a tray exit, and a second
    /// exit request all take the same route and produce the same report.
    pub lifecycle: std::sync::Arc<jarvis_core::lifecycle::LifecycleManager>,
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

    // local dictation: a bounded child process the user supplies, and a
    // microphone that is opened only between an explicit start and stop
    let whisper = tauri_commands::WhisperHandle::restore();

    // The full backup and restore. Building it also finishes or undoes a restore
    // that a crash interrupted, before any store is opened.
    let backup = tauri_commands::BackupHandle::restore(notes.clone());

    // Global voice input: the engine is the core's, the microphone is the
    // existing session's, and the production mode puts the text on the
    // protected clipboard for the person to paste.
    let voice_input = tauri_commands::VoiceInputHandle::restore(whisper.clone());

    // The conversation route reuses that engine for the question (the same
    // microphone handover, the same Whisper) and asks the local model the settings
    // already manage for the answer — one llama-server, not a second one. Until the
    // server is configured it answers with a typed reason, and nothing about the
    // isolation changes with the provider.
    let conversation = std::sync::Arc::new(tauri_commands::ConversationRuntime::new(
        voice_input.clone(),
        local_ai.clone(),
    ));
    // The provider is the shared gateway: starting, readiness and stopping stay the
    // existing lifecycle, and the route never starts a server of its own.
    conversation.use_local_ai();

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

    // The exit is a sequence, not a handful of calls: new work stops first, then
    // what is running, then the caches and the keys, and the last step is the
    // exit itself. Each step has its own timeout and one global deadline, so a
    // component that hangs cannot keep the window open.
    let lifecycle = {
        let mut manager = jarvis_core::lifecycle::LifecycleManager::new();
        {
            let local_ai = local_ai.clone();
            manager.add(
                "cancel-generation",
                std::time::Duration::from_secs(2),
                move || {
                    local_ai.gateway().cancel();
                    Ok(())
                },
            );
        }
        {
            let whisper = whisper.clone();
            manager.add_ok(
                "stop-dictation",
                std::time::Duration::from_secs(3),
                move || {
                    whisper.shutdown();
                },
            );
        }
        {
            let windows_actions = windows_actions.clone();
            manager.add_ok(
                "stop-timers",
                std::time::Duration::from_secs(2),
                move || {
                    windows_actions.shutdown();
                },
            );
        }
        {
            let autocorrect = autocorrect.clone();
            manager.add_ok(
                "drop-decrypted-caches",
                std::time::Duration::from_secs(2),
                move || autocorrect.clear_journals(),
            );
        }
        {
            // The managed model server is stopped last among the child
            // processes, after nothing can ask it for anything.
            let local_ai = local_ai.clone();
            manager.add_ok(
                "stop-llama-server",
                std::time::Duration::from_secs(5),
                move || {
                    local_ai.shutdown();
                },
            );
        }
        {
            let memory = memory.clone();
            manager.add_ok(
                "close-databases",
                std::time::Duration::from_secs(3),
                move || {
                    memory.shutdown();
                },
            );
        }
        {
            // The write-ahead logs are truncated so the next start finds complete
            // databases. It runs after nothing is writing and before the keys are
            // dropped, and a busy checkpoint is reported rather than forced.
            let data_dir = jarvis_core::backup::BackupRoots::production()
                .map(|roots| roots.data_dir)
                .ok();
            manager.add(
                "checkpoint-databases",
                std::time::Duration::from_secs(5),
                move || {
                    let Some(directory) = data_dir.clone() else {
                        return Err("the data directory is unknown".to_string());
                    };
                    jarvis_core::backup::checkpoint_databases(&directory).map(|_| ())
                },
            );
        }
        {
            // A restore that is still moving files is stopped at its next safe
            // point; its journal makes the result recoverable either way.
            let backup = backup.clone();
            manager.add_ok(
                "stop-backup",
                std::time::Duration::from_secs(5),
                move || {
                    backup.shutdown();
                },
            );
        }
        {
            // The key session is dropped before the window is gone, and the
            // report says whether it worked.
            let vault = vault.clone();
            manager.add(
                "zeroize-keys",
                std::time::Duration::from_secs(2),
                move || {
                    vault.lock_for_exit();
                    Ok(())
                },
            );
        }
        {
            // The voice host is a child of this process: a full exit stops it,
            // and a hidden window does not.
            manager.add_ok("stop-voice-host", std::time::Duration::from_secs(3), || {
                tauri_commands::stop();
            });
        }
        manager.add_ok("exit", std::time::Duration::from_millis(200), || {});
        std::sync::Arc::new(manager)
    };

    tauri::Builder::default()
        .manage(AppState {
            settings: manager,
            notes,
            vault,
            local_ai,
            memory,
            autocorrect,
            windows_actions,
            whisper,
            backup,
            voice_input,
            conversation: std::sync::Arc::clone(&conversation),
            lifecycle: std::sync::Arc::clone(&lifecycle),
        })
        // One instance: a second launch hands the arguments to the copy that is
        // already running and shows its window, so no second scheduler, tray, or
        // database can be opened.
        .plugin(tauri_plugin_single_instance::init(|app, _arguments, _cwd| {
            desktop::show_main_window(app);
        }))
        // Autostart through the supported per-user mechanism, with the argument
        // that says "start hidden". Nothing else is passed.
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![jarvis_core::desktop::START_MINIMIZED_FLAG]),
        ))
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
            tauri_commands::check_phrase_without_running,
            tauri_commands::command_catalog,

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
            tauri_commands::windows_actions_ai_request,

            // local dictation (a Whisper build the user supplies, no network)
            tauri_commands::whisper_status,
            tauri_commands::whisper_update_settings,
            tauri_commands::whisper_select_binary,
            tauri_commands::whisper_select_model,
            tauri_commands::whisper_dictate,
            tauri_commands::whisper_transcribe_file,
            tauri_commands::whisper_cancel,
            tauri_commands::whisper_clear_last,
            tauri_commands::whisper_check_microphone,

            // backup
            tauri_commands::backup_status,
            tauri_commands::backup_export,
            tauri_commands::backup_inspect,
            tauri_commands::backup_restore,
            tauri_commands::backup_discard_previous,
            tauri_commands::backup_delete_safety,

            // global voice input
            tauri_commands::voice_input_status,
            tauri_commands::voice_input_start,
            tauri_commands::voice_input_cancel,
            tauri_commands::voice_input_update_settings,
            tauri_commands::voice_input_clear_result,
            tauri_commands::voice_input_preview,
            tauri_commands::voice_input_copy_again,

            // the conversation: one question at a time, no command reaches it
            tauri_commands::conversation_status,
            tauri_commands::conversation_ask,
            tauri_commands::conversation_cancel,
            tauri_commands::conversation_stop,
            tauri_commands::conversation_clear,
            tauri_commands::conversation_set_profile,

            // the voice host: start it once, know what it is doing
            tauri_commands::voice_host_status,
            tauri_commands::voice_host_start,
            tauri_commands::voice_host_stop,
            tauri_commands::voice_host_note_handshake,
            tauri_commands::voice_host_note_ipc_closed,
            desktop::whisper_discover,
            desktop::whisper_apply_discovered,
            // the desktop shell: state, close behaviour, autostart, first run
            desktop::desktop_get_state,
            desktop::desktop_show_window,
            desktop::desktop_hide_window,
            desktop::desktop_request_exit,
            desktop::desktop_get_close_behavior,
            desktop::desktop_set_close_behavior,
            desktop::desktop_update_settings,
            desktop::desktop_lock_storage,
            desktop::autostart_get_state,
            desktop::autostart_enable,
            desktop::autostart_disable,
            desktop::setup_get_state,
            desktop::setup_complete_step,
            desktop::setup_skip_step,
            desktop::setup_finish,
            desktop::setup_reset,
            tauri_commands::diagnostics_run,
            tauri_commands::diagnostics_preview,
            tauri_commands::diagnostics_export,
            tauri_commands::diagnostics_summary,
        ])
        .setup(|app| {
            // The shell needs the application handle for autostart and the tray,
            // so it is built here and managed as its own state.
            // The voice input notice needs the window, and the window needs
            // the handle: it is attached here, once, where both exist.
            app.state::<AppState>().voice_input.attach(app.handle().clone());
            // The conversation emits its stages and its streamed deltas to the
            // window, so it needs the handle the same way.
            app.state::<AppState>()
                .conversation
                .attach(app.handle().clone());
            let desktop = std::sync::Arc::new(desktop::DesktopHandle::new(app.handle()));
            app.manage(std::sync::Arc::clone(&desktop));
            if let Err(error) = desktop::install_tray(app.handle()) {
                // A missing tray is not fatal: the window still works, and the
                // diagnostics report says so.
                log::error!("desktop: the tray could not be created: {error}");
            }
            // The recorder is prepared here, in the one startup route, and not
            // lazily on the first read: the window's dictation command, the tray,
            // and the microphone check all find it ready, and a machine where it
            // cannot be prepared says so once at startup with its exact code.
            match desktop::ensure_recorder_ready() {
                Ok(_) => {}
                Err(code) => log::warn!(
                    "recorder: not prepared at startup (error_code={code}); the recording paths will retry"
                ),
            }
            // An autostart launch stays in the tray: the window is hidden here,
            // after the icon exists, so an invisible application cannot happen.
            let settings = desktop.settings();
            if settings.autostart_enabled
                && settings.start_minimized
                && std::env::args().any(|argument| argument == jarvis_core::desktop::START_MINIMIZED_FLAG)
            {
                desktop::hide_main_window(app.handle());
            }
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
        .on_window_event(|window, event| {
            // The close button is answered in one place, so the tray, the
            // dialog, and the setting cannot disagree.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    desktop::on_close_requested(window.app_handle(), api);
                }
            }
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
                    // One route for a normal exit, a tray exit, and a repeated
                    // request: the second call does nothing.
                    let report = state.lifecycle.shutdown();
                    if report.is_clean() {
                        log::info!("exit: {}", report.summary());
                    } else {
                        log::warn!(
                            "exit incomplete: {}",
                            report
                                .problems()
                                .iter()
                                .map(|step| step.name.clone())
                                .collect::<Vec<_>>()
                                .join(", ")
                        );
                    }
                }
            }
        });
}

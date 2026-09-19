use jarvis_core::slots;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;

// include core
use jarvis_core::{
    audio, audio_processing, commands, config, db, i18n, intent,
    ipc::{self, IpcAction},
    listener, models, recorder, stt, voices, APP_CONFIG_DIR, APP_LOG_DIR, COMMANDS_LIST, DB,
};

// include log
#[macro_use]
extern crate simple_log;
mod log;

// include app
mod app;

// where a spoken phrase went, in lines that never contain it
mod diag;

// include tray
// @TODO. macOS currently not supported for tray functionality.
#[cfg(not(target_os = "macos"))]
mod tray;

static SHOULD_STOP: AtomicBool = AtomicBool::new(false);

fn main() -> Result<(), String> {
    // initialize directories
    config::init_dirs()?;

    // initialize logging
    log::init_logging()?;

    // log some base info
    info!("Starting Jarvis v{} ...", config::APP_VERSION.unwrap());
    info!(
        "Config directory is: {}",
        APP_CONFIG_DIR.get().unwrap().display()
    );
    info!("Log directory is: {}", APP_LOG_DIR.get().unwrap().display());

    // initialize settings
    let settings = db::init();

    // set global DB (for core modules that read settings at init time)
    DB.set(settings.arc().clone())
        .expect("DB already initialized");

    // init voices
    let voice_id = settings.lock().voice.clone();
    let language = settings.lock().language.clone();
    if let Err(e) = voices::init(&voice_id, &language) {
        warn!("Failed to init voices: {}", e);
    }

    // init i18n
    i18n::init(&settings.lock().language);

    // init recorder
    if recorder::init().is_err() {
        app::close(1);
    }

    // init models registry (scans available AI models)
    if let Err(e) = models::init() {
        warn!("Models registry init failed: {}", e);
    }

    // init stt engine
    if stt::init().is_err() {
        // @TODO. Allow continuing even without STT, if commands is using keywords or smthng?
        app::close(1); // cannot continue without stt
    }

    // init commands
    info!("Initializing commands.");
    let cmds = match commands::parse_commands() {
        Ok(c) => c,
        Err(e) => {
            warn!(
                "Failed to parse commands: {}. Starting with empty command list.",
                e
            );
            Vec::new()
        }
    };
    info!(
        "Commands initialized. Count: {}, List: {:?}",
        cmds.len(),
        commands::list_paths(&cmds)
    );
    COMMANDS_LIST.set(cmds).unwrap();

    // init audio
    if audio::init().is_err() {
        // @TODO. Allow continuing even without audio?
        app::close(1); // cannot continue without audio
    }

    // init wake-word engine
    if let Err(e) = listener::init() {
        error!("Wake-word engine init failed: {}", e);
        app::close(1);
    }

    // shared async runtime for intent classification, IPC, etc.
    let rt = Arc::new(tokio::runtime::Runtime::new().expect("Failed to create tokio runtime"));

    // init intent-recognition engine
    rt.block_on(async {
        if let Err(e) = intent::init(COMMANDS_LIST.get().unwrap()).await {
            error!("Failed to initialize intent classifier: {}", e);
            app::close(1);
        }
    });

    // init slots parsing engine
    slots::init()
        .map_err(|e| error!("Slot extraction init failed: {}", e))
        .ok();

    // init audio processing
    info!("Initializing audio processing...");
    if let Err(e) = audio_processing::init() {
        warn!("Audio processing init failed: {}", e);
    }

    // init IPC
    info!("Initializing IPC...");
    ipc::init();

    // The two executors a command pack can name that are not a program of their own:
    // a typed native action, which goes through the safe Windows-action pipeline, and
    // a typed event of this application. Registering them here is what makes a
    // `native` or `internal` command in a pack runnable; without a host, both answer
    // with a typed refusal instead of guessing.
    if !jarvis_core::commands::set_native_dispatch(app::dispatch_native) {
        warn!("A native command dispatcher was already registered");
    }
    if !jarvis_core::commands::set_internal_dispatch(app::dispatch_internal) {
        warn!("An internal command dispatcher was already registered");
    }

    // channel for text commands (manually written in the GUI)
    let (text_cmd_tx, text_cmd_rx) = mpsc::channel::<String>();

    ipc::set_action_handler(move |action| {
        match action {
            IpcAction::Stop => {
                info!("Received stop command from GUI");
                SHOULD_STOP.store(true, Ordering::SeqCst);
            }
            IpcAction::ReloadCommands => {
                info!("Received reload commands request");
                // TODO: implement reload
            }
            IpcAction::SetMuted { muted } => {
                // The pause the typed `stop_listening` event sets, and the resume the
                // window asks for. The device stays open either way.
                app::set_listener_paused(muted);
                if !muted {
                    // The window has finished its dictation/conversation. The
                    // reader loop reopens the recorder itself before reading.
                    app::finish_microphone_handoff();
                }
                info!("Listening paused: {}", muted);
                if !muted {
                    ipc::send(jarvis_core::ipc::IpcEvent::Listening);
                }
            }
            IpcAction::BeginMicrophoneHandoff => {
                app::begin_microphone_handoff();
            }
            IpcAction::FinishMicrophoneHandoff => {
                app::finish_microphone_handoff();
            }
            IpcAction::TextCommand { text } => {
                info!("Received text command (length={})", text.chars().count());
                if let Err(e) = text_cmd_tx.send(text) {
                    error!("Failed to send text command to app: {}", e);
                }
            }
            IpcAction::Ping => {
                // handled internally by server
            }
            _ => {}
        }
    });

    // start WebSocket server on the shared runtime
    let ipc_rt = Arc::clone(&rt);
    std::thread::spawn(move || {
        ipc_rt.block_on(ipc::start_server());
    });

    // start the app (in the background thread)
    let app_rt = Arc::clone(&rt);
    std::thread::spawn(move || {
        let _ = app::start(text_cmd_rx, &app_rt);
    });

    tray::init_blocking(settings);

    Ok(())
}

pub fn should_stop() -> bool {
    SHOULD_STOP.load(Ordering::SeqCst)
}

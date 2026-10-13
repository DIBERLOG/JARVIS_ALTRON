use jarvis_core::slots;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

// include core
use jarvis_core::{
    audio, audio_processing, commands, config, db, listener, recorder, stt, intent,
    ipc::{self, IpcAction},
    i18n, voices, models,
    APP_CONFIG_DIR, APP_LOG_DIR, APP_DIR, COMMANDS_LIST, DB,
};

// include log
#[macro_use]
extern crate simple_log;
mod log;

// include app
mod app;


static SHOULD_STOP: AtomicBool = AtomicBool::new(false);
static MICROPHONE_MUTED: AtomicBool = AtomicBool::new(false);

fn microphone_muted() -> bool {
    MICROPHONE_MUTED.load(Ordering::SeqCst)
}

fn main() -> Result<(), String> {
    if std::env::args().any(|arg| arg == "--release-smoke-test") {
        if !APP_DIR.join("resources/commands").is_dir() || models::vosk_models::scan_vosk_models().is_empty() {
            return Err("Release resources or Vosk models are missing".into());
        }
        println!("JARVIS core {}: packaged resources OK", config::APP_VERSION.unwrap_or("unknown"));
        return Ok(());
    }
    // initialize directories
    config::init_dirs()?;

    // initialize logging
    log::init_logging()?;

    // log some base info
    info!("Starting Jarvis v{} ...", config::APP_VERSION.unwrap());
    info!("ALTRON kernel {} active: catalog, routing, action planning, settings and audio history", commands::KERNEL_VERSION);
    info!("Config directory is: {}", APP_CONFIG_DIR.get().unwrap().display());
    info!("Log directory is: {}", APP_LOG_DIR.get().unwrap().display());

    // initialize settings
    let settings = db::init();
    MICROPHONE_MUTED.store(settings.lock().microphone_muted, Ordering::SeqCst);

    // set global DB (for core modules that read settings at init time)
    DB.set(settings.arc().clone())
            .expect("DB already initialized");
    jarvis_core::tts::prewarm_silero();

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
            warn!("Failed to parse commands: {}. Starting with empty command list.", e);
            Vec::new()
        }
    };
    info!("Commands initialized. Count: {}, List: {:?}", cmds.len(), commands::list_paths(&cmds));
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
    let rt = Arc::new(
        tokio::runtime::Runtime::new().expect("Failed to create tokio runtime")
    );

    // init intent-recognition engine
    rt.block_on(async {
        if let Err(e) = intent::init(COMMANDS_LIST.get().unwrap()).await {
            error!("Failed to initialize intent classifier: {}", e);
            app::close(1);
        }
    });

    // init slots parsing engine
    slots::init().map_err(|e| error!("Slot extraction init failed: {}", e)).ok();

    // init audio processing
    info!("Initializing audio processing...");
    if let Err(e) = audio_processing::init() {
        warn!("Audio processing init failed: {}", e);
    }

    // init IPC
    info!("Initializing IPC...");
    ipc::init();

    // channel for text commands (manually written in the GUI)
    let (text_cmd_tx, text_cmd_rx) = mpsc::channel::<app::IncomingCommand>();

    let ipc_settings = settings.clone();
    ipc::set_action_handler(move |action| {
        match action {
            IpcAction::Stop => {
                info!("Received stop command from GUI");
                SHOULD_STOP.store(true, Ordering::SeqCst);
            }
            IpcAction::ReloadCommands => {
                match commands::parse_commands() {
                    Ok(packs) => info!("Reloaded {} command packs; new phrases apply on the next command", packs.len()),
                    Err(error) => warn!("Could not reload commands: {error}"),
                }
            }
            IpcAction::SetMuted { muted } => {
                info!("Received mute request: {}", muted);
                MICROPHONE_MUTED.store(muted, Ordering::SeqCst);
                if let Err(error) = ipc_settings.write("microphone_muted", if muted { "true" } else { "false" }) {
                    warn!("Could not save microphone state: {error}");
                }
            }
            IpcAction::GetMuted => {
                ipc::send(jarvis_core::ipc::IpcEvent::MicrophoneMuted { muted: microphone_muted() });
                ipc::send(jarvis_core::ipc::IpcEvent::DialogueMode { active: app::dialogue_active() });
            }
            IpcAction::TextCommand { text } => {
                info!("Received authenticated text command");
                if let Err(e) = text_cmd_tx.send(app::IncomingCommand::Text(text)) {
                    error!("Failed to send text command to app: {}", e);
                }
            }
            IpcAction::RemoteCommand { id, request_id, expires_at } => {
                if jarvis_core::commands::remote_allowed(&id) {let _=text_cmd_tx.send(app::IncomingCommand::Remote{id,request_id,expires_at});}
            }
            IpcAction::Ping => {
                // handled internally by server
            }
            IpcAction::CenterReply { text, reply_id, follow_up } => {
                std::thread::spawn(move || app::center_reply(text, reply_id, follow_up));
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

    // This sidetone is separate from Voicemod's monitoring button. The
    // physical microphone is never mixed into Jarvis's VB-CABLE output.
    std::thread::spawn(monitor_self_loop);

    // The tray shell is intentionally disabled on this Windows build: its menu
    // dependency imports TaskDialogIndirect, which is unavailable on this PC.
    // The GUI remains the control surface and the assistant threads stay alive.
    loop {
        if SHOULD_STOP.load(Ordering::SeqCst) { break; }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    Ok(())
}

pub fn should_stop() -> bool {
    SHOULD_STOP.load(Ordering::SeqCst)
}

fn monitor_self_loop() {
    let python = APP_DIR.parent().and_then(|target| target.parent())
        .map(|root| root.join("tools/voice_training/.venv/Scripts/pythonw.exe"));
    let script = APP_DIR.join("resources/tts/MicMonitor.py");
    let mut child: Option<Child> = None;
    while !should_stop() {
        let enabled = !microphone_muted()
            && db::latest_settings().is_some_and(|settings| settings.monitor_self);
        if let Some(process) = child.as_mut() {
            if !enabled || process.try_wait().ok().flatten().is_some() {
                let _ = process.kill();
                let _ = process.wait();
                child = None;
            }
        }
        if enabled && child.is_none() {
            if let Some(python) = python.as_ref().filter(|path| path.is_file()) {
                if script.is_file() {
                    match Command::new(python).arg(&script)
                        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
                        Ok(process) => child = Some(process),
                        Err(error) => warn!("Microphone monitoring failed: {error}"),
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    if let Some(mut process) = child {
        let _ = process.kill();
        let _ = process.wait();
    }
}

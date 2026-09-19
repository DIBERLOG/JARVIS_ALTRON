use std::sync::mpsc::Receiver;
use std::time::Instant;
use std::time::SystemTime;

use jarvis_core::safety::{ConfirmationResult, GateDecision, SafetyGate};
use jarvis_core::windows_actions::{
    platform_backend, ActionRequestOutcome, ActionValue, VoiceRoute, WindowsActionSettings,
    WindowsActions, VOICE_CANCEL_WORDS, VOICE_CONFIRM_WORDS,
};
use jarvis_core::{
    audio_buffer::AudioRingBuffer,
    audio_processing, commands, config, i18n, intent,
    ipc::{self, IpcEvent},
    listener, recorder, slots, stt, voices, COMMANDS_LIST,
};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use rand::seq::SliceRandom;

use crate::{diag, should_stop};

static SAFETY_GATE: Lazy<Mutex<SafetyGate>> = Lazy::new(|| Mutex::new(SafetyGate::default()));

/// The safe Windows actions, opened once for the voice host.
///
/// It is the same pipeline the interface uses: the spoken phrase becomes a typed action, the
/// central policy decides what happens to it, and a risky one waits for a spoken "подтверждаю"
/// until it expires. Nothing here reads a command out of the transcript.
static WINDOWS_ACTIONS: Lazy<Option<Mutex<WindowsActions>>> = Lazy::new(|| {
    let directory = jarvis_core::notes::vault::VaultPaths::production()
        .map(|paths| paths.data_dir)
        .unwrap_or_else(|_| std::path::PathBuf::from("."));
    let settings = WindowsActions::stored_settings(&directory)
        .unwrap_or_else(|| WindowsActionSettings::default_for(&directory));
    let session = WindowsActions::open(&directory, platform_backend(), settings);
    // Without the allowlist lookup the executor can start nothing, which is the safe default.
    session.install_launch_lookup();
    Some(Mutex::new(session))
});

// VAD state machine
#[derive(Debug, Clone, Copy, PartialEq)]
enum VadState {
    WaitingForVoice,
    VoiceActive,
}

pub fn start(text_cmd_rx: Receiver<String>, rt: &tokio::runtime::Runtime) -> Result<(), ()> {
    main_loop(text_cmd_rx, rt)
}

fn main_loop(text_cmd_rx: Receiver<String>, rt: &tokio::runtime::Runtime) -> Result<(), ()> {
    let frame_length: usize = 512;
    let sample_rate: usize = 16000;
    let mut frame_buffer: Vec<i16> = vec![0; frame_length];

    // ring buffer: keeps last 5 seconds of audio (pre-roll)
    let mut audio_buffer = AudioRingBuffer::new(5.0, frame_length, sample_rate);

    // VAD state
    let mut vad_state = VadState::WaitingForVoice;
    let mut silence_frames: u32 = 0;

    // how many frames of silence before we consider speech ended
    // 1.5 seconds = 1.5 * (16000 / 512) ≈ 47 frames
    let silence_threshold: u32 = ((1.5 * sample_rate as f32) / frame_length as f32) as u32;

    voices::play_greet();

    match recorder::start_recording() {
        Ok(_) => info!(
            "Recording started. Microphone: {}",
            recorder::get_audio_device_name(recorder::get_selected_microphone_index())
        ),
        Err(_) => {
            error!("Cannot start recording.");
            return Err(());
        }
    }

    ipc::send(IpcEvent::Idle);

    // ### WAKE WORD DETECTION LOOP
    'wake_word: loop {
        if should_stop() {
            info!("Stop signal received, shutting down...");
            voices::play_goodbye();
            ipc::send(IpcEvent::Stopping);
            break;
        }

        if let Ok(text) = text_cmd_rx.try_recv() {
            process_text_command(&text, &rt);
            continue 'wake_word;
        }

        recorder::read_microphone(&mut frame_buffer);
        let processed = audio_processing::process(&frame_buffer);

        match vad_state {
            VadState::WaitingForVoice => {
                // always buffer audio
                audio_buffer.push(&frame_buffer);

                if processed.is_voice {
                    // voice started! flush buffer to Vosk
                    info!(
                        "VAD: Voice started, flushing {} buffered frames",
                        audio_buffer.len()
                    );

                    for buffered_frame in audio_buffer.drain_all() {
                        listener::data_callback(&buffered_frame);
                    }

                    vad_state = VadState::VoiceActive;
                    silence_frames = 0;
                }
            }

            VadState::VoiceActive => {
                // dual-feed: speech recognizer gets frames in parallel with wake word detector
                let _ = stt::recognize(&frame_buffer, false);

                // feed to wake word detector
                if let Some(_keyword_index) = listener::data_callback(&frame_buffer) {
                    // WAKE WORD DETECTED!
                    info!("Wake word activated!");
                    ipc::send(IpcEvent::WakeWordDetected);

                    stt::reset_wake_recognizer();
                    audio_processing::reset();

                    // brief sniff to keep feeding STT while transitioning
                    let sniff_frames = ((0.3 * sample_rate as f32) / frame_length as f32) as u32;
                    for _ in 0..sniff_frames {
                        recorder::read_microphone(&mut frame_buffer);
                        audio_processing::process(&frame_buffer);
                        stt::recognize(&frame_buffer, false);
                    }

                    ipc::send(IpcEvent::Listening);
                    recognize_command(&mut frame_buffer, &rt, frame_length, sample_rate, true);

                    // reset state after command
                    vad_state = VadState::WaitingForVoice;
                    silence_frames = 0;
                    audio_buffer.clear();
                    stt::reset_wake_recognizer();
                    stt::reset_speech_recognizer(); // NOW reset, after command is done
                    audio_processing::reset();
                    ipc::send(IpcEvent::Idle);

                    continue 'wake_word;
                }

                // track silence
                if processed.is_voice {
                    silence_frames = 0;
                } else {
                    silence_frames += 1;

                    if silence_frames > silence_threshold {
                        debug!("VAD: Silence timeout, returning to wait state");
                        vad_state = VadState::WaitingForVoice;
                        silence_frames = 0;
                        stt::reset_wake_recognizer();
                        stt::reset_speech_recognizer(); // reset since we were dual-feeding
                    }
                }
            }
        }
    }

    recorder::stop_recording().ok();
    ipc::send(IpcEvent::Stopping);

    Ok(())
}

// Voice recognition for command after wake word
fn recognize_command(
    frame_buffer: &mut [i16],
    rt: &tokio::runtime::Runtime,
    frame_length: usize,
    sample_rate: usize,
    prefed_audio: bool,
) {
    let mut audio_buffer = AudioRingBuffer::new(2.0, frame_length, sample_rate);
    let mut vad_state = if prefed_audio {
        VadState::VoiceActive
    } else {
        VadState::WaitingForVoice
    };
    let mut silence_frames: u32 = 0;
    let mut start = SystemTime::now();
    let mut first_recognition = prefed_audio;

    // longer silence threshold for commands (user might pause to think)
    // 5 seconds
    let silence_threshold: u32 = ((5.0 * sample_rate as f32) / frame_length as f32) as u32;

    loop {
        if crate::should_stop() {
            return;
        }

        recorder::read_microphone(frame_buffer);
        let processed = audio_processing::process(frame_buffer);

        match vad_state {
            VadState::WaitingForVoice => {
                audio_buffer.push(frame_buffer);

                if processed.is_voice {
                    // flush buffer to STT
                    for buffered_frame in audio_buffer.drain_all() {
                        stt::recognize(&buffered_frame, false);
                    }
                    vad_state = VadState::VoiceActive;
                    silence_frames = 0;
                } else {
                    silence_frames += 1;

                    if silence_frames > silence_threshold {
                        info!("Long silence detected, returning to wake word mode.");
                        return;
                    }
                }
            }

            VadState::VoiceActive => {
                // feed to STT
                if let Some(mut recognized_voice) = stt::recognize(frame_buffer, false) {
                    // The transcript is a person's speech. It is not written to the
                    // log: what is written is its length, on the diagnostic stage.
                    diag::received(recognized_voice.chars().count());

                    ipc::send(IpcEvent::SpeechRecognized {
                        text: recognized_voice.clone(),
                    });

                    recognized_voice = recognized_voice.to_lowercase();

                    // Global voice input: the phrase is an intent, not a
                    // command. It is matched here, where the listener lives,
                    // because the microphone has to change hands *here*: this
                    // process stops listening, and only then does the window
                    // process start the dictation. The recognised text is never
                    // sent anywhere — the event carries no transcript.
                    if jarvis_core::dictation::is_start_request(&recognized_voice) {
                        info!("Global voice input requested");
                        // The listener lets go of the microphone first.
                        jarvis_core::recorder::stop_recording().ok();
                        stt::reset_speech_recognizer();
                        ipc::send(IpcEvent::GlobalDictationRequested);
                        vad_state = VadState::WaitingForVoice;
                        silence_frames = 0;
                        audio_buffer.clear();
                        continue;
                    }

                    // check if wake word repeated (reactivate)
                    let wake_phrases = config::get_wake_phrases(&i18n::get_language());
                    let contains_wake = wake_phrases.iter().any(|wp| recognized_voice.contains(wp));
                    diag::wake_word(contains_wake, recognized_voice.chars().count());

                    if contains_wake {
                        // strip the wake word
                        let mut remaining = recognized_voice.clone();
                        for wp in wake_phrases {
                            remaining = remaining.replace(wp, "");
                        }
                        let remaining = remaining.trim();

                        if remaining.is_empty() {
                            if first_recognition {
                                // leftover wake word from dual-feed, just discard it
                                info!("Discarding initial wake word from prefed audio");
                                first_recognition = false;
                                stt::reset_speech_recognizer();
                                voices::play_reply();
                                vad_state = VadState::WaitingForVoice;
                                silence_frames = 0;
                                start = SystemTime::now();
                                audio_buffer.clear();
                                continue;
                            }

                            // just wake word, no command - reactivate
                            info!("Wake word repeated during chaining, reactivating...");
                            voices::play_reply();
                            stt::reset_speech_recognizer();
                            ipc::send(IpcEvent::Listening);

                            vad_state = VadState::WaitingForVoice;
                            silence_frames = 0;
                            start = SystemTime::now();
                            audio_buffer.clear();
                            continue;
                        } else {
                            // wake word + command in one phrase - execute the command part
                            info!(
                                "Wake word and command in one phrase (length={})",
                                remaining.chars().count()
                            );
                            recognized_voice = remaining.to_string();
                            // fall through to command execution below
                        }
                    }

                    first_recognition = false;

                    // filter activation phrases
                    // for tbr in config::ASSISTANT_PHRASES_TBR {
                    //     recognized_voice = recognized_voice.replace(tbr, "");
                    // }
                    for tbr in config::get_phrases_to_remove(&i18n::get_language()) {
                        recognized_voice = recognized_voice.replace(tbr, "");
                    }

                    recognized_voice = recognized_voice.trim().to_string();

                    // The order matters: nothing left after the wake word and the
                    // filler words is a different answer from a short phrase, and the
                    // diagnostic says which of the two happened.
                    if recognized_voice.is_empty() {
                        diag::rejection("empty_after_strip", 0);
                        continue;
                    }

                    if recognized_voice.chars().count() < 5 {
                        diag::rejection("too_short", recognized_voice.chars().count());
                        continue;
                    }

                    // execute command and check if we should chain
                    let should_chain = execute_command(&recognized_voice, rt);

                    if should_chain {
                        // chain: reset and continue listening
                        info!("Chaining enabled, continuing to listen...");
                        stt::reset_speech_recognizer();
                        vad_state = VadState::WaitingForVoice;
                        silence_frames = 0;
                        start = SystemTime::now();
                        audio_buffer.clear();
                        ipc::send(IpcEvent::Listening);
                        continue;
                    } else {
                        // no chain: return to wake word
                        info!("No chain, returning to wake word mode.");
                        return;
                    }
                }

                // track silence
                if processed.is_voice {
                    silence_frames = 0;
                } else {
                    silence_frames += 1;

                    if silence_frames > silence_threshold {
                        info!("Long silence detected, returning to wake word mode.");
                        return;
                    }
                }
            }
        }

        // timeout
        if let Ok(elapsed) = start.elapsed() {
            if elapsed > config::CMS_WAIT_DELAY {
                info!("Command timeout, returning to wake word mode.");
                return;
            }
        }
    }
}

fn process_text_command(text: &str, rt: &tokio::runtime::Runtime) {
    info!("Processing text command (length={})", text.chars().count());

    ipc::send(IpcEvent::SpeechRecognized {
        text: text.to_string(),
    });

    let mut filtered = text.to_lowercase();
    // for tbr in config::ASSISTANT_PHRASES_TBR {
    //     filtered = filtered.replace(tbr, "");
    // }
    for tbr in config::get_phrases_to_remove(&i18n::get_language()) {
        filtered = filtered.replace(tbr, "");
    }

    let filtered = filtered.trim();

    if filtered.is_empty() {
        ipc::send(IpcEvent::Idle);
        return;
    }

    // text commands never chain
    execute_command(filtered, rt);
}

// Execute command, returns true if chaining should continue
fn execute_command(text: &str, rt: &tokio::runtime::Runtime) -> bool {
    // One normalizer, the same one the settings page uses when it is asked what a
    // phrase would do: lower case, `ё` folded, punctuation dropped, the wake word
    // and the comma after it removed. It is used for *matching only*. The phrase
    // itself is what goes to the executor, so a slot value keeps the letters the
    // person said instead of the ones the matcher compared.
    let normalized = commands::normalize_phrase(text);
    diag::normalized(normalized.chars().count());
    if normalized.is_empty() {
        diag::rejection("empty_after_strip", 0);
        ipc::send(IpcEvent::Idle);
        return false;
    }

    // The safe actions come first: a phrase they understand is theirs, and a phrase they do not
    // understand falls through to the configured commands, which are unchanged.
    let mut unclear: Option<&'static str> = None;
    if let Some(handled) = try_windows_action(&normalized, &mut unclear) {
        return handled;
    }

    let commands_list = match COMMANDS_LIST.get() {
        Some(c) => c,
        None => {
            diag::rejection("no_commands", normalized.chars().count());
            ipc::send(IpcEvent::Error {
                message: "Commands not loaded".to_string(),
            });
            ipc::send(IpcEvent::Idle);
            return false;
        }
    };

    match normalized.as_str() {
        "отмена" | "cancel" => {
            let message = if matches!(SAFETY_GATE.lock().cancel(), ConfirmationResult::Cancelled) {
                "Действие отменено"
            } else {
                "Нет действия для отмены"
            };
            ipc::send(IpcEvent::Error {
                message: message.into(),
            });
            ipc::send(IpcEvent::Idle);
            return false;
        }
        "подтверждаю" | "подтверждаю действие" | "confirm" => {
            let result = SAFETY_GATE.lock().confirm(Instant::now());
            if let ConfirmationResult::Confirmed { command_id } = result {
                if let Some((path, command)) = commands_list.iter().find_map(|list| {
                    list.commands
                        .iter()
                        .find(|command| command.id == command_id)
                        .map(|command| (&list.path, command))
                }) {
                    return execute_resolved_command(path, command, text, None);
                }
                ipc::send(IpcEvent::Error {
                    message: "Подтверждённая команда больше недоступна".into(),
                });
            } else {
                let message = if matches!(result, ConfirmationResult::Expired) {
                    "Время подтверждения истекло"
                } else {
                    "Нет действия для подтверждения"
                };
                ipc::send(IpcEvent::Error {
                    message: message.into(),
                });
            }
            ipc::send(IpcEvent::Idle);
            return false;
        }
        _ => {}
    }

    // The AI intent is asked first, and the phrase matcher is the answer when the
    // AI has none — *and* when the AI names a command that is not in the loaded
    // packs. That second case used to end the search: a confident intent whose id
    // no longer exists (a stale training cache, a pack that was edited) left the
    // deterministic matcher unasked, and a phrase the packs understand was
    // reported as not found. A named command that cannot be resolved is not an
    // answer.
    let normalized_length = normalized.chars().count();
    let from_intent = match rt.block_on(intent::classify(&normalized)) {
        Some((intent_id, confidence)) => {
            info!(
                "Intent recognized: {} (confidence: {:.2})",
                intent_id, confidence
            );
            match intent::get_command_by_intent(commands_list, &intent_id) {
                Some(found) => Some(found),
                None => {
                    info!(
                        "Intent '{}' does not name a loaded command, using the phrase matcher",
                        intent_id
                    );
                    None
                }
            }
        }
        None => {
            info!("Intent not recognized, using the phrase matcher");
            None
        }
    };
    let cmd_result = match from_intent {
        Some(found) => Some(found),
        None => commands::fetch_command(&normalized, commands_list),
    };

    if let Some((cmd_path, cmd_config)) = cmd_result {
        diag::matched(&cmd_config.id, normalized_length);
        match SAFETY_GATE
            .lock()
            .request(&cmd_config.id, cmd_config.risk_level, Instant::now())
        {
            GateDecision::Approved => {}
            GateDecision::AwaitingConfirmation { .. } => {
                diag::rejection("awaiting_confirmation", normalized_length);
                ipc::send(IpcEvent::Error { message: "Это действие требует подтверждения. Скажите «подтверждаю» в течение 15 секунд или «отмена».".into() });
                ipc::send(IpcEvent::Idle);
                return false;
            }
            GateDecision::RejectedForbidden => {
                diag::rejection("forbidden", normalized_length);
                ipc::send(IpcEvent::Error {
                    message: "Это действие запрещено политикой безопасности".into(),
                });
                ipc::send(IpcEvent::Idle);
                return false;
            }
        }

        // extract slots if needed
        let extracted_slots = if !cmd_config.slots.is_empty() {
            let s = slots::extract(text, &cmd_config.slots);
            if !s.is_empty() {
                info!("Extracted {} slot(s)", s.len());
            }
            Some(s)
        } else {
            None
        };

        diag::started(&cmd_config.id, normalized_length);
        return execute_resolved_command(&cmd_path, &cmd_config, text, extracted_slots.as_ref());
    } else {
        diag::rejection(unclear.unwrap_or("no_match"), normalized_length);
        voices::play_not_found();
        ipc::send(IpcEvent::Error {
            message: "Command not found".to_string(),
        });
    }
    ipc::send(IpcEvent::Idle);
    false
}

/// Routes one phrase through the safe actions.
///
/// Returns `Some(false)` when the phrase belonged to this feature — whether it ran or is waiting
/// for a confirmation — and `None` when it did not, so the caller can try the configured
/// commands.
///
/// An *unclear* answer also returns `None`, and that is deliberate. It used to be reported and
/// returned straight away, which meant the safe-action router answered for phrases it had not
/// acted on: "открой браузер" was recognised as a launch, found no allowed application, and the
/// configured `browser_open` command was never consulted — the ordinary commands looked broken
/// while the router was only undecided. Nothing is executed on this path, so falling through
/// costs nothing and cannot run the wrong thing: the reason is kept in `unclear` and is reported
/// only if the configured commands have no answer either. A phrase the router *did* act on, or
/// refused, or that answers a pending confirmation, is still final.
fn try_windows_action(text: &str, unclear: &mut Option<&'static str>) -> Option<bool> {
    let session = WINDOWS_ACTIONS.as_ref()?;
    let lowered = text.trim().to_lowercase();

    // A confirmation or a refusal answers whatever is waiting, and is handled here first: the
    // words are the same ones the interface's dialog uses.
    if VOICE_CONFIRM_WORDS
        .iter()
        .any(|word| lowered == *word || lowered.contains(word))
    {
        let mut session = session.lock();
        if !session.has_pending() {
            return None;
        }
        let message = match session.confirm_pending() {
            Ok(result) => announce(&result.value),
            Err(error) => error_message(&error.to_string()),
        };
        voices::play_ok();
        ipc::send(IpcEvent::Error { message });
        ipc::send(IpcEvent::Idle);
        return Some(false);
    }
    if VOICE_CANCEL_WORDS.iter().any(|word| lowered == *word) {
        let mut session = session.lock();
        if !session.has_pending() {
            return None;
        }
        session.cancel();
        voices::play_ok();
        ipc::send(IpcEvent::Error {
            message: "Действие отменено".to_string(),
        });
        ipc::send(IpcEvent::Idle);
        return Some(false);
    }

    match session.lock().route_voice(text) {
        Ok(VoiceRoute::Requested { outcome }) => {
            let message = match &*outcome {
                ActionRequestOutcome::Executed { result } => announce(&result.value),
                ActionRequestOutcome::AwaitingConfirmation { preview } => format!(
                    "Это действие требует подтверждения ({}). Скажите «подтверждаю» в течение {} секунд или «отмена».",
                    preview.action_kind, preview.expires_in_seconds
                ),
                ActionRequestOutcome::Rejected { detail } => error_message(detail),
            };
            voices::play_ok();
            ipc::send(IpcEvent::Error { message });
            ipc::send(IpcEvent::Idle);
            Some(false)
        }
        // The phrase was an action the router could not decide. Nothing ran, so the
        // configured commands get their turn; the reason waits in `unclear` in case
        // they have no answer either.
        Ok(VoiceRoute::Ambiguous { reason }) => {
            *unclear = Some(match reason.as_str() {
                "windows-voice-application-not-allowed" => "action_application_not_allowed",
                "windows-voice-application-ambiguous" => "action_application_ambiguous",
                "windows-voice-launch-unspecified" => "action_launch_unspecified",
                "windows-voice-window-not-found" => "action_window_not_found",
                "windows-voice-window-ambiguous" => "action_window_ambiguous",
                "windows-voice-window-unspecified" => "action_window_unspecified",
                "windows-voice-move-unclear" => "action_move_unclear",
                "windows-voice-volume-unclear" => "action_volume_unclear",
                "windows-voice-no-foreground-window" => "action_no_foreground_window",
                _ => "action_unclear",
            });
            None
        }
        Ok(VoiceRoute::NotAnAction) | Ok(VoiceRoute::Disabled) | Err(_) => None,
    }
}

/// One line about a finished action, without ever repeating a path or a stored text.
fn announce(value: &ActionValue) -> String {
    match value {
        ActionValue::Volume { percent, muted } => {
            if *muted {
                format!("Громкость: {percent}%, звук выключен")
            } else {
                format!("Громкость: {percent}%")
            }
        }
        ActionValue::ScreenshotPath { .. } => "Снимок экрана записан в выбранную папку".to_string(),
        ActionValue::Locked => "Сеанс заблокирован".to_string(),
        ActionValue::Timer { .. } => "Таймер запущен".to_string(),
        ActionValue::Cancelled { .. } => "Таймер отменён".to_string(),
        ActionValue::Windows { windows } => format!("Найдено окон: {}", windows.len()),
        ActionValue::Launched { .. } => "Программа запущена".to_string(),
        ActionValue::None => "Готово".to_string(),
    }
}

/// A refusal, said in one short line. The core's errors carry no payload by construction.
fn error_message(code: &str) -> String {
    match code {
        "sensitive_window" => {
            "В фокусе окно, где могут быть пароли, поэтому снимок отклонён".to_string()
        }
        "confirmation_expired" => "Время подтверждения истекло".to_string(),
        "application_not_allowed" => "Эта программа не разрешена".to_string(),
        "window_expired" => "Список окон устарел, повторите".to_string(),
        other => format!("Действие не выполнено ({other})"),
    }
}

fn execute_resolved_command(
    cmd_path: &std::path::PathBuf,
    cmd_config: &jarvis_core::commands::JCommand,
    text: &str,
    slots: Option<&std::collections::HashMap<String, jarvis_core::commands::SlotValue>>,
) -> bool {
    match commands::execute_command(cmd_path, cmd_config, Some(text), slots) {
        Ok(chain) => {
            info!("Command executed successfully");
            diag::finished(&cmd_config.id, true, None, text.chars().count());
            // voices::play_ok();
            voices::play_random_from(cmd_config.get_sounds(&i18n::get_language()).as_slice());
            ipc::send(IpcEvent::CommandExecuted {
                id: cmd_config.id.clone(),
                success: true,
            });
            ipc::send(IpcEvent::Idle);
            return chain; // return chain status from command
        }
        Err(msg) => {
            error!("Error executing command: {}", msg);
            diag::finished(
                &cmd_config.id,
                false,
                Some("execution_failed"),
                text.chars().count(),
            );
            voices::play_error();
            ipc::send(IpcEvent::CommandExecuted {
                id: cmd_config.id.clone(),
                success: false,
            });
            ipc::send(IpcEvent::Error {
                message: msg.to_string(),
            });
        }
    }
    ipc::send(IpcEvent::Idle);
    false // no chain on error or not found
}

pub fn close(code: i32) {
    info!("Closing application.");
    voices::play_goodbye();
    ipc::send(IpcEvent::Stopping);
    std::process::exit(code);
}

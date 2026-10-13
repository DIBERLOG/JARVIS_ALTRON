use std::sync::mpsc::{self, Receiver, SyncSender};
use std::time::SystemTime;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use jarvis_core::{audio_buffer::AudioRingBuffer, audio_processing, commands, config, listener, recorder, stt, COMMANDS_LIST, intent, voices, ipc::{self, IpcEvent}, i18n, slots, tts};
use rand::seq::SliceRandom;

use crate::{microphone_muted, should_stop};

static DIALOGUE_MODE: AtomicBool = AtomicBool::new(false);

pub fn dialogue_active() -> bool { DIALOGUE_MODE.load(Ordering::SeqCst) }
static CHAIN_LISTENING: AtomicBool = AtomicBool::new(false);
static LAST_COMMAND: Mutex<Option<String>> = Mutex::new(None);
static DIALOGUE_HISTORY: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

#[derive(Clone)]
pub enum IncomingCommand { Text(String), Remote { id: String, request_id: String, expires_at: i64 } }
fn execute_remote(id: &str, request_id: &str, expires_at: i64) {
    let status=(|| -> Result<&str, String> {
        if SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_secs() as i64>expires_at || !commands::remote_allowed(id) {return Err("Команда недоступна или истекла".into());}
        let packs=commands::parse_commands()?;
        let center=commands::center::available_commands();
        let command=packs.iter().flat_map(|p|p.commands.iter()).chain(center.iter()).find(|c|c.id==id).cloned();
        let command=command.ok_or("Команда отсутствует на этом ПК")?;
        let phrase=command.get_phrases("ru").first().cloned().ok_or("У команды нет фразы")?;
        ipc::send(IpcEvent::RevealWindow);
        ipc::send(IpcEvent::SpeechRecognized {text:phrase.clone()});
        if command.cmd_type=="center" {ipc::send(IpcEvent::RemoteCenterCommand {id:id.into(),request_id:request_id.into()});return Ok("center_pending");}
        let pack=packs.iter().find(|p|p.commands.iter().any(|c|c.id==id)).ok_or("Нет каталога команды")?;
        commands::execute_command(&pack.path,&command,Some(&phrase),None)?;
        ipc::send(IpcEvent::CommandExecuted {id:id.into(),success:true});
        Ok("executed")
    })().unwrap_or("failed");
    if status!="center_pending" {ipc::send(IpcEvent::RemoteCommandResult {request_id:request_id.into(),status:status.into()});}
    ipc::send(IpcEvent::Idle);
}

// Execute commands serially off the capture thread. Slow Lua, network requests,
// model replies and restart warning clips must not suspend wake-word detection.
struct CommandDispatcher {
    sender: SyncSender<IncomingCommand>,
    pending: Arc<AtomicUsize>,
}

impl CommandDispatcher {
    fn start(mut execute: impl FnMut(&str) -> bool + Send + 'static) -> Self {
        let (sender, receiver) = mpsc::sync_channel::<IncomingCommand>(8);
        let pending = Arc::new(AtomicUsize::new(0));
        let worker_pending = pending.clone();
        std::thread::spawn(move || {
            for text in receiver {
                if should_stop() { worker_pending.fetch_sub(1, Ordering::SeqCst); continue; }
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match &text { IncomingCommand::Text(text)=>execute(text), IncomingCommand::Remote{id,request_id,expires_at}=>{execute_remote(id,request_id,*expires_at);false} }));
                CHAIN_LISTENING.store(result.unwrap_or_else(|_| {
                    ipc::send(IpcEvent::Error { message: "Ошибка выполнения команды".to_string() });
                    false
                }), Ordering::SeqCst);
                worker_pending.fetch_sub(1, Ordering::SeqCst);
            }
        });
        Self { sender, pending }
    }

    fn submit(&self, text: &str) -> bool {
        self.pending.fetch_add(1, Ordering::SeqCst);
        if self.sender.try_send(IncomingCommand::Text(text.to_string())).is_err() {
            self.pending.fetch_sub(1, Ordering::SeqCst);
            ipc::send(IpcEvent::Error { message: "Очередь команд занята. Попробуйте через несколько секунд.".to_string() });
            return false;
        }
        true
    }

    fn busy(&self) -> bool { self.pending.load(Ordering::SeqCst) > 0 }
}

fn dialogue_start_phrase(text: &str) -> bool {
    matches!(dialogue_phrase(text), "давай пообщаемся" | "давай поговорим" | "включи диалоговый режим" |
        "поговорим" | "пообщаемся" | "начни разговор" | "включи разговор" | "разговорный режим" |
        "включи разговорный режим" | "хочу поговорить" | "давай поболтаем" | "поболтаем" | "включи диалог")
}

fn dialogue_command_phrase(id: &str) -> Option<&'static str> {
    match id { "dialogue_start" => Some("давай поговорим"), "dialogue_stop" => Some("закончи разговор"), _ => None }
}

fn dialogue_stop_phrase(text: &str) -> bool {
    matches!(dialogue_phrase(text), "закончи разговор" | "закончим разговор" |
        "хватит общаться" | "выключи диалоговый режим" |
        "закрой диалог" | "режим команд" | "отмени разговор" | "отмена разговора" |
        "хватит разговаривать" | "заверши разговор" | "останови разговор" | "выключи разговор" |
        "выключи разговорный режим" | "вернись в режим команд" | "стоп разговор" | "закончить разговор")
}

fn dialogue_phrase(text: &str) -> &str {
    text.trim().trim_matches(|ch: char| ch.is_ascii_punctuation() || ch == '«' || ch == '»' || ch == '…').trim()
}

#[cfg(test)]
mod dialogue_tests {
    use super::{dialogue_start_phrase, dialogue_stop_phrase};
    #[test]
    fn intent_matches_use_real_dialogue_actions() {
        assert!(dialogue_start_phrase(super::dialogue_command_phrase("dialogue_start").unwrap()));
        assert!(dialogue_stop_phrase(super::dialogue_command_phrase("dialogue_stop").unwrap()));
        assert_eq!(super::dialogue_command_phrase("weather"), None);
    }

    #[test]
    fn start_and_exit_phrases_accept_terminal_punctuation() {
        assert!(dialogue_start_phrase("давай пообщаемся!"));
        assert!(dialogue_start_phrase("давай поболтаем"));
        assert!(dialogue_start_phrase("хочу поговорить"));
        assert!(dialogue_stop_phrase("вернись в режим команд"));
        assert!(dialogue_stop_phrase("закрой диалог."));
        assert!(dialogue_stop_phrase("режим команд"));
        assert!(!dialogue_stop_phrase("расскажи про режим команд"));
    }
}

#[cfg(test)]
mod dispatcher_tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn slow_commands_leave_listener_free_and_keep_execution_order() {
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let dispatcher = CommandDispatcher::start(move |text| {
            if text == "first" {
                entered_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(3)).unwrap();
            }
            done_tx.send(text.to_string()).unwrap();
            false
        });
        assert!(dispatcher.submit("first"));
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let started = Instant::now();
        assert!(dispatcher.submit("second"));
        assert!(started.elapsed() < Duration::from_millis(100));
        assert!(dispatcher.busy());
        assert!(done_rx.try_recv().is_err());
        release_tx.send(()).unwrap();
        assert_eq!(done_rx.recv_timeout(Duration::from_secs(2)).unwrap(), "first");
        assert_eq!(done_rx.recv_timeout(Duration::from_secs(2)).unwrap(), "second");
    }
}

// VAD state machine
#[derive(Debug, Clone, Copy, PartialEq)]
enum VadState {
    WaitingForVoice,
    VoiceActive,
}

pub fn start(text_cmd_rx: Receiver<IncomingCommand>, rt: &tokio::runtime::Runtime) -> Result<(), ()> {
    main_loop(text_cmd_rx, rt)
}

fn main_loop(text_cmd_rx: Receiver<IncomingCommand>, rt: &tokio::runtime::Runtime) -> Result<(), ()> {
    let command_runtime = rt.handle().clone();
    let dispatcher = CommandDispatcher::start(move |text| execute_command(text, &command_runtime));
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

    let mut recording_active = false;
    if !microphone_muted() {
        match recorder::start_recording() {
            Ok(_) => {
                recording_active = true;
                info!("Recording started. Microphone: {}",
                    recorder::get_audio_device_name(recorder::get_selected_microphone_index()));
            }
            Err(_) => {
                error!("Cannot start recording.");
                return Err(());
            }
        }
    }

    ipc::send(IpcEvent::MicrophoneMuted { muted: microphone_muted() });
    ipc::send(IpcEvent::Idle);

    // ### WAKE WORD DETECTION LOOP
    'wake_word: loop {
        if should_stop() {
            info!("Stop signal received, shutting down...");
            voices::play_goodbye();
            ipc::send(IpcEvent::Stopping);
            break;
        }

        if microphone_muted() {
            if recording_active {
                if recorder::stop_recording().is_err() {
                    warn!("Could not stop microphone capture; retrying while audio processing stays paused");
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    continue 'wake_word;
                }
                recording_active = false;
                audio_buffer.clear();
                vad_state = VadState::WaitingForVoice;
                silence_frames = 0;
                stt::reset_wake_recognizer();
                stt::reset_speech_recognizer();
                ipc::send(IpcEvent::MicrophoneMuted { muted: true });
                ipc::send(IpcEvent::Idle);
            }
            if let Ok(text) = text_cmd_rx.try_recv() {
                match text {IncomingCommand::Text(text)=>process_text_command(&text,&dispatcher),request=>{dispatcher.pending.fetch_add(1,Ordering::SeqCst);if dispatcher.sender.try_send(request).is_err(){dispatcher.pending.fetch_sub(1,Ordering::SeqCst);}}}
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
            continue 'wake_word;
        }

        if !recording_active {
            match recorder::start_recording() {
                Ok(()) => {
                    recording_active = true;
                    audio_buffer.clear();
                    vad_state = VadState::WaitingForVoice;
                    silence_frames = 0;
                    stt::reset_wake_recognizer();
                    stt::reset_speech_recognizer();
                    ipc::send(IpcEvent::MicrophoneMuted { muted: false });
                    ipc::send(IpcEvent::Idle);
                }
                Err(()) => {
                    error!("Cannot resume microphone recording");
                    crate::MICROPHONE_MUTED.store(true, Ordering::SeqCst);
                    ipc::send(IpcEvent::MicrophoneMuted { muted: true });
                    ipc::send(IpcEvent::Error { message: "Не удалось включить микрофон".to_string() });
                    continue 'wake_word;
                }
            }
        }

        if let Ok(text) = text_cmd_rx.try_recv() {
            match text {IncomingCommand::Text(text)=>process_text_command(&text,&dispatcher),request=>{dispatcher.pending.fetch_add(1,Ordering::SeqCst);if dispatcher.sender.try_send(request).is_err(){dispatcher.pending.fetch_sub(1,Ordering::SeqCst);}}}
            continue 'wake_word;
        }

        // Dialogue stays open across utterances: no wake word is required until exit.
        // Keep wake-word detection live during playback. Open-mic dialogue/chaining
        // resumes only after playback, so ordinary assistant speech is not a command.
        if !dispatcher.busy() && !playback_active() && (DIALOGUE_MODE.load(Ordering::SeqCst) || CHAIN_LISTENING.swap(false, Ordering::SeqCst)) {
            CHAIN_LISTENING.store(false, Ordering::SeqCst);
            ipc::send(IpcEvent::Listening);
            recognize_command(&mut frame_buffer, &dispatcher, frame_length, sample_rate, false);
            vad_state = VadState::WaitingForVoice;
            silence_frames = 0;
            audio_buffer.clear();
            stt::reset_wake_recognizer();
            stt::reset_speech_recognizer();
            audio_processing::reset();
            if !DIALOGUE_MODE.load(Ordering::SeqCst) {
                ipc::send(IpcEvent::Idle);
            }
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
                    info!("VAD: Voice started, flushing {} buffered frames", audio_buffer.len());
                    
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
                    if playback_active() { tts::stop(); }
                    
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
                    recognize_command(&mut frame_buffer, &dispatcher, frame_length, sample_rate, true);

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
    dispatcher: &CommandDispatcher,
    frame_length: usize,
    sample_rate: usize,
    prefed_audio: bool
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
    let mut pending_question = String::new();
    let question_pause_frames = ((2.2 * sample_rate as f32) / frame_length as f32).ceil() as u32;
    info!("Speech endpoint: {:.1}s pause tolerance; recognition segments are combined", 2.2);
    
    // longer silence threshold for commands (user might pause to think)
    // 5 seconds
    let silence_threshold: u32 = ((5.0 * sample_rate as f32) / frame_length as f32) as u32;
    
    loop {
        if crate::should_stop() {
            return;
        }
        if microphone_muted() {
            return;
        }

        // An explicit wake word allows barge-in. Otherwise return to the main
        // wake-word loop rather than discarding all microphone input during speech.
        if !prefed_audio && playback_active() { return; }
        
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
                if let Some(part) = stt::recognize(frame_buffer, false) {
                    if !part.trim().is_empty() {
                        if !pending_question.is_empty() { pending_question.push(' '); }
                        pending_question.push_str(part.trim());
                    }
                }
                let ready = !processed.is_voice && silence_frames >= question_pause_frames;
                if ready && !pending_question.is_empty() {
                    let mut recognized_voice = std::mem::take(&mut pending_question);
                    info!("Recognized voice: {}", recognized_voice);
                    
                    ipc::send(IpcEvent::SpeechRecognized {
                        text: recognized_voice.clone(),
                    });
                    
                    recognized_voice = recognized_voice.to_lowercase();
                    
                    // check if wake word repeated (reactivate)
                    let wake_phrases = config::get_wake_phrases(&i18n::get_language());
                    let contains_wake = wake_phrases.iter().any(|wp| recognized_voice.contains(wp));

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
                            info!("Wake word + command during chaining: '{}'", remaining);
                            recognized_voice = remaining.to_string();
                            // fall through to command execution below
                        }
                    }

                    first_recognition = false;
                    
                    recognized_voice = remove_assistant_phrases(recognized_voice);
                    
                    if recognized_voice.len() < 5 && !center_pending() {
                        debug!("Ignoring too short recognition: '{}'", recognized_voice);
                        continue;
                    }

                    if recognized_voice.is_empty() {
                        continue;
                    }
                    
                    // execute command and check if we should chain
                    dispatcher.submit(&recognized_voice);
                    return;
                }
                
                // track silence
                if processed.is_voice {
                    silence_frames = 0;
                    start = SystemTime::now();
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


pub static CENTER_PENDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static CENTER_DEADLINE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
fn center_time() -> u64 {std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()}
fn center_pending() -> bool {
    if center_time()>CENTER_DEADLINE.load(Ordering::SeqCst) {CENTER_PENDING.store(false,Ordering::SeqCst);}
    CENTER_PENDING.load(Ordering::SeqCst)
}
pub fn center_reply(text: String, reply_id: String, follow_up: bool) {
    if reply_id == "reminder_notice" {
        // Background reminders must not interrupt speech or alter a clarification dialogue.
        while playback_active() || (CENTER_PENDING.load(Ordering::SeqCst) && center_time() < CENTER_DEADLINE.load(Ordering::SeqCst)) {
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        tts::speak(&text);
        return;
    }
    let timer_warning = matches!(reply_id.as_str(), "timer_warning" | "timer_five_minutes" | "timer_one_minute" | "timer_finished");
    if timer_warning && (CENTER_PENDING.load(Ordering::SeqCst) || playback_active()) { return; }
    if !timer_warning { CENTER_DEADLINE.store(center_time()+90,Ordering::SeqCst);CENTER_PENDING.store(follow_up, Ordering::SeqCst); }
    let language = i18n::get_language();
    if reply_id.is_empty() || !voices::play_command_reply(&reply_id, &language) { tts::speak(&text); }
    if !timer_warning { CHAIN_LISTENING.store(follow_up, Ordering::SeqCst); }
}
fn center_phrase(text: &str) -> bool {
    if ["открой расписание", "покажи расписание", "расписание"].contains(&text) { return true; }
    if text.starts_with("напомни") || (text.contains("напоминан") && ["измен", "редакт", "перенес"].iter().any(|word|text.contains(word))) { return true; }
    if ["почт", "письм", "outlook", "аутлук"].iter().any(|word| text.contains(word))
        && ["откр", "покаж", "обнов", "проверь", "прочит", "напиш", "напис", "созда", "состав", "подготов", "отправ", "ответ", "сохран", "продолж", "непрочитан"].iter().any(|word| text.contains(word)) { return true; }
    if jarvis_core::commands::center::matches_phrase(text) { return true; }
    let actionable=["откр","покаж","запуст","созда","добав","перев","отмет","сброс","приостан","пауз","продолж","возобнов","запи","допол","обнов","останов","выключ","начни","заверш","установ","измени"].iter().any(|word|text.contains(word));
    if !actionable && !["центр","календарь","заметки","привычки","таймер","мои параметры","расписание на сегодня"].contains(&text)
        && !text.starts_with("погода на") && !text.starts_with("прогноз на") && !text.starts_with("мой вес") {return false;}
    ["центр", "календар", "заметк", "чек-лист", "чек лист", "диктовк", "таймер", "пресет", "напоминан", "день рождения", "дни рождения", "привычк", "трениров", "зарядк", "мои параметры", "статистик", "хранилищ", "живые логи", "историю запросов", "история запросов", "голос в текст", "пароли", "город для прогноза", "озвучк", "погода на", "прогноз на", "запиши вес", "мой вес", "измерение", "расписание на сегодня"].iter().any(|word| text.contains(word))
        || ((text.contains("новост") || text.contains("лент")) && ["откр", "перев", "обнов"].iter().any(|verb| text.contains(verb)))
}
fn playback_active() -> bool { tts::is_speaking() || jarvis_core::audio::is_playing() }

fn process_text_command(text: &str, dispatcher: &CommandDispatcher) {
    info!("Processing authenticated text command");
    
    ipc::send(IpcEvent::SpeechRecognized { text: text.to_string() });
    
    let filtered = remove_assistant_phrases(text.to_lowercase());

    let filtered = filtered.trim();
    
    if filtered.is_empty() {
        ipc::send(IpcEvent::Idle);
        return;
    }
    
    // text commands never chain
    if playback_active() { tts::stop(); }
    dispatcher.submit(filtered);
}


// Execute command, returns true if chaining should continue
fn execute_command(text: &str, rt: &tokio::runtime::Handle) -> bool {
    let normalized = if center_pending() { text.to_owned() } else { commands::center::normalize_phrase(text) };
    let text = normalized.as_str();
    if text == "отмена" || text == "отмени" {
        if !center_pending() && dialogue_active() { return execute_command("закончи разговор", rt); }
        CENTER_PENDING.store(false, Ordering::SeqCst);
        ipc::send(IpcEvent::CenterCommand { text: "отмена".into() });
        return false;
    }
    if (center_pending() && CENTER_PENDING.swap(false, Ordering::SeqCst))
        || (!commands::is_negated(text) && center_phrase(text)) {
        if ipc::has_clients() { ipc::send(IpcEvent::CenterCommand { text: text.into() }); }
        else { center_reply("Откройте окно JARVIS, сэр. Центр сейчас не подключён.".into(), "voice_center_unavailable".into(), false); }
        return true;
    }
    let language = i18n::get_language();
    if dialogue_start_phrase(text) {
        DIALOGUE_MODE.store(true, Ordering::SeqCst);
        ipc::send(IpcEvent::DialogueMode { active: true });
        if let Ok(mut history) = DIALOGUE_HISTORY.lock() { history.clear(); }
        if !voices::play_command_reply("dialogue_start", &language) {
            tts::speak("Слушаю, сэр.");
        }
        ipc::send(IpcEvent::CommandExecuted { id: "dialogue_start".to_string(), success: true });
        ipc::send(IpcEvent::Listening);
        return true;
    }
    if DIALOGUE_MODE.load(Ordering::SeqCst) && dialogue_stop_phrase(text) {
        DIALOGUE_MODE.store(false, Ordering::SeqCst);
        ipc::send(IpcEvent::DialogueMode { active: false });
        if let Ok(mut history) = DIALOGUE_HISTORY.lock() { history.clear(); }
        if !voices::play_command_reply("dialogue_stop", &language) {
            tts::speak("Как скажете, сэр. Возвращаюсь в режим команд.");
        }
        ipc::send(IpcEvent::CommandExecuted { id: "dialogue_stop".to_string(), success: true });
        ipc::send(IpcEvent::Idle);
        return false;
    }
    // Re-read command manifests so added phrases and edited packs take effect
    // without a restart. The startup snapshot remains a safe fallback.
    let current_commands = commands::parse_commands().ok();
    let commands_list = match current_commands.as_deref().or_else(|| COMMANDS_LIST.get().map(Vec::as_slice)) {
        Some(c) => c,
        None => {
            ipc::send(IpcEvent::Error { message: "Commands not loaded".to_string() });
            ipc::send(IpcEvent::Idle);
            return false;
        }
    };
    
    let in_dialogue = DIALOGUE_MODE.load(Ordering::SeqCst);
    // The replacement kernel decides aliases/templates/approximate matches.
    // An ambiguous selection must not be overridden by a model prediction.
    let selected_command = match commands::resolve_command(text, commands_list, true) {
        commands::CommandSelection::Found(path, command) => Some((path, command)),
        commands::CommandSelection::Ambiguous(ids) => {
            warn!("ALTRON kernel: ambiguous command: {:?}", ids);
            if !voices::play_command_reply("voice_clarify", &language) {
                tts::speak("Уточните команду, сэр. Подходят несколько действий.");
            }
            ipc::send(IpcEvent::Listening);
            return true;
        }
        commands::CommandSelection::Missing => None,
    };
    let explicit_web_search = ["найди в интернете", "поищи в интернете", "найти в интернете", "проверь в сети", "найди в сети", "поищи в сети", "погугли", "загугли", "отыщи в интернете"]
        .iter().any(|phrase| text.contains(phrase));
    if (in_dialogue || explicit_web_search) && selected_command.is_none() {
        let response_started = std::time::Instant::now();
        let history = if in_dialogue { DIALOGUE_HISTORY.lock().map(|saved| saved.clone()).unwrap_or_default() } else { Vec::new() };
        match jarvis_core::chat::ask_spoken(text, &history) {
            Ok(answer) => {
                info!("Voice latency: chat response queued after {} ms", response_started.elapsed().as_millis());
                if let Ok(mut saved) = DIALOGUE_HISTORY.lock() { if in_dialogue {
                    saved.push((text.to_string(), answer.clone()));
                    let excess = saved.len().saturating_sub(8);
                    if excess > 0 { saved.drain(..excess); }
                } }
                return in_dialogue;
            }
            Err(error) => {
                warn!("Dialogue chat failed: {}", error);
                tts::speak(&format!("Не удалось получить ответ: {error}"));
                return in_dialogue;
            }
        }
    }
    let cmd_result = selected_command.or_else(|| {
        // Do not turn a negative request or an uncertain power action into an
        // executable command through the compatibility intent backend.
        if commands::is_negated(text) { return None; }
        if let Some((intent_id, confidence)) = rt.block_on(intent::classify(text)) {
            info!("Intent recognized: {} (confidence: {:.2})", intent_id, confidence);
            intent::get_command_by_intent(commands_list, &intent_id)
                .filter(|(_, command)| commands::allow_intent_candidate(command))
        } else {
            None
        }
    });
    
    if let Some((cmd_path, cmd_config)) = cmd_result {
        info!("Command found: {:?}", cmd_path);

        // Intent/fuzzy matches must change the same state as exact phrases.
        // The manifest has no Lua action to activate/deactivate the dialogue.
        if let Some(phrase) = dialogue_command_phrase(&cmd_config.id) {
            if cmd_config.id == "dialogue_start" || dialogue_active() { return execute_command(phrase, rt); }
        }

        if cmd_config.id == "repeat_command" {
            let previous = LAST_COMMAND.lock().ok().and_then(|saved| saved.clone());
            if let Some(previous) = previous {
                if !voices::play_command_reply("repeat_command", &language) {
                    tts::speak("Повторяю последнюю команду.");
                }
                ipc::send(IpcEvent::CommandExecuted {
                    id: "repeat_command".to_string(),
                    success: true,
                });
                return execute_command(&previous, rt);
            }
            tts::speak("Пока нечего повторять.");
            ipc::send(IpcEvent::CommandExecuted {
                id: "repeat_command".to_string(),
                success: false,
            });
            ipc::send(IpcEvent::Idle);
            return false;
        }
        
        // extract slots if needed
        let extracted_slots = if !cmd_config.slots.is_empty() {
            let s = slots::extract(text, &cmd_config.slots);
            if !s.is_empty() {
                info!("Extracted slots: {:?}", s);
            }
            Some(s)
        } else {
            None
        };

        // Dynamic Lua responses still speak their values after the recorded introduction.
        // Restart actions must wait for their warning clip before terminating the process.
        let play_before = matches!(cmd_config.id.as_str(),
            "counter" | "counter_add" | "counter_subtract" | "telegram_open" |
            "set_city" | "test_greet_name" | "jarvis_restart" | "computer_restart");
        let played_before = play_before && voices::play_command_reply(&cmd_config.id, &language);
        if !played_before {
            match cmd_config.id.as_str() {
                "counter_add" => { tts::speak("Добавляю один к счётчику."); }
                "counter_subtract" => { tts::speak("Убираю один из счётчика."); }
                "telegram_open" => { tts::speak("Открываю Телеграм."); }
                _ => {}
            }
        }

        match commands::execute_command(&cmd_path, &cmd_config, Some(&text), extracted_slots.as_ref()) {
            Ok(chain) => {
                info!("Command executed successfully");
                if let Ok(mut saved) = LAST_COMMAND.lock() {
                    *saved = Some(text.to_string());
                }
                if !matches!(cmd_config.id.as_str(), "weather" | "counter_add" | "counter_subtract" | "telegram_open") && !played_before
                    && !voices::play_command_reply(&cmd_config.id, &language) {
                    voices::play_random_from(cmd_config.get_sounds(&language).as_slice());
                }
                ipc::send(IpcEvent::CommandExecuted {
                    id: cmd_config.id.clone(),
                    success: true,
                });
                ipc::send(IpcEvent::Idle);
                return chain || DIALOGUE_MODE.load(Ordering::SeqCst);
            }
            Err(msg) => {
                error!("Error executing command: {}", msg);
                voices::play_error();
                ipc::send(IpcEvent::CommandExecuted {
                    id: cmd_config.id.clone(),
                    success: false,
                });
                ipc::send(IpcEvent::Error { message: msg.to_string() });
            }
        }
    } else {
        info!("No command found for: {}", text);
        voices::play_not_found();
        ipc::send(IpcEvent::Error { 
            message: format!("Command not found: {}", text) 
        });
    }
    
    ipc::send(IpcEvent::Idle);
    DIALOGUE_MODE.load(Ordering::SeqCst)
}


pub fn close(code: i32) {
    info!("Closing application.");
    voices::play_goodbye();
    ipc::send(IpcEvent::Stopping);
    std::process::exit(code);
}

fn remove_assistant_phrases(mut text: String) -> String {
    // Strip only the wake word. Verbs such as "давай" and "покажи" are part
    // of real commands and must survive recognition.
    for phrase in config::get_wake_phrases(&i18n::get_language()) {
        if text == *phrase {
            return String::new();
        }

        if let Some(rest) = text.strip_prefix(phrase) {
            if rest.starts_with(|ch: char| ch.is_whitespace() || matches!(ch, ',' | ':' | '!' | '.' | '،')) {
                text = rest.trim_start_matches(|ch: char| ch.is_whitespace() || matches!(ch, ',' | ':' | '!' | '.' | '،')).to_string();
            }
        }
    }
    text
}

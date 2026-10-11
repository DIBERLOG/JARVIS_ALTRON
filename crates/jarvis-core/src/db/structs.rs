use crate::config;
use serde::{Deserialize, Serialize};

use crate::config::structs::SpeechToTextEngine;
use crate::config::structs::WakeWordEngine;
use crate::config::structs::NoiseSuppressionBackend;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Settings {
    pub microphone: i32,
    #[serde(default)]
    pub microphone_muted: bool,
    pub voice: String,

    pub wake_word_engine: WakeWordEngine,

    // backend selections (string IDs matching model or code backend IDs)
    #[serde(default = "default_intent_backend")]
    pub intent_backend: String,
    #[serde(default = "default_slots_backend")]
    pub slots_backend: String,
    #[serde(default = "default_vad_backend")]
    pub vad_backend: String,

    pub gliner_model: String,

    pub speech_to_text_engine: SpeechToTextEngine,
    pub vosk_model: String,

    // audio processing
    pub noise_suppression: NoiseSuppressionBackend,
    pub gain_normalizer: bool,

    #[serde(default = "default_language")]
    pub language: String,

    #[serde(default = "default_chat_provider")]
    pub chat_provider: String,
    #[serde(default = "default_local_chat_model")]
    pub local_chat_model: String,
    #[serde(default = "default_deepseek_chat_model")]
    pub deepseek_chat_model: String,
    #[serde(default = "default_openai_chat_model")]
    pub openai_chat_model: String,
    #[serde(default)]
    pub chat_history: String,
    #[serde(default)]
    pub schedule_website: String,
    #[serde(default = "default_chat_speak_responses")]
    pub chat_speak_responses: bool,
    #[serde(default = "default_tts_mode")]
    pub tts_mode: String,
    #[serde(default = "default_audio_output_mode")]
    pub audio_output_mode: String,
    #[serde(default)]
    pub monitor_self: bool,
    #[serde(default = "default_personality")]
    pub personality: String,
    #[serde(default = "default_voice_dialogue_personality")]
    pub voice_dialogue_personality: String,
    #[serde(default)]
    pub center_data: String,
    #[serde(default)]
    pub center_reminder_announcements: String,
    #[serde(default)]
    pub news_translation_cache: String,

    pub api_keys: ApiKeys,
}

fn default_intent_backend() -> String { config::DEFAULT_INTENT_BACKEND.to_string() }
fn default_slots_backend() -> String { config::DEFAULT_SLOTS_BACKEND.to_string() }
fn default_vad_backend() -> String { config::DEFAULT_VAD_BACKEND.to_string() }
fn default_language() -> String { crate::i18n::detect_system_language().to_string() }
fn default_chat_provider() -> String { "local".to_string() }
fn default_local_chat_model() -> String { "qwen3:8b".to_string() }
fn default_deepseek_chat_model() -> String { "deepseek-flash".to_string() }
fn default_openai_chat_model() -> String { "gpt-4.1-mini".to_string() }
fn default_chat_speak_responses() -> bool { true }
fn default_tts_mode() -> String { "xtts".to_string() }
fn default_audio_output_mode() -> String { "direct".to_string() }
fn default_personality() -> String { "jarvis".to_string() }
fn default_voice_dialogue_personality() -> String { "jarvis".to_string() }

// ### KEY-VALUE ACCESS

impl Settings {
    /// read a setting by key. returns None for unknown keys.
    pub fn get(&self, key: &str) -> Option<String> {
        match key {
            "selected_microphone"       => Some(self.microphone.to_string()),
            "microphone_muted"           => Some(self.microphone_muted.to_string()),
            "assistant_voice"           => Some(self.voice.clone()),
            "selected_wake_word_engine" => Some(format!("{:?}", self.wake_word_engine)),
            "intent_backend"            => Some(self.intent_backend.clone()),
            "slots_backend"             => Some(self.slots_backend.clone()),
            "vad_backend"               => Some(self.vad_backend.clone()),
            "selected_gliner_model"     => Some(self.gliner_model.clone()),
            "selected_vosk_model"       => Some(self.vosk_model.clone()),
            "speech_to_text_engine"     => Some(format!("{:?}", self.speech_to_text_engine)),
            "noise_suppression"         => Some(format!("{:?}", self.noise_suppression)),
            "gain_normalizer"           => Some(self.gain_normalizer.to_string()),
            "language"                  => Some(self.language.clone()),
            "chat_provider"             => Some(self.chat_provider.clone()),
            "local_chat_model"          => Some(self.local_chat_model.clone()),
            "deepseek_chat_model"       => Some(self.deepseek_chat_model.clone()),
            "openai_chat_model"         => Some(self.openai_chat_model.clone()),
            "chat_history_v1"           => Some(self.chat_history.clone()),
            "schedule_website"          => Some(self.schedule_website.clone()),
            "chat_speak_responses"      => Some(self.chat_speak_responses.to_string()),
            "tts_mode"                  => Some(self.tts_mode.clone()),
            "audio_output_mode"         => Some(self.audio_output_mode.clone()),
            "monitor_self"              => Some(self.monitor_self.to_string()),
            "assistant_personality"      => Some(self.personality.clone()),
            "voice_dialogue_personality" => Some(self.voice_dialogue_personality.clone()),
            "center_data"             => Some(self.center_data.clone()),
            "center_reminder_announcements" => Some(self.center_reminder_announcements.clone()),
            "news_translation_cache_v1" => Some(self.news_translation_cache.clone()),
            "api_key__picovoice"        => Some(self.api_keys.picovoice.clone()),
            "api_key__openai"           => Some(self.api_keys.openai.clone()),
            "api_key__deepseek"         => Some(self.api_keys.deepseek.clone()),
            _ => None,
        }
    }

    /// write a setting by key. returns Err for unknown keys or invalid values.
    pub fn set(&mut self, key: &str, val: &str) -> Result<(), String> {
        match key {
            "selected_microphone" => {
                self.microphone = val.parse::<i32>()
                    .map_err(|_| format!("invalid integer: '{}'", val))?;
            }
            "microphone_muted" => self.microphone_muted = match val {
                "true" => true,
                "false" => false,
                _ => return Err("microphone_muted must be true or false".into()),
            },
            "assistant_voice" => {
                self.voice = val.to_string();
            }
            "selected_wake_word_engine" => {
                self.wake_word_engine = match val.to_lowercase().as_str() {
                    "rustpotter" => WakeWordEngine::Rustpotter,
                    "vosk"       => WakeWordEngine::Vosk,
                    "porcupine"  => WakeWordEngine::Porcupine,
                    _ => return Err(format!("unknown wake word engine: '{}'", val)),
                };
            }
            "intent_backend" => {
                self.intent_backend = val.to_string();
            }
            "slots_backend" => {
                self.slots_backend = val.to_string();
            }
            "vad_backend" => {
                self.vad_backend = val.to_string();
            }
            "selected_gliner_model" => {
                self.gliner_model = val.to_string();
            }
            "selected_vosk_model" => {
                self.vosk_model = val.to_string();
            }
            "noise_suppression" => {
                self.noise_suppression = match val.to_lowercase().as_str() {
                    "none"        => NoiseSuppressionBackend::None,
                    "nnnoiseless" => NoiseSuppressionBackend::Nnnoiseless,
                    _ => return Err(format!("unknown noise suppression backend: '{}'", val)),
                };
            }
            "gain_normalizer" => {
                self.gain_normalizer = match val.to_lowercase().as_str() {
                    "true"  => true,
                    "false" => false,
                    _ => return Err(format!("expected 'true' or 'false', got: '{}'", val)),
                };
            }
            "language" => {
                self.language = val.to_string();
            }
            "chat_provider" => {
                if !matches!(val, "local" | "deepseek" | "openai") { return Err("unsupported chat provider".into()); }
                self.chat_provider = val.to_string();
            }
            "local_chat_model" => self.local_chat_model = val.to_string(),
            "deepseek_chat_model" => self.deepseek_chat_model = val.to_string(),
            "openai_chat_model" => {
                if val.trim().is_empty() || val.len() > 200 { return Err("Укажите модель OpenAI".into()); }
                self.openai_chat_model = val.trim().to_string();
            },
            "schedule_website" => {
                let raw=val.trim();
                if !raw.is_empty() {
                    if raw.len()>4096 { return Err("Ссылка слишком длинная".into()); }
                    let url=url::Url::parse(raw).map_err(|_| "Введите полную ссылку: https://example.com")?;
                    if !matches!(url.scheme(),"http"|"https") || url.host_str().is_none() || !url.username().is_empty() || url.password().is_some() {
                        return Err("Разрешены только ссылки http/https без логина и пароля в адресе".into());
                    }
                }
                self.schedule_website=raw.into();
            },
            "chat_history_v1" => {
                if val.len() > 20_000_000 { return Err("История чатов превышает 20 МБ".into()); }
                let history: serde_json::Value = serde_json::from_str(val).map_err(|_| "Некорректная история чатов")?;
                if !history.is_array() { return Err("История чатов должна быть списком".into()); }
                self.chat_history = val.to_string();
            },
            "chat_speak_responses" => self.chat_speak_responses = match val { "true" => true, "false" => false, _ => return Err("expected true or false".into()) },
            "tts_mode" => { if !matches!(val, "silero" | "xtts" | "s2") { return Err("tts mode must be silero, xtts or s2".into()) }; self.tts_mode = val.to_string(); }
            "monitor_self" => self.monitor_self = match val { "true" => true, "false" => false, _ => return Err("expected true or false".into()) },
            "audio_output_mode" => { if !matches!(val,"direct"|"voicemod") { return Err("unsupported audio output mode".into()) }; self.audio_output_mode=val.into(); },
            "assistant_personality" => { if val != "jarvis" && val != "altron" { return Err("personality must be jarvis or altron".into()) }; self.personality = val.to_string(); }
            "voice_dialogue_personality" => { if val != "jarvis" && val != "altron" { return Err("voice dialogue personality must be jarvis or altron".into()) }; self.voice_dialogue_personality = val.to_string(); }
            "center_data" => {
                if val.len() > 20_000_000 { return Err("Центр превышает 20 МБ. Удалите часть изображений или заметок.".into()); }
                serde_json::from_str::<serde_json::Value>(val).map_err(|e| format!("invalid center data: {e}"))?;
                self.center_data = val.to_string();
            }
            "center_reminder_announcements" => {
                if val.len()>4_000_000 { return Err("Слишком большой журнал напоминаний".into()); }
                let ledger: std::collections::HashMap<String, Vec<String>> = serde_json::from_str(val).map_err(|_|"Некорректный журнал напоминаний")?;
                if ledger.values().any(|keys| keys.len()>1000) { return Err("Слишком много отметок напоминания".into()); }
                self.center_reminder_announcements = val.to_string();
            }
            "news_translation_cache_v1" => {
                if val.len()>4_000_000 {return Err("Кэш переводов слишком большой".into())}
                let entries:serde_json::Value=serde_json::from_str(val).map_err(|_|"Некорректный кэш переводов")?;
                if !entries.is_array(){return Err("Некорректный кэш переводов".into())}
                self.news_translation_cache=val.to_string();
            }
            "api_key__picovoice" => {
                self.api_keys.picovoice = val.to_string();
            }
            "api_key__openai" => {
                self.api_keys.openai = val.to_string();
            }
            "api_key__deepseek" => self.api_keys.deepseek = val.to_string(),
            _ => return Err(format!("unknown setting: '{}'", key)),
        }
        Ok(())
    }

    /// all valid setting keys (for enumeration, debugging, etc.)
    pub fn keys() -> &'static [&'static str] {
        &[
            "selected_microphone",
            "microphone_muted",
            "assistant_voice",
            "selected_wake_word_engine",
            "intent_backend",
            "slots_backend",
            "vad_backend",
            "selected_gliner_model",
            "selected_vosk_model",
            "speech_to_text_engine",
            "noise_suppression",
            "gain_normalizer",
            "language",
            "chat_provider",
            "local_chat_model",
            "deepseek_chat_model",
            "openai_chat_model",
            "chat_history_v1",
            "schedule_website",
            "chat_speak_responses",
            "tts_mode",
            "audio_output_mode",
            "monitor_self",
            "assistant_personality",
            "voice_dialogue_personality",
            "center_data",
            "center_reminder_announcements",
            "api_key__picovoice",
            "api_key__openai",
            "api_key__deepseek",
        ]
    }
}

// ### DEFAULT

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            microphone: -1,
            microphone_muted: false,
            voice: String::from(""),

            wake_word_engine: config::DEFAULT_WAKE_WORD_ENGINE,

            intent_backend: config::DEFAULT_INTENT_BACKEND.to_string(),
            slots_backend: config::DEFAULT_SLOTS_BACKEND.to_string(),
            vad_backend: config::DEFAULT_VAD_BACKEND.to_string(),

            gliner_model: String::new(),
            speech_to_text_engine: config::DEFAULT_SPEECH_TO_TEXT_ENGINE,
            vosk_model: String::from(""),

            noise_suppression: config::DEFAULT_NOISE_SUPPRESSION,
            gain_normalizer: config::DEFAULT_GAIN_NORMALIZER,

            language: crate::i18n::detect_system_language().to_string(),

            chat_provider: default_chat_provider(),
            local_chat_model: default_local_chat_model(),
            deepseek_chat_model: default_deepseek_chat_model(),
            openai_chat_model: default_openai_chat_model(),
            chat_history: String::new(),
            schedule_website: String::new(),
            chat_speak_responses: default_chat_speak_responses(),
            tts_mode: default_tts_mode(),
            audio_output_mode: default_audio_output_mode(),
            monitor_self: false,
            personality: default_personality(),
            voice_dialogue_personality: default_voice_dialogue_personality(),
            center_data: String::new(),
            center_reminder_announcements: String::new(),
            news_translation_cache: String::new(),

            api_keys: ApiKeys {
                picovoice: String::from(""),
                openai: String::from(""),
                deepseek: String::from(""),
            },
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ApiKeys {
    pub picovoice: String,
    pub openai: String,
    #[serde(default)]
    pub deepseek: String,
}

#[cfg(test)]
mod tts_mode_tests {
    #[test]
    fn schedule_website_accepts_only_web_links_and_can_be_cleared() {
        let mut settings=Settings::default();
        assert_eq!(settings.get("schedule_website").as_deref(),Some(""));
        settings.set("schedule_website","https://example.com/calendar?a=1&b=2").unwrap();
        for bad in ["javascript:alert(1)","file:///C:/test","https://user:password@example.com","not a link"] {
            assert!(settings.set("schedule_website",bad).is_err());
        }
        assert_eq!(settings.schedule_website,"https://example.com/calendar?a=1&b=2");
        settings.set("schedule_website","").unwrap();
        assert!(settings.schedule_website.is_empty());
    }
    #[test]
    fn openai_settings_and_history_migrate_without_losing_existing_data() {
        let mut value=serde_json::to_value(Settings::default()).unwrap();
        for key in ["openai_chat_model","chat_history"] { value.as_object_mut().unwrap().remove(key); }
        let mut settings:Settings=serde_json::from_value(value).unwrap();
        assert_eq!(settings.openai_chat_model,"gpt-4.1-mini");
        assert_eq!(settings.chat_history,"");
        settings.set("chat_provider","openai").unwrap();
        settings.set("chat_history_v1","[]").unwrap();
        assert!(settings.set("chat_history_v1","{}").is_err());
        assert!(settings.set("chat_provider","unknown").is_err());
        assert!(settings.set("openai_chat_model","").is_err());
    }
    use super::Settings;

    #[test]
    fn audio_route_defaults_to_direct_and_validates_saved_values() {
        let mut value=serde_json::to_value(Settings::default()).unwrap();
        value.as_object_mut().unwrap().remove("audio_output_mode");
        let mut settings:Settings=serde_json::from_value(value).unwrap();
        assert_eq!(settings.get("audio_output_mode").as_deref(),Some("direct"));
        settings.set("audio_output_mode","voicemod").unwrap();
        let restored:Settings=serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert_eq!(restored.get("audio_output_mode").as_deref(),Some("voicemod"));
        assert!(settings.set("audio_output_mode","unknown").is_err());
    }

    #[test]
    fn old_settings_default_to_xtts() {
        let mut value = serde_json::to_value(Settings::default()).unwrap();
        value.as_object_mut().unwrap().remove("tts_mode");
        let settings: Settings = serde_json::from_value(value).unwrap();
        assert_eq!(settings.get("tts_mode").as_deref(), Some("xtts"));
    }

    #[test]
    fn only_supported_tts_modes_can_be_saved() {
        let mut settings = Settings::default();
        settings.set("tts_mode", "xtts").unwrap();
        assert_eq!(settings.get("tts_mode").as_deref(), Some("xtts"));
        settings.set("tts_mode", "s2").unwrap();
        assert_eq!(settings.get("tts_mode").as_deref(), Some("s2"));
        assert!(settings.set("tts_mode", "windows").is_err());
    }

    #[test]
    fn microphone_monitor_setting_survives_serialization() {
        let mut settings = super::Settings::default();
        settings.set("monitor_self", "true").unwrap();
        let restored: super::Settings = serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert_eq!(restored.get("monitor_self").as_deref(), Some("true"));
    }
}

#[cfg(test)]
mod center_data_tests {
    use super::Settings;

    #[test]
    fn center_data_survives_settings_round_trip() {
        let mut settings = Settings::default();
        settings.set("center_data", r#"{"reminders":[{"id":"1","title":"Зал","dueAt":"2026-10-01T09:00","done":false}],"birthdays":[]}"#).unwrap();
        settings.set("center_reminder_announcements", r#"{"1":["2026-10-01T09:00:0"]}"#).unwrap();
        let restored: Settings = serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert!(restored.get("center_data").unwrap().contains("Зал"));
        assert!(restored.get("center_reminder_announcements").unwrap().contains("2026-10-01"));
        assert!(settings.set("center_reminder_announcements", "[]").is_err());
        assert!(settings.set("center_data", "not-json").is_err());
    }
}

use once_cell::sync::{Lazy, OnceCell};
use parking_lot::RwLock;
use std::{sync::Arc};
use platform_dirs::AppDirs;
use std::path::PathBuf;

#[macro_use]
extern crate log;

pub mod time;

pub mod audio;
pub mod commands;
pub mod config;
pub mod db;
pub mod i18n;
pub mod notifications;
pub mod platform;
pub mod weather_city;
pub mod news_filter;

#[cfg(feature = "jarvis_app")]
pub mod listener;

pub mod recorder;

#[cfg(feature = "jarvis_app")]
pub mod stt;

#[cfg(feature = "intent")]
pub mod intent;

#[cfg(feature = "jarvis_app")]
pub mod slots;

pub mod models;

// re-exported from models/
pub use models::vosk_models;
pub use models::gliner_models;

#[cfg(feature = "jarvis_app")]
pub mod audio_processing;

#[cfg(feature = "jarvis_app")]
pub mod ipc;

pub mod voices;
pub mod tts;

pub mod chat;

pub mod audio_buffer;

#[cfg(feature = "lua")]
pub mod lua;

// shared statics
// pub static APP_DIR: Lazy<PathBuf> = Lazy::new(|| std::env::current_dir().unwrap());
pub static APP_DIR: Lazy<PathBuf> = Lazy::new(|| {
    let executable = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("jarvis"));
    platform::resource_root(&executable, std::env::var_os("JARVIS_RESOURCE_ROOT").map(PathBuf::from))
});
pub static SOUND_DIR: Lazy<PathBuf> = Lazy::new(|| APP_DIR.clone().join(config::SOUND_PATH));
pub static APP_DIRS: OnceCell<AppDirs> = OnceCell::new();
pub static APP_CONFIG_DIR: OnceCell<PathBuf> = OnceCell::new();
pub static APP_LOG_DIR: OnceCell<PathBuf> = OnceCell::new();
pub static DB: OnceCell<Arc<RwLock<db::structs::Settings>>> = OnceCell::new();
pub static COMMANDS_LIST: OnceCell<Vec<JCommandsList>> = OnceCell::new();

// re-exports
pub use commands::JCommandsList;
pub use config::structs::*;
pub use db::structs::Settings;
pub use db::SettingsManager;

// use crate::commands::{JComandsList, JCommand};

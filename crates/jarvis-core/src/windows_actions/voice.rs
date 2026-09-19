//! The voice router: a recognized phrase becomes the same typed action as a click.
//!
//! This module only *maps* a phrase to a [`WindowsAction`]. It never executes anything: the
//! caller takes the action through the same policy, the same confirmation gate, and the same
//! executor as the interface and the model. That is the whole point of having it here rather
//! than in the voice host — a second execution path is how a safe feature grows an unsafe
//! one.
//!
//! Two rules that matter more than the phrase list:
//!
//! * **an ambiguity is not an action.** If a phrase names a window and two windows match, or
//!   names an application that is not in the allowed list, the answer is
//!   [`VoiceOutcome::Ambiguous`] or [`VoiceOutcome::NotAnAction`] — never a guess;
//! * **a dictated sentence stays a sentence.** Numbers, durations, and identifiers are parsed
//!   from a small, closed vocabulary, and the rest of the phrase is only ever used as reminder
//!   *text* (which is data, sealed locally) or as a title fragment to look up. Nothing from a
//!   transcript is ever treated as a command line, a path, or an argument.

use super::allowlist::AllowedApplications;
use super::executor::WindowRegistry;
use super::model::{
    ScreenshotTarget, VolumeDirection, WindowOperation, WindowsAction, MAX_REMINDER_CHARS,
};

/// What a transcript turned into.
#[derive(Clone, Debug, PartialEq)]
pub enum VoiceOutcome {
    /// One action, ready to be requested through the normal path.
    Action(Box<WindowsAction>),
    /// The phrase is an action, but something about it is not decided (several windows match,
    /// an application is not allowed, a number is missing).
    Ambiguous { reason: &'static str },
    /// The phrase is not a Windows action; the caller may try the ordinary command route.
    NotAnAction,
}

impl VoiceOutcome {
    pub fn action(&self) -> Option<&WindowsAction> {
        match self {
            Self::Action(action) => Some(action),
            _ => None,
        }
    }
}

/// Default step of a relative volume change from voice.
pub const VOICE_VOLUME_STEP: u8 = 10;

/// The words that mean "confirm" and "cancel" for a pending action.
pub const VOICE_CONFIRM_WORDS: [&str; 4] =
    ["подтверждаю", "подтверждаю действие", "confirm", "yes"];
pub const VOICE_CANCEL_WORDS: [&str; 5] = ["отмена", "отменить", "отмени", "cancel", "no"];

/// Whether a transcript is a confirmation of the pending action.
pub fn is_confirmation(text: &str) -> bool {
    let normalized = normalize(text);
    VOICE_CONFIRM_WORDS.contains(&normalized.as_str())
}

/// Whether a transcript cancels the pending action.
pub fn is_cancellation(text: &str) -> bool {
    let normalized = normalize(text);
    VOICE_CANCEL_WORDS.contains(&normalized.as_str())
}

/// Maps one transcript to an action.
///
/// `windows` is the current listing, so a phrase that names a window is resolved against
/// identifiers this application just minted rather than against a title.
pub fn route(
    text: &str,
    allowlist: &AllowedApplications,
    windows: &WindowRegistry,
    now_ms: u64,
) -> VoiceOutcome {
    let normalized = normalize(text);
    if normalized.is_empty() {
        return VoiceOutcome::NotAnAction;
    }
    if let Some(outcome) = volume(&normalized) {
        return outcome;
    }
    if let Some(outcome) = window_operation(&normalized, windows, now_ms) {
        return outcome;
    }
    if let Some(outcome) = timer_or_reminder(&normalized) {
        return outcome;
    }
    if lock(&normalized) {
        return VoiceOutcome::Action(Box::new(WindowsAction::LockWorkstation));
    }
    if list_windows(&normalized) {
        return VoiceOutcome::Action(Box::new(WindowsAction::ListWindows));
    }
    if screenshot(&normalized) {
        return VoiceOutcome::Action(Box::new(WindowsAction::TakeScreenshot {
            target: ScreenshotTarget::PrimaryMonitor,
        }));
    }
    if let Some(outcome) = launch(&normalized, allowlist) {
        return outcome;
    }
    VoiceOutcome::NotAnAction
}

fn volume(text: &str) -> Option<VoiceOutcome> {
    if !mentions(text, &["громкость", "звук", "volume", "sound", "гучність"]) {
        // "тише"/"громче" without the word volume is still a volume command.
        if mentions(
            text,
            &["тише", "потише", "громче", "погромче", "quieter", "louder"],
        ) {
            let direction = if mentions(text, &["тише", "потише", "quieter"]) {
                VolumeDirection::Down
            } else {
                VolumeDirection::Up
            };
            return Some(VoiceOutcome::Action(Box::new(
                WindowsAction::ChangeVolume {
                    direction,
                    step: VOICE_VOLUME_STEP,
                },
            )));
        }
        return None;
    }
    if mentions(text, &["выключи", "отключи", "mute", "заглуши"]) {
        return Some(VoiceOutcome::Action(Box::new(WindowsAction::MuteVolume {
            muted: true,
        })));
    }
    if mentions(text, &["включи", "верни", "unmute"]) {
        return Some(VoiceOutcome::Action(Box::new(WindowsAction::MuteVolume {
            muted: false,
        })));
    }
    if let Some(percent) = first_percent(text) {
        return Some(VoiceOutcome::Action(Box::new(WindowsAction::SetVolume {
            percent,
        })));
    }
    if mentions(
        text,
        &["тише", "потише", "убавь", "снизь", "quieter", "down"],
    ) {
        return Some(VoiceOutcome::Action(Box::new(
            WindowsAction::ChangeVolume {
                direction: VolumeDirection::Down,
                step: VOICE_VOLUME_STEP,
            },
        )));
    }
    if mentions(text, &["громче", "прибавь", "увеличь", "louder", "up"]) {
        return Some(VoiceOutcome::Action(Box::new(
            WindowsAction::ChangeVolume {
                direction: VolumeDirection::Up,
                step: VOICE_VOLUME_STEP,
            },
        )));
    }
    // "громкость" with no number and no direction: ask a question, do not guess.
    if mentions(text, &["какая", "сколько", "current", "what"]) {
        return Some(VoiceOutcome::Action(Box::new(WindowsAction::GetVolume)));
    }
    Some(VoiceOutcome::Ambiguous {
        reason: "windows-voice-volume-unclear",
    })
}

fn window_operation(text: &str, windows: &WindowRegistry, now_ms: u64) -> Option<VoiceOutcome> {
    let operation = if mentions(text, &["сверни", "минимизируй", "minimize"]) {
        WindowOperation::Minimize
    } else if mentions(text, &["разверни", "максимизируй", "maximize"]) {
        WindowOperation::Maximize
    } else if mentions(text, &["восстанови", "верни окно", "restore"]) {
        WindowOperation::Restore
    } else if mentions(text, &["закрой окно", "закрыть окно", "close window"])
    {
        WindowOperation::Close
    } else if mentions(text, &["передвинь", "перемести", "move window"]) {
        // A move needs coordinates; a spoken position is not something this stage guesses.
        return Some(VoiceOutcome::Ambiguous {
            reason: "windows-voice-move-unclear",
        });
    } else if mentions(text, &["подвинь", "сдвинь", "перемести"]) {
        // A spoken position or size is not something this stage interprets: the interface has
        // a form with numbers for that, and guessing a geometry from a sentence is exactly
        // the kind of invention this feature avoids.
        return Some(VoiceOutcome::Ambiguous {
            reason: "windows-voice-move-unclear",
        });
    } else {
        return None;
    };

    if !mentions(
        text,
        &["окно", "window", "вікно", "программу", "приложение"],
    ) {
        return Some(VoiceOutcome::Ambiguous {
            reason: "windows-voice-window-unspecified",
        });
    }
    let named = named_window(text, windows, now_ms);
    match named {
        WindowChoice::One(window) => Some(VoiceOutcome::Action(Box::new(WindowsAction::Window {
            window_id: window,
            operation,
        }))),
        WindowChoice::None => Some(VoiceOutcome::Ambiguous {
            reason: "windows-voice-window-not-found",
        }),
        WindowChoice::Many => Some(VoiceOutcome::Ambiguous {
            reason: "windows-voice-window-ambiguous",
        }),
    }
}

enum WindowChoice {
    One(super::model::WindowId),
    None,
    Many,
}

fn current_window(
    windows: &WindowRegistry,
    now_ms: u64,
) -> Result<super::model::WindowId, VoiceOutcome> {
    let Some(window) = windows.foreground(now_ms) else {
        return Err(VoiceOutcome::Ambiguous {
            reason: "windows-voice-no-foreground-window",
        });
    };
    super::model::WindowId::from_stored(window.id).map_err(|_| VoiceOutcome::Ambiguous {
        reason: "windows-voice-window-not-found",
    })
}

/// Resolves "… окно <что-то>" against the current listing.
fn named_window(text: &str, windows: &WindowRegistry, now_ms: u64) -> WindowChoice {
    // Without a name, the foreground window is the one the user means.
    for filler in ["текущее", "это", "current", "this", "активное"] {
        if text.contains(filler) {
            return match current_window(windows, now_ms) {
                Ok(window) => WindowChoice::One(window),
                Err(_) => WindowChoice::None,
            };
        }
    }
    let Some((_, fragment)) = text.split_once("окно ") else {
        return match current_window(windows, now_ms) {
            Ok(window) => WindowChoice::One(window),
            Err(_) => WindowChoice::None,
        };
    };
    let fragment = fragment.trim();
    if fragment.is_empty() {
        return match current_window(windows, now_ms) {
            Ok(window) => WindowChoice::One(window),
            Err(_) => WindowChoice::None,
        };
    }
    let matches = windows.matching_title(fragment, now_ms);
    match matches.len() {
        0 => WindowChoice::None,
        1 => super::model::WindowId::from_stored(matches[0].id.clone())
            .map(WindowChoice::One)
            .unwrap_or(WindowChoice::None),
        _ => WindowChoice::Many,
    }
}

fn timer_or_reminder(text: &str) -> Option<VoiceOutcome> {
    let is_reminder = mentions(text, &["напомни", "напоминание", "remind", "нагадай"]);
    let is_timer = mentions(text, &["таймер", "timer", "обратный отсчёт"]);
    if !is_reminder && !is_timer {
        return None;
    }
    let Some(seconds) = duration_seconds(text) else {
        return Some(VoiceOutcome::Ambiguous {
            reason: "windows-voice-duration-unclear",
        });
    };
    if is_reminder {
        let message = reminder_text(text);
        if message.is_empty() {
            return Some(VoiceOutcome::Ambiguous {
                reason: "windows-voice-reminder-text-missing",
            });
        }
        return Some(VoiceOutcome::Action(Box::new(
            WindowsAction::CreateReminder {
                delay_seconds: seconds,
                message,
            },
        )));
    }
    Some(VoiceOutcome::Action(Box::new(WindowsAction::CreateTimer {
        duration_seconds: seconds,
    })))
}

/// The text of a reminder: everything after the verb, minus the time expression.
fn reminder_text(text: &str) -> String {
    let mut body = text.to_string();
    for verb in [
        "напомни",
        "напомнить",
        "напоминание",
        "remind me to",
        "remind",
    ] {
        if let Some(index) = body.find(verb) {
            body = body[index + verb.len()..].to_string();
        }
    }
    // "через 30 минут" is the time, not the message.
    if let Some(index) = body.find("через") {
        let after = &body[index + "через".len()..];
        let mut words = after.split_whitespace();
        let mut consumed = 0usize;
        for word in words.by_ref() {
            consumed += word.len() + 1;
            if number_in_word(word).is_some() || duration_unit(word).is_some() {
                continue;
            }
            break;
        }
        body = after[consumed.min(after.len())..].to_string();
    }
    body = body
        .replace("сделать", "")
        .replace("что", "")
        .trim()
        .trim_start_matches([':', ',', '-', '—'])
        .trim()
        .to_string();
    body.chars().take(MAX_REMINDER_CHARS).collect()
}

fn lock(text: &str) -> bool {
    mentions(
        text,
        &[
            "заблокируй",
            "заблокировать",
            "блокировка",
            "lock workstation",
            "lock computer",
            "заблокуй",
        ],
    )
}

fn list_windows(text: &str) -> bool {
    mentions(
        text,
        &["список окон", "какие окна", "list windows", "покажи окна"],
    )
}

fn screenshot(text: &str) -> bool {
    mentions(
        text,
        &["скриншот", "снимок экрана", "screenshot", "знімок екрана"],
    )
}

fn launch(text: &str, allowlist: &AllowedApplications) -> Option<VoiceOutcome> {
    let verbs = [
        "открой",
        "запусти",
        "включи",
        "open",
        "launch",
        "start",
        "відкрий",
        "запусти",
    ];
    if !mentions(text, &verbs) {
        return None;
    }
    let mut remainder = text.to_string();
    for verb in verbs {
        if let Some(index) = remainder.find(verb) {
            remainder = remainder[index + verb.len()..].to_string();
            break;
        }
    }
    let target = remainder
        .trim()
        .trim_start_matches([':', ',', '-', '—'])
        .trim()
        .to_string();
    if target.is_empty() {
        return Some(VoiceOutcome::Ambiguous {
            reason: "windows-voice-launch-unspecified",
        });
    }
    let matches: Vec<&super::allowlist::AllowedApplication> = allowlist
        .enabled()
        .into_iter()
        .filter(|application| {
            let name = application.display_name.to_lowercase();
            name.contains(&target) || target.contains(&name)
        })
        .collect();
    match matches.len() {
        0 => Some(VoiceOutcome::Ambiguous {
            reason: "windows-voice-application-not-allowed",
        }),
        1 => Some(VoiceOutcome::Action(Box::new(
            WindowsAction::LaunchAllowedApplication {
                application_id: matches[0].application_id(),
            },
        ))),
        _ => Some(VoiceOutcome::Ambiguous {
            reason: "windows-voice-application-ambiguous",
        }),
    }
}

/// Seconds in a phrase such as "через 30 минут" or "на 10 минут".
///
/// Only a small vocabulary is understood; anything else is reported as unclear rather than
/// guessed.
fn duration_seconds(text: &str) -> Option<u64> {
    for word in text.split_whitespace() {
        if let Some(value) = number_in_word(word) {
            let unit = next_unit(text, word);
            let multiplier = unit.unwrap_or(60);
            let seconds = value.saturating_mul(multiplier);
            if (super::model::MIN_TIMER_SECONDS..=super::model::MAX_REMINDER_SECONDS)
                .contains(&seconds)
            {
                return Some(seconds);
            }
            return None;
        }
    }
    None
}

fn next_unit(text: &str, number_word: &str) -> Option<u64> {
    let index = text.find(number_word)?;
    let after = &text[index + number_word.len()..];
    after.split_whitespace().find_map(duration_unit)
}

fn duration_unit(word: &str) -> Option<u64> {
    let word = word.trim_matches(|character: char| !character.is_alphabetic());
    match word {
        "секунду" | "секунды" | "секунд" | "сек" | "second" | "seconds" | "секунду." => {
            Some(1)
        }
        "минуту" | "минуты" | "минут" | "мин" | "minute" | "minutes" => {
            Some(60)
        }
        "час" | "часа" | "часов" | "hour" | "hours" | "годину" | "години" => {
            Some(3600)
        }
        _ => None,
    }
}

fn number_in_word(word: &str) -> Option<u64> {
    let digits: String = word
        .chars()
        .filter(|character| character.is_ascii_digit())
        .collect();
    if !digits.is_empty() {
        return digits.parse::<u64>().ok();
    }
    // A small set of spoken numbers, because a voice transcript writes them as words.
    const WORDS: [(&str, u64); 20] = [
        ("пять", 5),
        ("десять", 10),
        ("пятнадцать", 15),
        ("двадцать", 20),
        ("тридцать", 30),
        ("сорок", 40),
        ("пятьдесят", 50),
        ("шестьдесят", 60),
        ("семьдесят", 70),
        ("восемьдесят", 80),
        ("девяносто", 90),
        ("сто", 100),
        ("five", 5),
        ("ten", 10),
        ("fifteen", 15),
        ("twenty", 20),
        ("thirty", 30),
        ("forty", 40),
        ("fifty", 50),
        ("sixty", 60),
    ];
    let lowered = word.trim_matches(|character: char| !character.is_alphabetic());
    WORDS
        .iter()
        .find(|(name, _)| *name == lowered)
        .map(|(_, value)| *value)
}

/// The first percentage in a phrase, 0–100.
fn first_percent(text: &str) -> Option<u8> {
    for word in text.split_whitespace() {
        if let Some(value) = number_in_word(word) {
            if value <= 100 {
                return Some(value as u8);
            }
        }
    }
    None
}

fn mentions(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}

/// Lowercases and collapses whitespace, dropping the filler words the host adds.
fn normalize(text: &str) -> String {
    let mut cleaned = text.to_lowercase().replace(['ё', 'Ё'], "е");
    cleaned = cleaned.replace(['\u{2014}', '\u{2013}'], " ");
    for filler in [
        "пожалуйста",
        "jarvis",
        "джарвис",
        "altron",
        "алтрон",
        "please",
    ] {
        cleaned = cleaned.replace(filler, " ");
    }
    cleaned
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || character == '%' {
                character
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::windows_actions::allowlist::{AllowedApplicationDraft, AllowedApplications};
    use crate::windows_actions::backend::NativeWindow;
    use crate::windows_actions::model::{WindowId, WindowState};
    use tempfile::tempdir;

    fn window(title: &str, foreground: bool) -> NativeWindow {
        NativeWindow {
            id: WindowId::mint().unwrap().as_str().to_string(),
            native_id: 5,
            process_id: 6,
            process_name: "App.exe".to_string(),
            title: title.to_string(),
            state: WindowState::Normal,
            monitor: 1,
            is_own_process: false,
            is_foreground: foreground,
            work_area: (0, 0, 1920, 1040),
        }
    }

    fn registry(windows: Vec<NativeWindow>) -> WindowRegistry {
        let mut registry = WindowRegistry::new();
        registry.replace(windows, 1_000);
        registry
    }

    fn allowlist_with(directory: &std::path::Path, name: &str) -> AllowedApplications {
        let program = directory.join(format!("{name}.exe"));
        std::fs::write(&program, b"FICTIONAL").unwrap();
        let mut allowlist = AllowedApplications::open(directory);
        allowlist
            .add(
                &AllowedApplicationDraft {
                    display_name: name.to_string(),
                    path: program.to_string_lossy().into_owned(),
                    fixed_arguments: Vec::new(),
                    working_directory: None,
                },
                "now",
            )
            .unwrap();
        allowlist
    }

    #[test]
    fn volume_phrases_become_typed_actions() {
        let directory = tempdir().unwrap();
        let allowlist = AllowedApplications::open(directory.path());
        let windows = registry(Vec::new());
        assert_eq!(
            route("громкость 40 процентов", &allowlist, &windows, 1_100).action(),
            Some(&WindowsAction::SetVolume { percent: 40 })
        );
        assert_eq!(
            route("сделай тише", &allowlist, &windows, 1_100).action(),
            Some(&WindowsAction::ChangeVolume {
                direction: VolumeDirection::Down,
                step: VOICE_VOLUME_STEP
            })
        );
        assert_eq!(
            route("выключи звук", &allowlist, &windows, 1_100).action(),
            Some(&WindowsAction::MuteVolume { muted: true })
        );
        assert_eq!(
            route("включи звук", &allowlist, &windows, 1_100).action(),
            Some(&WindowsAction::MuteVolume { muted: false })
        );
        // A phrase without a number or a direction is a question, not a guess.
        assert!(matches!(
            route("громкость", &allowlist, &windows, 1_100),
            VoiceOutcome::Ambiguous { .. }
        ));
    }

    #[test]
    fn timers_and_reminders_carry_a_bounded_duration() {
        let directory = tempdir().unwrap();
        let allowlist = AllowedApplications::open(directory.path());
        let windows = registry(Vec::new());
        match route("поставь таймер на 10 минут", &allowlist, &windows, 1_100) {
            VoiceOutcome::Action(action) => {
                assert_eq!(
                    *action,
                    WindowsAction::CreateTimer {
                        duration_seconds: 600
                    }
                )
            }
            other => panic!("unexpected {other:?}"),
        }
        match route(
            "напомни через 30 минут сделать перерыв",
            &allowlist,
            &windows,
            1_100,
        ) {
            VoiceOutcome::Action(action) => match *action {
                WindowsAction::CreateReminder {
                    delay_seconds,
                    message,
                } => {
                    assert_eq!(delay_seconds, 1800);
                    assert!(message.contains("перерыв"), "{message}");
                }
                other => panic!("unexpected action {other:?}"),
            },
            other => panic!("unexpected {other:?}"),
        }
        // A duration this feature does not accept is not guessed into one.
        assert!(matches!(
            route("поставь таймер на 2 секунды", &allowlist, &windows, 1_100),
            VoiceOutcome::Ambiguous { .. }
        ));
        assert!(matches!(
            route("напомни через 10 минут", &allowlist, &windows, 1_100),
            VoiceOutcome::Ambiguous { .. }
        ));
    }

    #[test]
    fn window_phrases_resolve_against_the_current_listing() {
        let directory = tempdir().unwrap();
        let allowlist = AllowedApplications::open(directory.path());
        let windows = registry(vec![
            window("Notepad — notes.txt", true),
            window("Calculator", false),
        ]);
        match route("сверни текущее окно", &allowlist, &windows, 1_100) {
            VoiceOutcome::Action(action) => match *action {
                WindowsAction::Window { operation, .. } => {
                    assert_eq!(operation, WindowOperation::Minimize)
                }
                other => panic!("unexpected {other:?}"),
            },
            other => panic!("unexpected {other:?}"),
        }
        // A named window that does not exist is not guessed.
        assert!(matches!(
            route("сверни окно проводник", &allowlist, &windows, 1_100),
            VoiceOutcome::Ambiguous { .. }
        ));
        // Two matches are an ambiguity, not a coin toss.
        let ambiguous = registry(vec![window("Notepad A", false), window("Notepad B", false)]);
        assert!(matches!(
            route("сверни окно notepad", &allowlist, &ambiguous, 1_100),
            VoiceOutcome::Ambiguous { .. }
        ));
        // A stale listing cannot be used at all.
        assert!(matches!(
            route("сверни текущее окно", &allowlist, &windows, 10_000_000),
            VoiceOutcome::Ambiguous { .. }
        ));
    }

    #[test]
    fn a_launch_only_resolves_to_an_allowed_identifier() {
        let directory = tempdir().unwrap();
        let allowlist = allowlist_with(directory.path(), "Калькулятор");
        let windows = registry(Vec::new());
        match route("открой калькулятор", &allowlist, &windows, 1_100) {
            VoiceOutcome::Action(action) => match *action {
                WindowsAction::LaunchAllowedApplication { application_id } => {
                    assert!(application_id.as_str().starts_with("app_"))
                }
                other => panic!("unexpected {other:?}"),
            },
            other => panic!("unexpected {other:?}"),
        }
        // An application that is not allowed is never started from a phrase.
        assert!(matches!(
            route("открой командную строку", &allowlist, &windows, 1_100),
            VoiceOutcome::Ambiguous { .. }
        ));
        assert!(matches!(
            route("запусти", &allowlist, &windows, 1_100),
            VoiceOutcome::Ambiguous { .. }
        ));
    }

    #[test]
    fn lock_list_and_screenshot_phrases_are_recognized() {
        let directory = tempdir().unwrap();
        let allowlist = AllowedApplications::open(directory.path());
        let windows = registry(Vec::new());
        assert_eq!(
            route("заблокируй компьютер", &allowlist, &windows, 1_100).action(),
            Some(&WindowsAction::LockWorkstation)
        );
        assert_eq!(
            route("покажи окна", &allowlist, &windows, 1_100).action(),
            Some(&WindowsAction::ListWindows)
        );
        assert_eq!(
            route("сделай скриншот", &allowlist, &windows, 1_100).action(),
            Some(&WindowsAction::TakeScreenshot {
                target: ScreenshotTarget::PrimaryMonitor
            })
        );
    }

    #[test]
    fn ordinary_sentences_are_not_actions() {
        let directory = tempdir().unwrap();
        let allowlist = AllowedApplications::open(directory.path());
        let windows = registry(Vec::new());
        for phrase in ["расскажи анекдот", "как дела", "what is the weather"]
        {
            assert_eq!(
                route(phrase, &allowlist, &windows, 1_100),
                VoiceOutcome::NotAnAction,
                "{phrase}"
            );
        }
        // This one starts with an opening verb, so it *is* an action attempt: the router says
        // it cannot decide, which is the safe answer. Nothing is guessed at, and no text ever
        // becomes a path or a command line.
        assert_eq!(
            route(
                "открой файл с паролями в vault",
                &allowlist,
                &windows,
                1_100
            ),
            VoiceOutcome::Ambiguous {
                reason: "windows-voice-application-not-allowed",
            }
        );
    }

    #[test]
    fn a_dictated_sentence_never_becomes_a_command_line() {
        let directory = tempdir().unwrap();
        let allowlist = AllowedApplications::open(directory.path());
        let windows = registry(Vec::new());
        // The phrase mentions a shell and an argument, yet only a reminder or nothing comes
        // out of the router: there is no path from text to a command.
        let outcome = route("открой cmd.exe /c format c:", &allowlist, &windows, 1_100);
        assert!(matches!(outcome, VoiceOutcome::Ambiguous { .. }));
        let outcome = route(
            "напомни через 5 минут удалить все файлы",
            &allowlist,
            &windows,
            1_100,
        );
        match outcome {
            VoiceOutcome::Action(action) => {
                // The text is reminder *content*, which is sealed and never executed.
                assert!(matches!(*action, WindowsAction::CreateReminder { .. }));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn confirm_and_cancel_words_are_recognized() {
        assert!(is_confirmation("Подтверждаю"));
        assert!(is_confirmation("подтверждаю действие"));
        assert!(is_confirmation("yes"));
        assert!(!is_confirmation("подтверди"));
        assert!(is_cancellation("Отмена"));
        assert!(is_cancellation("отмени"));
        assert!(is_cancellation("no"));
        assert!(!is_cancellation("может быть"));
    }

    #[test]
    fn the_filler_words_of_the_host_are_ignored() {
        let directory = tempdir().unwrap();
        let allowlist = AllowedApplications::open(directory.path());
        let windows = registry(Vec::new());
        assert_eq!(
            route(
                "Джарвис, пожалуйста, заблокируй компьютер",
                &allowlist,
                &windows,
                1_100
            )
            .action(),
            Some(&WindowsAction::LockWorkstation)
        );
    }

    #[test]
    fn spoken_numbers_are_understood_for_the_small_vocabulary() {
        assert_eq!(number_in_word("десять"), Some(10));
        assert_eq!(number_in_word("thirty"), Some(30));
        assert_eq!(number_in_word("10"), Some(10));
        assert_eq!(number_in_word("минут"), None);
        assert_eq!(duration_unit("минут"), Some(60));
        assert_eq!(duration_unit("часа"), Some(3600));
        assert_eq!(duration_unit("секунд"), Some(1));
        assert_eq!(duration_unit("years"), None);
    }
}

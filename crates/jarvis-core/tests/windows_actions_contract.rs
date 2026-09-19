//! The shape contract between the core and the interface.
//!
//! The interface is written in TypeScript, so a renamed field in Rust is not a compile error
//! anywhere: it is a silently empty dialog at run time. This test compares what the core
//! actually serializes with what `frontend/src/lib/windows-actions-model.ts` declares, field
//! name by field name, and fails when the two drift apart.
//!
//! It reads the interface's sources on purpose. The alternative — trusting two files to be
//! edited together — is what this test exists to replace.

use std::path::{Path, PathBuf};

use jarvis_core::windows_actions::{
    ActionPreview, ActionResult, ActionRisk, ActionSource, ActionStatus, AuditEntry, PreviewField,
    ScheduledKind, ScheduledStatus, ScheduledView, WindowState, WindowSummary,
};

fn model_source() -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate lives in a workspace");
    std::fs::read_to_string(root.join("frontend/src/lib/windows-actions-model.ts"))
        .expect("the interface model must exist")
}

/// The field names declared inside one `export interface Name { ... }` block.
fn interface_fields(source: &str, name: &str) -> Vec<String> {
    let header = format!("export interface {name} {{");
    let start = source
        .find(&header)
        .unwrap_or_else(|| panic!("the interface must declare {name}"))
        + header.len();
    let end = source[start..]
        .find("\n}")
        .map(|offset| start + offset)
        .unwrap_or_else(|| panic!("{name} must end with a closing brace"));
    let body = &source[start..end];
    let mut fields: Vec<String> = body
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('/') || trimmed.starts_with('*') {
                return None;
            }
            let (name, _) = trimmed.split_once(':')?;
            let name = name.trim();
            if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            }
        })
        .collect();
    fields.sort();
    fields
}

/// The keys of a serialized value, sorted.
fn json_keys(value: &serde_json::Value) -> Vec<String> {
    let mut keys: Vec<String> = value
        .as_object()
        .expect("the core serializes this as an object")
        .keys()
        .cloned()
        .collect();
    keys.sort();
    keys
}

fn sample_preview() -> ActionPreview {
    ActionPreview {
        token: "0123456789abcdef0123456789abcdef".to_string(),
        action_kind: "close_window".to_string(),
        risk: ActionRisk::Confirm,
        source: ActionSource::DirectGui,
        title_key: "windows-confirm-close".to_string(),
        fields: vec![PreviewField {
            label_key: "windows-field-window".to_string(),
            value: "Notepad".to_string(),
        }],
        consequences: vec!["windows-consequence-close".to_string()],
        expires_in_seconds: 45,
        cancellable: true,
    }
}

fn sample_result() -> ActionResult {
    ActionResult {
        action_id: "aabbccdd".to_string(),
        action_kind: "get_volume".to_string(),
        status: ActionStatus::Executed,
        source: ActionSource::Voice,
        value: jarvis_core::windows_actions::ActionValue::Volume {
            percent: 40,
            muted: false,
        },
        detail: None,
        duration_ms: 3,
    }
}

#[test]
fn the_preview_carries_exactly_the_fields_the_interface_declares() {
    let source = model_source();
    let sent = json_keys(&serde_json::to_value(sample_preview()).unwrap());
    assert_eq!(
        sent,
        interface_fields(&source, "ActionPreview"),
        "ActionPreview and the interface's ActionPreview have drifted apart"
    );
    // The two names that were wrong once, spelled out: a mistake here is a dialog that shows
    // nothing, so the test says which spelling is the right one.
    assert!(sent.contains(&"action_kind".to_string()));
    assert!(!sent.contains(&"action_type".to_string()));
}

#[test]
fn the_result_carries_exactly_the_fields_the_interface_declares() {
    let source = model_source();
    let sent = json_keys(&serde_json::to_value(sample_result()).unwrap());
    assert_eq!(
        sent,
        interface_fields(&source, "ActionResult"),
        "ActionResult and the interface's ActionResult have drifted apart"
    );
}

#[test]
fn a_log_entry_and_a_scheduled_view_keep_their_fields() {
    let source = model_source();
    let entry = AuditEntry {
        timestamp: "2026-01-01T00:00:00Z".to_string(),
        action_type: "set_volume".to_string(),
        source: ActionSource::DirectGui,
        risk: ActionRisk::Safe,
        decision: ActionStatus::Executed,
        result: "ok".to_string(),
        duration_ms: 1,
        error_category: None,
        target: Some("app_1".to_string()),
    };
    assert_eq!(
        json_keys(&serde_json::to_value(entry).unwrap()),
        interface_fields(&source, "AuditEntry")
    );

    let view = ScheduledView {
        id: "aabbccdd".to_string(),
        kind: ScheduledKind::Reminder,
        status: ScheduledStatus::Pending,
        source: ActionSource::Voice,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        fires_at: "2026-01-01T00:10:00Z".to_string(),
        remaining_seconds: 600,
        message: Some("позвонить".to_string()),
        message_unreadable: false,
    };
    assert_eq!(
        json_keys(&serde_json::to_value(view).unwrap()),
        interface_fields(&source, "ScheduledView")
    );
}

#[test]
fn a_window_summary_keeps_its_fields() {
    let source = model_source();
    let window = WindowSummary {
        id: "aabbccdd".to_string(),
        title: "Notepad".to_string(),
        process: "notepad.exe".to_string(),
        state: WindowState::Normal,
        monitor: 1,
        sensitive: false,
        foreground: true,
    };
    assert_eq!(
        json_keys(&serde_json::to_value(window).unwrap()),
        interface_fields(&source, "WindowSummary")
    );
}

#[test]
fn the_enums_are_tagged_the_way_the_interface_reads_them() {
    let source = model_source();
    // The tags are what a TypeScript union matches on, so a renamed tag is a union that never
    // matches and a dialog that never appears.
    let action = jarvis_core::windows_actions::WindowsAction::GetVolume;
    let action = serde_json::to_value(action).unwrap();
    assert!(
        action.get("action").is_some(),
        "WindowsAction is tagged with `action`"
    );
    assert!(source.contains("action: \"get_volume\""));

    let value = serde_json::to_value(jarvis_core::windows_actions::ActionValue::Locked).unwrap();
    assert!(
        value.get("value").is_some(),
        "ActionValue is tagged with `value`"
    );
    assert!(source.contains("{ value: \"locked\" }"));

    // The outcome and the voice route are declared as unions of objects with these tags.
    assert!(source.contains("outcome: \"executed\""));
    assert!(source.contains("outcome: \"awaiting_confirmation\""));
    assert!(source.contains("outcome: \"rejected\""));
    assert!(source.contains("result: \"requested\""));
    assert!(source.contains("result: \"ambiguous\""));
    assert!(source.contains("result: \"not_an_action\""));
    assert!(source.contains("result: \"disabled\""));

    // Every risk and status the core can produce has a name in the model.
    for status in [
        ActionStatus::Requested,
        ActionStatus::Confirmed,
        ActionStatus::Cancelled,
        ActionStatus::Expired,
        ActionStatus::Executed,
        ActionStatus::Rejected,
        ActionStatus::Failed,
    ] {
        assert!(
            source.contains(&format!("\"{}\"", status.as_str())),
            "the interface must know the status {}",
            status.as_str()
        );
    }
    for risk in [ActionRisk::Safe, ActionRisk::Confirm, ActionRisk::Forbidden] {
        let serialized = serde_json::to_value(risk).unwrap();
        let name = serialized.as_str().unwrap().to_string();
        assert!(
            source.contains(&format!("\"{name}\"")),
            "the interface must know the risk {name}"
        );
    }
    for source in [
        ActionSource::DirectGui,
        ActionSource::Voice,
        ActionSource::LocalAi,
        ActionSource::InternalTimer,
    ] {
        let serialized = serde_json::to_value(source).unwrap();
        let name = serialized.as_str().unwrap().to_string();
        assert!(
            source_file_contains(&name),
            "the interface must know the action source {name}"
        );
    }
}

fn source_file_contains(name: &str) -> bool {
    model_source().contains(&format!("\"{name}\""))
}

#[test]
fn every_locale_translates_every_message_the_dialog_and_the_panel_use() {
    // The three Fluent files are compiled into the binary. A parse error, a duplicated key, or
    // a key that only one language has would come back here as the raw key, which is exactly
    // what a user would see.
    let english = model_keys();
    assert!(
        english.len() >= 200,
        "expected the whole key set, got {}",
        english.len()
    );
    for language in ["en", "ru", "ua"] {
        jarvis_core::i18n::init(language);
        jarvis_core::i18n::set_language(language);
        assert_eq!(jarvis_core::i18n::get_language(), language);
        for key in &english {
            let translated = jarvis_core::i18n::t(key);
            assert_ne!(&translated, key, "{language} cannot translate {key}");
            assert!(
                !translated.trim().is_empty(),
                "{language} shows {key} as empty"
            );
        }
        // A message that carries a variable is formatted by the core, and the frontend has no
        // arguments to pass: the two keys the interface renders itself must have no placeholder.
        for key in ["windows-actions-volume", "windows-actions-confirm-expires"] {
            if english.contains(&key.to_string()) {
                assert!(
                    !jarvis_core::i18n::t(key).contains('{'),
                    "{key} must not carry a placeholder the interface cannot fill"
                );
            }
        }
    }
}

/// Every message key of the feature, taken from the English file.
fn model_keys() -> Vec<String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate lives in a workspace");
    let source = std::fs::read_to_string(root.join("crates/jarvis-core/src/i18n/locales/en.ftl"))
        .expect("the English locale must exist");
    let mut keys: Vec<String> = source
        .lines()
        .filter_map(|line| {
            let (key, _) = line.split_once(" = ")?;
            let key = key.trim();
            let known = key.starts_with("windows-actions-")
                || key.starts_with("windows-field-")
                || key.starts_with("windows-confirm-")
                || key.starts_with("windows-consequence-");
            known.then(|| key.to_string())
        })
        .collect();
    keys.sort();
    keys.dedup();
    keys
}

#[test]
fn the_locales_have_the_same_keys() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate lives in a workspace");
    let english = model_keys();
    for language in ["ru", "ua"] {
        let source = std::fs::read_to_string(root.join(format!(
            "crates/jarvis-core/src/i18n/locales/{language}.ftl"
        )))
        .expect("the locale must exist");
        let mut keys: Vec<String> = source
            .lines()
            .filter_map(|line| {
                let (key, _) = line.split_once(" = ")?;
                let key = key.trim();
                let known = key.starts_with("windows-actions-")
                    || key.starts_with("windows-field-")
                    || key.starts_with("windows-confirm-")
                    || key.starts_with("windows-consequence-");
                known.then(|| key.to_string())
            })
            .collect();
        keys.sort();
        keys.dedup();
        assert_eq!(
            keys, english,
            "{language}.ftl and en.ftl must have the same keys"
        );
    }
}
#[test]
fn the_command_names_the_interface_calls_all_exist() {
    // The wrapper file names the commands; each one must be registered in the GUI's handler
    // list, or the interface would call into nothing.
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate lives in a workspace");
    let wrapper = std::fs::read_to_string(root.join("frontend/src/lib/windows-actions.ts"))
        .expect("the wrapper must exist");
    let main = std::fs::read_to_string(root.join("crates/jarvis-gui/src/main.rs"))
        .expect("the GUI entry point must exist");
    let mut checked = 0usize;
    for line in wrapper.lines() {
        let Some(start) = line.find("invoke<") else {
            continue;
        };
        let rest = &line[start..];
        let Some(open) = rest.find('"') else { continue };
        let Some(close) = rest[open + 1..].find('"') else {
            continue;
        };
        let command = &rest[open + 1..open + 1 + close];
        assert!(
            command.starts_with("windows_actions_"),
            "unexpected command {command} in the windows-actions wrapper"
        );
        assert!(
            main.contains(command),
            "{command} is called by the interface but not registered in main.rs"
        );
        checked += 1;
    }
    assert!(
        checked >= 15,
        "expected the whole command surface, saw {checked}"
    );
}

/// The path of the interface model, for the diagnostics of a failing check.
fn _model_path() -> PathBuf {
    PathBuf::from("frontend/src/lib/windows-actions-model.ts")
}

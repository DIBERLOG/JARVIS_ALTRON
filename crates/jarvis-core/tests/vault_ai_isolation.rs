//! Architectural boundary: nothing on the AI, voice, or scripting surface may
//! reach the password vault.
//!
//! The vault is a Rust-only capability:
//!
//! * `ChatProvider` and the AI layer never receive a `VaultStore` or a
//!   `VaultSession`; they only see the conversation contracts in `src/ai`;
//! * voice commands come from the static catalogue in `resources/commands`, and
//!   that catalogue contains no vault entry, so no phrase can list or reveal a
//!   password;
//! * Lua scripting (`src/lua`) has its own API surface and no vault handle;
//! * the DTOs that travel over IPC for lists and details cannot carry a password
//!   because the types do not have such a field.
//!
//! These checks are deliberately source-level and structural: they fail if a
//! future change leaks a vault reference into a surface that must not have one.

use std::fs;
use std::path::{Path, PathBuf};

use jarvis_core::vault::{
    SecretRevealResult, VaultItemDetails, VaultItemSummary, VAULT_PAYLOAD_SCHEMA_VERSION,
};

/// Source directories that must never mention the vault.
const FORBIDDEN_SURFACES: &[&str] = &[
    "src/ai",
    "src/lua",
    "src/slots",
    "src/intent",
    "src/listener",
    "src/stt",
    "src/commands.rs",
];

/// Identifiers that only vault-aware code may use.
const FORBIDDEN_IDENTIFIERS: &[&str] = &[
    "vault",
    "VaultStore",
    "VaultSession",
    "VaultItemPayload",
    "reveal_secret",
    "password_for_clipboard",
    "JARVIS/vault",
    "vault.sqlite3",
];

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn collect_rust_files(path: &Path, collected: &mut Vec<PathBuf>) {
    if path.is_file() {
        if path.extension().map(|value| value == "rs").unwrap_or(false) {
            collected.push(path.to_path_buf());
        }
        return;
    }
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        collect_rust_files(&entry.path(), collected);
    }
}

#[test]
fn ai_voice_and_scripting_sources_never_reference_the_vault() {
    let root = manifest_dir();
    let mut files: Vec<PathBuf> = Vec::new();
    for surface in FORBIDDEN_SURFACES {
        let path = root.join(surface);
        assert!(
            path.exists(),
            "expected the surface {surface} to exist; the scan would be meaningless otherwise"
        );
        collect_rust_files(&path, &mut files);
    }
    // A wrong path must not silently pass the scan.
    assert!(
        files.len() >= 20,
        "expected to scan the AI/voice surface, found only {} files",
        files.len()
    );

    let mut violations: Vec<String> = Vec::new();
    for file in &files {
        let contents = fs::read_to_string(file).unwrap_or_default();
        let lowered = contents.to_lowercase();
        for identifier in FORBIDDEN_IDENTIFIERS {
            if lowered.contains(&identifier.to_lowercase()) {
                violations.push(format!(
                    "{} references {identifier}",
                    file.strip_prefix(&root).unwrap_or(file).display()
                ));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "the vault must stay unreachable from AI, voice, and scripting surfaces:\n{}",
        violations.join("\n")
    );
}

#[test]
fn the_voice_command_catalogue_has_no_vault_commands() {
    let catalogue = manifest_dir().join("..").join("..").join("resources").join("commands");
    let mut files: Vec<PathBuf> = Vec::new();
    collect_all_files(&catalogue, &mut files);
    assert!(
        !files.is_empty(),
        "the command catalogue should contain the voice command definitions"
    );

    let mut violations: Vec<String> = Vec::new();
    for file in &files {
        let Ok(contents) = fs::read_to_string(file) else {
            continue;
        };
        let lowered = contents.to_lowercase();
        for needle in ["vault", "password", "пароль", "парол"] {
            if lowered.contains(needle) {
                violations.push(format!(
                    "{} contains '{}'",
                    file.file_name().unwrap_or_default().to_string_lossy(),
                    needle
                ));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "voice commands must not be able to touch the vault:\n{}",
        violations.join("\n")
    );
}

fn collect_all_files(path: &Path, collected: &mut Vec<PathBuf>) {
    if path.is_file() {
        collected.push(path.to_path_buf());
        return;
    }
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        collect_all_files(&entry.path(), collected);
    }
}

/// A payload sent to the interface for browsing may carry a flag about a
/// password, but never a password or free-form secret notes.
fn assert_no_secret_field(payload: &serde_json::Value, context: &str) {
    let object = payload
        .as_object()
        .unwrap_or_else(|| panic!("{context} must serialize to an object"));
    for (key, field) in object {
        let lowered = key.to_lowercase();
        if lowered.contains("password") {
            assert!(
                field.is_boolean(),
                "{context}: '{key}' must be a flag, not secret material"
            );
        }
        assert_ne!(
            lowered, "notes",
            "{context}: free-form notes must not travel with a browsing payload"
        );
        if let Some(text) = field.as_str() {
            assert_ne!(
                text, "FICTIONAL_ITEM_PASSWORD",
                "{context}: a secret leaked into '{key}'"
            );
        }
    }
}

#[test]
fn list_and_detail_payloads_cannot_carry_a_password() {
    let summary = VaultItemSummary {
        id: uuid::Uuid::new_v4(),
        revision: 3,
        name: "FICTIONAL_ITEM_NAME".to_string(),
        username: "FICTIONAL_ITEM_USERNAME".to_string(),
        url_host: Some("fictional.example.com".to_string()),
        tags: vec!["FICTIONAL_TAG".to_string()],
        favorite: true,
        has_password: true,
        created_at: "2026-01-01T00:00:00+00:00".to_string(),
        updated_at: "2026-01-01T00:00:00+00:00".to_string(),
        deleted_at: None,
    };
    let details = VaultItemDetails {
        id: summary.id,
        revision: summary.revision,
        name: summary.name.clone(),
        username: summary.username.clone(),
        urls: vec!["https://fictional.example.com".to_string()],
        tags: summary.tags.clone(),
        favorite: summary.favorite,
        created_at: summary.created_at.clone(),
        updated_at: summary.updated_at.clone(),
        deleted_at: None,
    };

    for payload in [
        serde_json::to_value(&summary).unwrap(),
        serde_json::to_value(&details).unwrap(),
    ] {
        assert_no_secret_field(&payload, "list and detail payloads");
    }

    // The explicit reveal result is the only carrier, and it reports which
    // payload version it belongs to through the item revision.
    let revealed = SecretRevealResult {
        id: summary.id,
        revision: summary.revision,
        password: "FICTIONAL_ITEM_PASSWORD".to_string(),
        notes: "FICTIONAL_ITEM_NOTES".to_string(),
        reveal_timeout_seconds: 30,
    };
    let rendered = serde_json::to_value(&revealed).unwrap().to_string();
    assert!(rendered.contains("FICTIONAL_ITEM_PASSWORD"));
    assert_eq!(VAULT_PAYLOAD_SCHEMA_VERSION, 1);
    // Diagnostics still redact it.
    assert!(!format!("{revealed:?}").contains("FICTIONAL_ITEM_PASSWORD"));
    assert!(!format!("{summary:?}").contains("FICTIONAL_ITEM_NAME"));
    assert!(!format!("{details:?}").contains("FICTIONAL_ITEM_USERNAME"));
}

#[test]
fn the_ai_contract_module_does_not_mention_secrets() {
    let provider = manifest_dir().join("src").join("ai").join("provider.rs");
    let contents = fs::read_to_string(&provider).expect("the AI provider contract must exist");
    let lowered = contents.to_lowercase();
    for needle in ["vault", "password", "secret"] {
        assert!(
            !lowered.contains(needle),
            "the AI contract must not mention {needle}"
        );
    }
}

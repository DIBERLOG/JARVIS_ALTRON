//! Unit tests for the password vault: store behavior, generator, clipboard
//! timing, and the domain separation between vault and notes records.
//!
//! Every password and item in this file is fictional.

use super::clipboard::{
    clamp_clear_seconds, ClipboardBackend, ClipboardError, ClipboardGuard, ClipboardOutcome,
    MAX_CLEAR_SECONDS, MIN_CLEAR_SECONDS,
};
use super::generator::{uniform_index_from, PasswordPolicy};
use super::model::*;
use super::session::{
    normalize_timeout, IdleLock, DEFAULT_IDLE_TIMEOUT_SECONDS, IDLE_TIMEOUT_OPTIONS,
};
use super::store::VaultStore;
use crate::sync::crypto::{random_master_key, KeyPurpose, MasterKey, PurposeKeyProvider};
use crate::sync::{
    ApplyOutcome, CryptoProvider, DeviceId, InMemorySyncRepository, PayloadContext,
    SyncEntityType, SyncMutation, SyncOperationKind, SyncRepository,
};
use std::time::{Duration, Instant};
use uuid::Uuid;

type TestStore = VaultStore<InMemorySyncRepository, PurposeKeyProvider>;

fn store() -> TestStore {
    let master = random_master_key().unwrap();
    let provider = PurposeKeyProvider::derive(&master, KeyPurpose::Vault).unwrap();
    VaultStore::new(
        InMemorySyncRepository::new(),
        provider,
        DeviceId::new("vault_test_device").unwrap(),
    )
}

fn draft(name: &str, password: &str) -> VaultItemDraft {
    VaultItemDraft {
        name: name.to_string(),
        username: "fictional-user".to_string(),
        password: password.to_string(),
        urls: vec!["https://fictional.example.com/login?token=hidden".to_string()],
        notes: "FICTIONAL_ITEM_NOTES".to_string(),
        tags: vec!["work".to_string()],
        favorite: false,
    }
}

/// In-memory clipboard so the timing rules can be tested without a desktop.
///
/// A shared state handle lets a test play the part of the user copying their own
/// text between two guard calls.
#[derive(Default)]
struct FakeState {
    value: Option<String>,
    writes: Vec<String>,
    clears: usize,
    fail_write: bool,
    fail_read: bool,
}

#[derive(Clone, Default)]
struct FakeClipboard {
    state: std::sync::Arc<std::sync::Mutex<FakeState>>,
}

impl FakeClipboard {
    fn value(&self) -> Option<String> {
        self.state.lock().unwrap().value.clone()
    }

    fn writes(&self) -> Vec<String> {
        self.state.lock().unwrap().writes.clone()
    }

    fn clears(&self) -> usize {
        self.state.lock().unwrap().clears
    }

    /// Plays the part of the user copying unrelated text.
    fn user_copies(&self, text: &str) {
        self.state.lock().unwrap().value = Some(text.to_string());
    }

    fn fail_write(&self) {
        self.state.lock().unwrap().fail_write = true;
    }

    fn fail_read(&self) {
        self.state.lock().unwrap().fail_read = true;
    }
}

impl ClipboardBackend for FakeClipboard {
    fn read_text(&mut self) -> Result<Option<String>, ClipboardError> {
        let state = self.state.lock().unwrap();
        if state.fail_read {
            return Err(ClipboardError::Unavailable);
        }
        Ok(state.value.clone())
    }

    fn write_text(&mut self, value: &str) -> Result<(), ClipboardError> {
        let mut state = self.state.lock().unwrap();
        if state.fail_write {
            return Err(ClipboardError::Unavailable);
        }
        state.value = Some(value.to_string());
        state.writes.push(value.to_string());
        Ok(())
    }

    fn clear(&mut self) -> Result<(), ClipboardError> {
        let mut state = self.state.lock().unwrap();
        state.clears += 1;
        state.value = None;
        Ok(())
    }
}

// ------------------------------------------------------------------- storage

#[test]
fn creates_reads_and_updates_an_item_without_leaking_the_password() {
    let mut store = store();
    let created = store
        .create_item(&draft("FICTIONAL_LOGIN", "FICTIONAL_PASSWORD_ONE"))
        .unwrap();
    assert_eq!(created.revision, 1);
    assert_eq!(created.name, "FICTIONAL_LOGIN");
    // The details type has no password field at all.
    let details_debug = format!("{created:?}");
    assert!(!details_debug.contains("FICTIONAL_PASSWORD_ONE"));

    let revealed = store
        .reveal_secret(created.id, 30)
        .unwrap();
    assert_eq!(revealed.password, "FICTIONAL_PASSWORD_ONE");
    assert_eq!(revealed.notes, "FICTIONAL_ITEM_NOTES");
    // The reveal type redacts in diagnostics.
    assert!(!format!("{revealed:?}").contains("FICTIONAL_PASSWORD_ONE"));

    let updated = store
        .update_item(created.id, &draft("FICTIONAL_LOGIN", "FICTIONAL_PASSWORD_TWO"))
        .unwrap();
    assert_eq!(updated.revision, 2);
    assert_eq!(
        store.reveal_secret(created.id, 30).unwrap().password,
        "FICTIONAL_PASSWORD_TWO"
    );
}

#[test]
fn summaries_never_contain_the_password_and_expose_only_the_url_host() {
    let mut store = store();
    store
        .create_item(&draft("FICTIONAL_LOGIN", "FICTIONAL_PASSWORD_ONE"))
        .unwrap();
    let list = store.list_items(&VaultQuery::default()).unwrap();
    assert_eq!(list.items.len(), 1);
    let summary = &list.items[0];
    assert_eq!(summary.url_host.as_deref(), Some("fictional.example.com"));
    assert!(summary.has_password);
    // The query token in the URL path must not be part of the summary.
    let rendered = format!("{summary:?} {}", summary.url_host.clone().unwrap_or_default());
    assert!(!rendered.contains("token=hidden"));
    assert!(!rendered.contains("FICTIONAL_PASSWORD_ONE"));
}

#[test]
fn favorite_trash_restore_and_purge_behave_as_expected() {
    let mut store = store();
    let item = store
        .create_item(&draft("FICTIONAL_LOGIN", "FICTIONAL_PASSWORD_ONE"))
        .unwrap();

    let favorite = store.set_favorite(item.id, true).unwrap();
    assert!(favorite.favorite);
    assert_eq!(store.stats().unwrap().favorites, 1);

    let trashed = store.trash_item(item.id).unwrap();
    assert!(trashed.deleted_at.is_some());
    assert_eq!(
        store
            .list_items(&VaultQuery {
                trash: VaultTrashFilter::Trashed,
                ..VaultQuery::default()
            })
            .unwrap()
            .items
            .len(),
        1
    );
    assert!(store
        .list_items(&VaultQuery::default())
        .unwrap()
        .items
        .is_empty());

    let restored = store.restore_item(item.id).unwrap();
    assert!(restored.deleted_at.is_none());

    store.purge_item(item.id).unwrap();
    assert!(store.get_item(item.id).unwrap().is_none());
    assert_eq!(
        store.update_item(item.id, &draft("x", "y")),
        Err(VaultError::Deleted)
    );
    // Nothing is left in the trash either.
    assert!(store
        .list_items(&VaultQuery {
            trash: VaultTrashFilter::All,
            ..VaultQuery::default()
        })
        .unwrap()
        .items
        .is_empty());
}

#[test]
fn search_covers_name_username_urls_tags_and_notes() {
    let mut store = store();
    store
        .create_item(&draft("Bank account", "FICTIONAL_PASSWORD_ONE"))
        .unwrap();
    let mut other = draft("Mail", "FICTIONAL_PASSWORD_TWO");
    other.tags = vec!["personal".into()];
    other.urls = vec!["https://mail.fictional.example".into()];
    store.create_item(&other).unwrap();

    let count = |store: &mut TestStore, needle: &str| -> usize {
        store
            .list_items(&VaultQuery {
                search: needle.to_string(),
                ..VaultQuery::default()
            })
            .unwrap()
            .items
            .len()
    };
    assert_eq!(count(&mut store, "BANK"), 1);
    assert_eq!(count(&mut store, "fictional-user"), 2);
    assert_eq!(count(&mut store, "personal"), 1);
    assert_eq!(count(&mut store, "mail.fictional"), 1);
    assert_eq!(count(&mut store, "FICTIONAL_ITEM_NOTES"), 2);
    assert_eq!(count(&mut store, "missing"), 0);
    // A password is never part of the search index.
    assert_eq!(count(&mut store, "FICTIONAL_PASSWORD_ONE"), 0);

    // Searching must never match a password, because the plaintext query is only
    // compared against non-secret display fields plus notes.
    let tagged = store
        .list_items(&VaultQuery {
            tag: Some("PERSONAL".into()),
            ..VaultQuery::default()
        })
        .unwrap();
    assert_eq!(tagged.items.len(), 1);
    assert_eq!(
        store.all_tags().unwrap(),
        vec!["personal".to_string(), "work".to_string()]
    );
    assert_eq!(store.stats().unwrap().items_total, 2);
}

#[test]
fn favorites_filter_and_sorting_are_stable() {
    let mut store = store();
    let first = store
        .create_item(&draft("Alpha", "FICTIONAL_PASSWORD_ONE"))
        .unwrap();
    store
        .create_item(&draft("Beta", "FICTIONAL_PASSWORD_TWO"))
        .unwrap();
    store.set_favorite(first.id, true).unwrap();

    let favorites = store
        .list_items(&VaultQuery {
            favorites_only: true,
            ..VaultQuery::default()
        })
        .unwrap();
    assert_eq!(favorites.items.len(), 1);
    assert_eq!(favorites.items[0].id, first.id);

    let by_name = store
        .list_items(&VaultQuery {
            sort: VaultSort::NameAsc,
            ..VaultQuery::default()
        })
        .unwrap();
    assert_eq!(by_name.items[0].name, "Alpha");
    assert_eq!(by_name.items[1].name, "Beta");
}

#[test]
fn debug_output_never_contains_item_content() {
    let mut store = store();
    let item = store
        .create_item(&draft("FICTIONAL_LOGIN", "FICTIONAL_PASSWORD_ONE"))
        .unwrap();
    let draft_value = draft("FICTIONAL_LOGIN", "FICTIONAL_PASSWORD_ONE");
    let list = store.list_items(&VaultQuery::default()).unwrap();
    let payload = store.decrypt_item_set().unwrap().remove(0);
    let rendered = format!("{item:?} {draft_value:?} {list:?} {payload:?}");
    assert!(!rendered.contains("FICTIONAL_LOGIN"));
    assert!(!rendered.contains("FICTIONAL_PASSWORD_ONE"));
    assert!(!rendered.contains("FICTIONAL_ITEM_NOTES"));
    assert!(rendered.contains("<redacted>"));
}

#[test]
fn items_survive_a_store_restart_and_a_foreign_key_cannot_read_them() {
    let device = DeviceId::new("vault_test_device").unwrap();
    let master = random_master_key().unwrap();
    let same_key = MasterKey::from_bytes(*master.as_array());

    let mut first = VaultStore::new(
        InMemorySyncRepository::new(),
        PurposeKeyProvider::derive(&master, KeyPurpose::Vault).unwrap(),
        device.clone(),
    );
    let item = first
        .create_item(&draft("FICTIONAL_LOGIN", "FICTIONAL_PASSWORD_ONE"))
        .unwrap();
    let repository = first.into_repository();

    // Same master key again: the item opens.
    let mut reopened = VaultStore::new(
        repository,
        PurposeKeyProvider::derive(&same_key, KeyPurpose::Vault).unwrap(),
        device.clone(),
    );
    assert_eq!(
        reopened.reveal_secret(item.id, 30).unwrap().password,
        "FICTIONAL_PASSWORD_ONE"
    );
    let repository = reopened.into_repository();

    // A different master key fails closed and is reported, not hidden.
    let mut foreign = VaultStore::new(
        repository,
        PurposeKeyProvider::derive(&random_master_key().unwrap(), KeyPurpose::Vault).unwrap(),
        device,
    );
    assert_eq!(foreign.get_item(item.id), Err(VaultError::Unreadable));
    let list = foreign
        .list_items(&VaultQuery {
            trash: VaultTrashFilter::All,
            ..VaultQuery::default()
        })
        .unwrap();
    assert!(list.items.is_empty());
    assert_eq!(list.unreadable, 1);
}

#[test]
fn a_note_record_cannot_be_read_as_a_vault_item_and_the_other_way_round() {
    let master = random_master_key().unwrap();
    let notes_provider =
        crate::sync::crypto::MasterKeyCryptoProvider::new(MasterKey::from_bytes(*master.as_array()));
    let vault_provider = PurposeKeyProvider::derive(&master, KeyPurpose::Vault).unwrap();

    // A vault record written with the vault key.
    let vault_context = PayloadContext::new(SyncEntityType::VaultRecord, Uuid::new_v4());
    let vault_payload = vault_provider
        .encrypt(&vault_context, b"FICTIONAL_VAULT_SECRET")
        .unwrap();

    // The notes provider cannot read it, and neither can a notes-domain store.
    assert!(notes_provider.decrypt(&vault_context, &vault_payload).is_err());

    // A note record written with the notes key cannot be read by the vault.
    let note_context = PayloadContext::new(SyncEntityType::Note, Uuid::new_v4());
    let note_payload = notes_provider
        .encrypt(&note_context, b"FICTIONAL_NOTE_SECRET")
        .unwrap();
    assert!(vault_provider.decrypt(&note_context, &note_payload).is_err());

    // Even in the same entity type the keys differ, so cross-reading fails.
    let same_type_context = PayloadContext::new(SyncEntityType::VaultRecord, Uuid::new_v4());
    let vault_payload_two = vault_provider
        .encrypt(&same_type_context, b"FICTIONAL_VAULT_SECRET")
        .unwrap();
    let other_vault = PurposeKeyProvider::derive(
        &random_master_key().unwrap(),
        KeyPurpose::Vault,
    )
    .unwrap();
    assert!(other_vault
        .decrypt(&same_type_context, &vault_payload_two)
        .is_err());

    // The vault provider is the vault domain, and it never exposes the master key.
    assert_eq!(vault_provider.purpose(), KeyPurpose::Vault);
}

#[test]
fn conflicts_offer_all_three_resolutions() {
    let master = random_master_key().unwrap();
    let device = DeviceId::new("vault_test_device").unwrap();
    let provider = PurposeKeyProvider::derive(&master, KeyPurpose::Vault).unwrap();
    let mut store = VaultStore::new(InMemorySyncRepository::new(), provider, device);

    let item = store
        .create_item(&draft("current name", "FICTIONAL_PASSWORD_ONE"))
        .unwrap();

    // A stale second version arrives for the same entity.
    let twin = PurposeKeyProvider::derive(
        &MasterKey::from_bytes(*master.as_array()),
        KeyPurpose::Vault,
    )
    .unwrap();
    let conflict_id = {
        let mut incoming = draft("incoming name", "FICTIONAL_PASSWORD_TWO");
        incoming.favorite = true;
        let payload = VaultItemPayload::create(&incoming).unwrap();
        let context = PayloadContext::new(SyncEntityType::VaultRecord, item.id);
        let encrypted = twin
            .encrypt(&context, &payload.to_bytes().unwrap())
            .unwrap();
        let mutation = SyncMutation {
            operation_id: Uuid::new_v4(),
            entity_id: item.id,
            entity_type: SyncEntityType::VaultRecord,
            device_id: DeviceId::new("other_device").unwrap(),
            device_sequence: 1,
            base_revision: 0,
            kind: SyncOperationKind::Update,
            timestamp: chrono::Utc::now().to_rfc3339(),
            schema_version: SyncMutation::CURRENT_SCHEMA_VERSION,
            encrypted_payload: Some(encrypted),
        };
        match store.repository_mut().apply_mutation(mutation).unwrap() {
            ApplyOutcome::Conflict { conflict_id } => conflict_id,
            other => panic!("expected a conflict, got {other:?}"),
        }
    };

    let view = store.conflicts().unwrap();
    assert_eq!(view.len(), 1);
    assert_eq!(view[0].current_name.as_deref(), Some("current name"));
    assert_eq!(view[0].incoming_name.as_deref(), Some("incoming name"));
    // A conflict listing never carries a password.
    assert!(!format!("{view:?}").contains("FICTIONAL_PASSWORD"));
    assert_eq!(
        store.get_item(item.id).unwrap().unwrap().name,
        "current name"
    );

    let outcome = store
        .resolve_conflict(conflict_id, VaultConflictResolution::AcceptIncoming)
        .unwrap();
    assert_eq!(outcome.updated_entity_id, Some(item.id));
    assert!(store.conflicts().unwrap().is_empty());
    let details = store.get_item(item.id).unwrap().unwrap();
    assert_eq!(details.name, "incoming name");
    assert!(details.favorite);
    assert_eq!(
        store.reveal_secret(item.id, 30).unwrap().password,
        "FICTIONAL_PASSWORD_TWO"
    );
}

#[test]
fn keep_both_stores_the_incoming_version_as_a_second_item() {
    let master = random_master_key().unwrap();
    let device = DeviceId::new("vault_test_device").unwrap();
    let provider = PurposeKeyProvider::derive(&master, KeyPurpose::Vault).unwrap();
    let mut store = VaultStore::new(InMemorySyncRepository::new(), provider, device);
    let item = store
        .create_item(&draft("current name", "FICTIONAL_PASSWORD_ONE"))
        .unwrap();

    let twin = PurposeKeyProvider::derive(
        &MasterKey::from_bytes(*master.as_array()),
        KeyPurpose::Vault,
    )
    .unwrap();
    let payload = VaultItemPayload::create(&draft("incoming name", "FICTIONAL_PASSWORD_TWO"))
        .unwrap();
    let context = PayloadContext::new(SyncEntityType::VaultRecord, item.id);
    let encrypted = twin
        .encrypt(&context, &payload.to_bytes().unwrap())
        .unwrap();
    let mutation = SyncMutation {
        operation_id: Uuid::new_v4(),
        entity_id: item.id,
        entity_type: SyncEntityType::VaultRecord,
        device_id: DeviceId::new("other_device").unwrap(),
        device_sequence: 1,
        base_revision: 0,
        kind: SyncOperationKind::Update,
        timestamp: chrono::Utc::now().to_rfc3339(),
        schema_version: SyncMutation::CURRENT_SCHEMA_VERSION,
        encrypted_payload: Some(encrypted),
    };
    let conflict_id = match store.repository_mut().apply_mutation(mutation).unwrap() {
        ApplyOutcome::Conflict { conflict_id } => conflict_id,
        other => panic!("expected a conflict, got {other:?}"),
    };

    let outcome = store
        .resolve_conflict(conflict_id, VaultConflictResolution::KeepBoth)
        .unwrap();
    let created = outcome.created_entity_id.unwrap();
    assert_ne!(created, item.id);
    assert!(store.conflicts().unwrap().is_empty());
    let list = store
        .list_items(&VaultQuery {
            sort: VaultSort::NameAsc,
            ..VaultQuery::default()
        })
        .unwrap();
    assert_eq!(list.items.len(), 2);
    assert_eq!(
        store.reveal_secret(created, 30).unwrap().password,
        "FICTIONAL_PASSWORD_TWO"
    );
    assert_eq!(
        store.reveal_secret(item.id, 30).unwrap().password,
        "FICTIONAL_PASSWORD_ONE"
    );
}

#[test]
fn keep_current_leaves_the_item_untouched() {
    let master = random_master_key().unwrap();
    let device = DeviceId::new("vault_test_device").unwrap();
    let provider = PurposeKeyProvider::derive(&master, KeyPurpose::Vault).unwrap();
    let mut store = VaultStore::new(InMemorySyncRepository::new(), provider, device);
    let item = store
        .create_item(&draft("current name", "FICTIONAL_PASSWORD_ONE"))
        .unwrap();

    let twin = PurposeKeyProvider::derive(
        &MasterKey::from_bytes(*master.as_array()),
        KeyPurpose::Vault,
    )
    .unwrap();
    let payload =
        VaultItemPayload::create(&draft("incoming name", "FICTIONAL_PASSWORD_TWO")).unwrap();
    let context = PayloadContext::new(SyncEntityType::VaultRecord, item.id);
    let encrypted = twin
        .encrypt(&context, &payload.to_bytes().unwrap())
        .unwrap();
    let mutation = SyncMutation {
        operation_id: Uuid::new_v4(),
        entity_id: item.id,
        entity_type: SyncEntityType::VaultRecord,
        device_id: DeviceId::new("other_device").unwrap(),
        device_sequence: 1,
        base_revision: 0,
        kind: SyncOperationKind::Update,
        timestamp: chrono::Utc::now().to_rfc3339(),
        schema_version: SyncMutation::CURRENT_SCHEMA_VERSION,
        encrypted_payload: Some(encrypted),
    };
    let conflict_id = match store.repository_mut().apply_mutation(mutation).unwrap() {
        ApplyOutcome::Conflict { conflict_id } => conflict_id,
        other => panic!("expected a conflict, got {other:?}"),
    };

    store
        .resolve_conflict(conflict_id, VaultConflictResolution::KeepCurrent)
        .unwrap();
    assert!(store.conflicts().unwrap().is_empty());
    assert_eq!(
        store.get_item(item.id).unwrap().unwrap().name,
        "current name"
    );
    assert_eq!(
        store.reveal_secret(item.id, 30).unwrap().password,
        "FICTIONAL_PASSWORD_ONE"
    );
}

#[test]
fn a_metadata_edit_never_erases_the_stored_secret() {
    let mut store = store();
    let item = store
        .create_item(&draft("FICTIONAL_LOGIN", "FICTIONAL_PASSWORD_ONE"))
        .unwrap();

    // Rename, retag, and favourite without ever revealing the secret.
    let metadata = VaultMetadataDraft {
        name: "renamed".to_string(),
        username: "renamed-user".to_string(),
        urls: vec!["https://renamed.example.com".to_string()],
        tags: vec!["renamed-tag".to_string()],
        favorite: true,
    };
    let updated = store.update_metadata(item.id, &metadata).unwrap();
    assert_eq!(updated.name, "renamed");
    assert!(updated.favorite);
    assert_eq!(updated.revision, 2);

    // The secret is intact and unchanged.
    let revealed = store.reveal_secret(item.id, 30).unwrap();
    assert_eq!(revealed.password, "FICTIONAL_PASSWORD_ONE");
    assert_eq!(revealed.notes, "FICTIONAL_ITEM_NOTES");

    // Only an explicit secret update replaces it.
    store
        .update_secrets(item.id, "FICTIONAL_PASSWORD_TWO", "new notes")
        .unwrap();
    let revealed = store.reveal_secret(item.id, 30).unwrap();
    assert_eq!(revealed.password, "FICTIONAL_PASSWORD_TWO");
    assert_eq!(revealed.notes, "new notes");
    assert_eq!(
        store.get_item(item.id).unwrap().unwrap().name,
        "renamed",
        "a secret update must not disturb the metadata"
    );

    // A metadata edit is validated like any other write.
    let mut bad = metadata;
    bad.name = "n".repeat(MAX_NAME_CHARS + 1);
    assert_eq!(
        store.update_metadata(item.id, &bad),
        Err(VaultError::NameTooLong)
    );
    assert_eq!(
        store.reveal_secret(item.id, 30).unwrap().password,
        "FICTIONAL_PASSWORD_TWO"
    );
}

#[test]
fn malformed_drafts_are_rejected() {
    let mut store = store();
    let mut long_name = draft("ok", "FICTIONAL_PASSWORD_ONE");
    long_name.name = "n".repeat(MAX_NAME_CHARS + 1);
    assert_eq!(store.create_item(&long_name), Err(VaultError::NameTooLong));

    let mut newline_username = draft("ok", "FICTIONAL_PASSWORD_ONE");
    newline_username.username = "bad\nuser".into();
    assert_eq!(
        store.create_item(&newline_username),
        Err(VaultError::UsernameHasControlCharacters)
    );

    let mut long_password = draft("ok", "FICTIONAL_PASSWORD_ONE");
    long_password.password = "p".repeat(MAX_PASSWORD_BYTES + 1);
    assert_eq!(
        store.create_item(&long_password),
        Err(VaultError::PasswordTooLong)
    );

    let mut many_urls = draft("ok", "FICTIONAL_PASSWORD_ONE");
    many_urls.urls = (0..MAX_URLS + 1)
        .map(|index| format!("https://fictional.example/{index}"))
        .collect();
    assert_eq!(store.create_item(&many_urls), Err(VaultError::TooManyUrls));

    let mut many_tags = draft("ok", "FICTIONAL_PASSWORD_ONE");
    many_tags.tags = (0..MAX_TAGS + 1).map(|index| format!("tag{index}")).collect();
    assert_eq!(store.create_item(&many_tags), Err(VaultError::TooManyTags));

    // Passwords may contain spaces and symbols; that must be accepted.
    let ok = store
        .create_item(&draft("Symbols", "p a s s !@#$%^&*()_+{}:\"<>?|"))
        .unwrap();
    assert_eq!(
        store.reveal_secret(ok.id, 30).unwrap().password,
        "p a s s !@#$%^&*()_+{}:\"<>?|"
    );
}

// ----------------------------------------------------------------- generator

#[test]
fn generator_respects_length_and_selected_categories() {
    let policy = PasswordPolicy {
        length: 24,
        lowercase: true,
        uppercase: true,
        digits: true,
        symbols: true,
        exclude_similar: false,
        require_each_category: true,
    };
    let password = super::generator::generate_password(&policy).unwrap();
    assert_eq!(password.chars().count(), 24);
    assert!(password.chars().any(|c| c.is_ascii_lowercase()));
    assert!(password.chars().any(|c| c.is_ascii_uppercase()));
    assert!(password.chars().any(|c| c.is_ascii_digit()));
    assert!(password.chars().any(|c| !c.is_ascii_alphanumeric()));

    // Only digits, with the guarantee on, still yields a digits-only password.
    let digits_only = PasswordPolicy {
        length: 12,
        lowercase: false,
        uppercase: false,
        digits: true,
        symbols: false,
        exclude_similar: false,
        require_each_category: true,
    };
    let password = super::generator::generate_password(&digits_only).unwrap();
    assert!(password.chars().all(|c| c.is_ascii_digit()));

    // At least one character of every selected category, over many samples.
    for _ in 0..25 {
        let password = super::generator::generate_password(&PasswordPolicy::default()).unwrap();
        assert_eq!(password.chars().count(), 20);
        assert!(password.chars().any(|c| c.is_ascii_lowercase()));
        assert!(password.chars().any(|c| c.is_ascii_uppercase()));
        assert!(password.chars().any(|c| c.is_ascii_digit()));
        assert!(password.chars().any(|c| !c.is_ascii_alphanumeric()));
    }
}

#[test]
fn generator_can_exclude_similar_characters() {
    let policy = PasswordPolicy {
        length: 32,
        lowercase: true,
        uppercase: true,
        digits: true,
        symbols: false,
        exclude_similar: true,
        require_each_category: true,
    };
    for _ in 0..25 {
        let password = super::generator::generate_password(&policy).unwrap();
        for character in password.chars() {
            assert!(
                !"il1IL|Lo0O".contains(character),
                "similar character leaked into the password"
            );
        }
    }
}

#[test]
fn generator_refuses_impossible_policies() {
    let none = PasswordPolicy {
        length: 20,
        lowercase: false,
        uppercase: false,
        digits: false,
        symbols: false,
        exclude_similar: false,
        require_each_category: false,
    };
    assert_eq!(
        super::generator::generate_password(&none),
        Err(VaultError::InvalidPasswordPolicy)
    );

    let too_short = PasswordPolicy {
        length: 2,
        ..PasswordPolicy::default()
    };
    assert_eq!(
        super::generator::generate_password(&too_short),
        Err(VaultError::InvalidPasswordPolicy)
    );

    let too_long = PasswordPolicy {
        length: super::generator::MAX_LENGTH + 1,
        ..PasswordPolicy::default()
    };
    assert_eq!(
        super::generator::generate_password(&too_long),
        Err(VaultError::InvalidPasswordPolicy)
    );

    // Four categories in a three-character password cannot be satisfied.
    let cramped = PasswordPolicy {
        length: 3,
        require_each_category: true,
        ..PasswordPolicy::default()
    };
    assert_eq!(
        super::generator::generate_password(&cramped),
        Err(VaultError::InvalidPasswordPolicy)
    );
}

#[test]
fn index_sampling_rejects_out_of_range_bytes_instead_of_folding_them() {
    // 256 % 200 == 56, so bytes 200..=255 must be rejected rather than mapped
    // with a modulo, which is what would bias the first 56 indices.
    let mut script = vec![199u8, 200, 0, 255, 42].into_iter();
    let mut next = || script.next().unwrap();
    assert_eq!(uniform_index_from(200, &mut next), 199);
    assert_eq!(uniform_index_from(200, &mut next), 0);
    assert_eq!(uniform_index_from(200, &mut next), 42);

    // When the limit divides 256 exactly nothing is rejected.
    let mut script = vec![255u8, 0].into_iter();
    let mut next = || script.next().unwrap();
    assert_eq!(uniform_index_from(64, &mut next), 63);
    assert_eq!(uniform_index_from(64, &mut next), 0);
}

#[test]
fn index_sampling_covers_every_value_without_a_visible_bias() {
    // A deterministic byte source: the low byte of a simple LCG. This is a
    // sanity check on the mapping, the rejection test above is the strict one.
    let mut state: u32 = 0x1234_5678;
    let mut next = move || {
        state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        (state >> 16) as u8
    };
    let limit = 200usize;
    let samples = 200_000usize;
    let mut counts = vec![0usize; limit];
    for _ in 0..samples {
        counts[uniform_index_from(limit, &mut next)] += 1;
    }
    let expected = samples as f64 / limit as f64;
    for (index, count) in counts.iter().enumerate() {
        let deviation = (*count as f64 - expected).abs() / expected;
        assert!(
            deviation < 0.15,
            "index {index} deviates by {:.1}%",
            deviation * 100.0
        );
    }
}

#[test]
fn entropy_estimate_tracks_the_policy() {
    let small = PasswordPolicy {
        length: 10,
        lowercase: true,
        uppercase: false,
        digits: false,
        symbols: false,
        exclude_similar: false,
        require_each_category: false,
    };
    let large = PasswordPolicy {
        length: 20,
        ..small
    };
    let small_bits = super::generator::estimate_entropy_bits(&small);
    let large_bits = super::generator::estimate_entropy_bits(&large);
    assert!(large_bits > small_bits);
    assert!((small_bits - 10.0 * 26f64.log2()).abs() < 0.01);
    assert_eq!(
        super::generator::estimate_entropy_bits(&PasswordPolicy {
            lowercase: false,
            uppercase: false,
            digits: false,
            symbols: false,
            ..small
        }),
        0.0
    );
}

#[test]
fn generated_passwords_are_unique() {
    let policy = PasswordPolicy::default();
    let mut seen = std::collections::HashSet::new();
    for _ in 0..64 {
        let password = super::generator::generate_password(&policy).unwrap();
        assert!(seen.insert(password), "the generator repeated a password");
    }
}

// ----------------------------------------------------------------- clipboard

#[test]
fn clipboard_copy_arms_a_timer_and_clears_only_our_value() {
    let clipboard = FakeClipboard::default();
    let mut guard = ClipboardGuard::new(clipboard.clone());
    let start = Instant::now();

    let status = guard.copy_secret_at("FICTIONAL_PASSWORD_ONE", 30, start).unwrap();
    assert!(status.armed);
    assert_eq!(status.timeout_seconds, 30);
    assert_eq!(status.remaining_seconds, 31);
    assert_eq!(clipboard.value().as_deref(), Some("FICTIONAL_PASSWORD_ONE"));

    // Before the deadline nothing happens.
    assert_eq!(
        guard.wipe_if_due(start + Duration::from_secs(10)).unwrap(),
        ClipboardOutcome::Idle
    );
    assert!(guard.is_armed());
    assert_eq!(clipboard.clears(), 0);

    // At the deadline our value is cleared.
    assert_eq!(
        guard
            .wipe_if_due(start + Duration::from_secs(31))
            .unwrap(),
        ClipboardOutcome::Cleared
    );
    assert!(!guard.is_armed());
    assert_eq!(clipboard.clears(), 1);
    assert_eq!(clipboard.value(), None);
    assert!(!guard.status().armed);
}

#[test]
fn clipboard_leaves_a_newer_user_value_alone() {
    let clipboard = FakeClipboard::default();
    let mut guard = ClipboardGuard::new(clipboard.clone());
    let start = Instant::now();
    guard
        .copy_secret_at("FICTIONAL_PASSWORD_ONE", 15, start)
        .unwrap();

    // The user copies their own text before the deadline.
    clipboard.user_copies("USER_TEXT");

    assert_eq!(
        guard
            .wipe_if_due(start + Duration::from_secs(20))
            .unwrap(),
        ClipboardOutcome::LeftAlone
    );
    assert!(!guard.is_armed());
    assert_eq!(clipboard.clears(), 0, "foreign clipboard text must survive");
    assert_eq!(clipboard.value().as_deref(), Some("USER_TEXT"));
}

#[test]
fn clipboard_errors_never_include_the_secret() {
    let clipboard = FakeClipboard::default();
    clipboard.fail_write();
    let mut guard = ClipboardGuard::new(clipboard.clone());
    let error = guard
        .copy_secret("FICTIONAL_PASSWORD_ONE", 30)
        .unwrap_err();
    let rendered = format!("{error:?} {error}");
    assert!(!rendered.contains("FICTIONAL_PASSWORD_ONE"));
    assert_eq!(error, ClipboardError::Unavailable);
    assert!(!guard.is_armed());
    assert!(clipboard.writes().is_empty());

    // An unreadable clipboard is left alone, not cleared blindly.
    let unreadable = FakeClipboard::default();
    let mut guard = ClipboardGuard::new(unreadable.clone());
    let start = Instant::now();
    guard
        .copy_secret_at("FICTIONAL_PASSWORD_ONE", 20, start)
        .unwrap();
    unreadable.fail_read();
    assert_eq!(
        guard.wipe_if_due(start + Duration::from_secs(21)).unwrap(),
        ClipboardOutcome::Unreadable
    );
    assert_eq!(unreadable.clears(), 0);
    assert!(!guard.is_armed());
}

#[test]
fn clipboard_timeout_is_clamped_and_replaced_on_retry() {
    assert_eq!(clamp_clear_seconds(1), MIN_CLEAR_SECONDS);
    assert_eq!(clamp_clear_seconds(30), 30);
    assert_eq!(clamp_clear_seconds(600), MAX_CLEAR_SECONDS);

    let clipboard = FakeClipboard::default();
    let mut guard = ClipboardGuard::new(clipboard.clone());
    let start = Instant::now();
    let first = guard.copy_secret_at("FICTIONAL_PASSWORD_ONE", 5, start).unwrap();
    assert_eq!(first.timeout_seconds, MIN_CLEAR_SECONDS);
    let second = guard
        .copy_secret_at("FICTIONAL_PASSWORD_TWO", 900, start + Duration::from_secs(1))
        .unwrap();
    assert_eq!(second.timeout_seconds, MAX_CLEAR_SECONDS);
    assert_eq!(second.remaining_seconds, 61);
    assert_eq!(
        clipboard.writes(),
        vec![
            "FICTIONAL_PASSWORD_ONE".to_string(),
            "FICTIONAL_PASSWORD_TWO".to_string()
        ]
    );

    // The replaced timer must not fire against the newer value at the old
    // deadline: only the new deadline clears it.
    assert_eq!(
        guard.wipe_if_due(start + Duration::from_secs(16)).unwrap(),
        ClipboardOutcome::Idle
    );
    assert_eq!(clipboard.clears(), 0);

    // Locking cancels the timer and clears our value immediately.
    assert_eq!(guard.cancel().unwrap(), ClipboardOutcome::Cleared);
    assert!(!guard.is_armed());
    assert_eq!(clipboard.clears(), 1);
    assert_eq!(guard.cancel().unwrap(), ClipboardOutcome::Idle);
}

#[test]
fn clipboard_debug_never_shows_the_secret() {
    let clipboard = FakeClipboard::default();
    let mut guard = ClipboardGuard::new(clipboard);
    guard
        .copy_secret("FICTIONAL_PASSWORD_ONE", 30)
        .unwrap();
    let rendered = format!("{guard:?}");
    assert!(!rendered.contains("FICTIONAL_PASSWORD_ONE"));
    assert!(rendered.contains("<redacted>"));
}

// ---------------------------------------------------------------- auto-lock

#[test]
fn idle_lock_fires_only_after_the_configured_timeout() {
    let start = Instant::now();
    let mut idle = IdleLock::with_timeout_at(300, start);
    assert_eq!(idle.timeout_seconds(), 300);
    assert!(idle.is_automatic());
    assert_eq!(idle.remaining_seconds(start), Some(300));

    assert!(!idle.is_due(start + Duration::from_secs(299)));
    assert_eq!(
        idle.remaining_seconds(start + Duration::from_secs(299)),
        Some(1)
    );
    assert!(idle.is_due(start + Duration::from_secs(300)));
    assert!(idle.is_due(start + Duration::from_secs(9_999)));

    // Activity restarts the clock.
    let activity = start + Duration::from_secs(290);
    idle.touch(activity);
    assert!(!idle.is_due(start + Duration::from_secs(300)));
    assert!(idle.is_due(activity + Duration::from_secs(300)));
}

#[test]
fn idle_lock_supports_never_and_the_documented_options() {
    let start = Instant::now();
    let never = IdleLock::with_timeout_at(0, start);
    assert!(!never.is_automatic());
    assert_eq!(never.remaining_seconds(start), None);
    assert!(!never.is_due(start + Duration::from_secs(86_400)));

    for option in IDLE_TIMEOUT_OPTIONS {
        let idle = IdleLock::with_timeout_at(option, start);
        assert_eq!(idle.timeout_seconds(), option);
    }

    // An unsupported value must not silently shorten or extend the policy.
    let fallback = IdleLock::with_timeout_at(7, start);
    assert_eq!(fallback.timeout_seconds(), DEFAULT_IDLE_TIMEOUT_SECONDS);
    assert_eq!(normalize_timeout(12_345), DEFAULT_IDLE_TIMEOUT_SECONDS);
    assert_eq!(normalize_timeout(60), 60);

    // The timeout can be changed at runtime, including to "never".
    let mut idle = IdleLock::with_timeout_at(60, start);
    idle.set_timeout(0);
    assert!(!idle.is_due(start + Duration::from_secs(600)));
    idle.set_timeout(1800);
    assert!(idle.is_due(start + Duration::from_secs(1800)));
}

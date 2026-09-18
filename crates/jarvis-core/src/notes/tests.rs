//! Unit tests for the notes layer.

use super::model::*;
use super::store::NoteStore;
use crate::sync::crypto::{random_master_key, MasterKey, MasterKeyCryptoProvider};
use crate::sync::{
    ApplyOutcome, CryptoProvider, DeviceId, InMemorySyncRepository, PayloadContext,
    SyncEntityType, SyncMutation, SyncOperationKind, SyncRepository,
};
use uuid::Uuid;

type TestStore = NoteStore<InMemorySyncRepository, MasterKeyCryptoProvider>;

fn store() -> TestStore {
    NoteStore::new(
        InMemorySyncRepository::new(),
        MasterKeyCryptoProvider::new(random_master_key().unwrap()),
        DeviceId::new("notes_test_device").unwrap(),
    )
}

fn draft(title: &str, body: &str) -> NoteDraft {
    NoteDraft {
        title: title.to_string(),
        body: body.to_string(),
        folder_id: None,
        tags: Vec::new(),
    }
}

#[test]
fn creates_reads_and_updates_a_note() {
    let mut store = store();
    let created = store.create_note(&draft("FICTIONAL_TITLE", "FICTIONAL_BODY")).unwrap();
    assert_eq!(created.revision, 1);
    assert!(!created.pinned);
    assert!(created.deleted_at.is_none());

    let fetched = store.get_note(created.id).unwrap().unwrap();
    assert_eq!(fetched.title, "FICTIONAL_TITLE");
    assert_eq!(fetched.body, "FICTIONAL_BODY");

    let updated = store
        .update_note(created.id, &draft("FICTIONAL_TITLE_2", "FICTIONAL_BODY_2"))
        .unwrap();
    assert_eq!(updated.revision, 2);
    assert_eq!(updated.title, "FICTIONAL_TITLE_2");
}

#[test]
fn debug_output_never_contains_note_content() {
    let mut store = store();
    let created = store
        .create_note(&draft("FICTIONAL_TITLE", "FICTIONAL_BODY"))
        .unwrap();
    let draft_value = draft("FICTIONAL_TITLE", "FICTIONAL_BODY");
    let list = store
        .list_notes(&NoteQuery {
            trash: TrashFilter::All,
            ..NoteQuery::default()
        })
        .unwrap();
    let rendered = format!(
        "{created:?} {draft_value:?} {list:?} {:?}",
        created.to_summary()
    );
    assert!(!rendered.contains("FICTIONAL_TITLE"));
    assert!(!rendered.contains("FICTIONAL_BODY"));
}

#[test]
fn trash_and_restore_keep_the_content() {
    let mut store = store();
    let created = store.create_note(&draft("FICTIONAL", "body")).unwrap();

    let trashed = store.trash_note(created.id).unwrap();
    assert!(trashed.is_trashed());
    assert_eq!(store.get_note(created.id).unwrap().unwrap().body, "body");

    let active = store
        .list_notes(&NoteQuery {
            trash: TrashFilter::Active,
            ..NoteQuery::default()
        })
        .unwrap();
    assert!(active.items.is_empty());
    let trashed_list = store
        .list_notes(&NoteQuery {
            trash: TrashFilter::Trashed,
            ..NoteQuery::default()
        })
        .unwrap();
    assert_eq!(trashed_list.items.len(), 1);

    let restored = store.restore_note(created.id).unwrap();
    assert!(!restored.is_trashed());
    assert_eq!(restored.title, "FICTIONAL");
}

#[test]
fn purge_writes_a_tombstone_and_hides_the_note_everywhere() {
    let mut store = store();
    let created = store.create_note(&draft("FICTIONAL", "body")).unwrap();
    store.purge_note(created.id).unwrap();

    assert!(store.get_note(created.id).unwrap().is_none());
    for trash in [TrashFilter::Active, TrashFilter::Trashed, TrashFilter::All] {
        let list = store.list_notes(&NoteQuery { trash, ..NoteQuery::default() }).unwrap();
        assert!(list.items.is_empty());
    }
    assert_eq!(store.update_note(created.id, &draft("x", "y")), Err(NoteError::Deleted));
}

#[test]
fn search_matches_title_body_and_tags_case_insensitively() {
    let mut store = store();
    store
        .create_note(&draft("Grocery list", "milk and bread"))
        .unwrap();
    let mut tagged = draft("Ideas", "nothing here");
    tagged.tags = vec!["Projects".into()];
    store.create_note(&tagged).unwrap();

    let search = |store: &mut TestStore, needle: &str| -> usize {
        store
            .list_notes(&NoteQuery {
                search: needle.to_string(),
                ..NoteQuery::default()
            })
            .unwrap()
            .items
            .len()
    };
    assert_eq!(search(&mut store, "GROCERY"), 1);
    assert_eq!(search(&mut store, "bread"), 1);
    assert_eq!(search(&mut store, "projects"), 1);
    assert_eq!(search(&mut store, "missing"), 0);
}

#[test]
fn folders_partition_notes_and_purge_detaches_them() {
    let mut store = store();
    let folder = store.create_folder("Work").unwrap();
    let mut assignment = draft("Report", "quarterly");
    assignment.folder_id = Some(folder.id);
    let note = store.create_note(&assignment).unwrap();

    let in_folder = store
        .list_notes(&NoteQuery {
            folder_id: Some(folder.id),
            ..NoteQuery::default()
        })
        .unwrap();
    assert_eq!(in_folder.items.len(), 1);

    store.purge_folder(folder.id).unwrap();
    assert!(store.folders().unwrap().is_empty());
    let detached = store.get_note(note.id).unwrap().unwrap();
    assert!(detached.folder_id.is_none());
}

#[test]
fn tags_are_normalized_deduplicated_and_collected() {
    let mut store = store();
    let mut tagged = draft("Tagged", "body");
    tagged.tags = vec![" #Work ".into(), "work".into(), "Home".into(), "  ".into()];
    let note = store.create_note(&tagged).unwrap();
    assert_eq!(note.tags, vec!["Work".to_string(), "Home".to_string()]);

    let tags = store.all_tags().unwrap();
    assert_eq!(tags, vec!["Home".to_string(), "Work".to_string()]);

    let filtered = store
        .list_notes(&NoteQuery {
            tag: Some("work".into()),
            ..NoteQuery::default()
        })
        .unwrap();
    assert_eq!(filtered.items.len(), 1);
}

#[test]
fn pinned_notes_sort_first() {
    let mut store = store();
    let first = store.create_note(&draft("First", "a")).unwrap();
    let second = store.create_note(&draft("Second", "b")).unwrap();
    store.set_pinned(second.id, true).unwrap();

    let list = store
        .list_notes(&NoteQuery {
            pinned_first: true,
            sort: NoteSort::TitleAsc,
            ..NoteQuery::default()
        })
        .unwrap();
    assert_eq!(list.items[0].id, second.id);
    let unpinned_first = store
        .list_notes(&NoteQuery {
            pinned_first: false,
            sort: NoteSort::TitleAsc,
            ..NoteQuery::default()
        })
        .unwrap();
    assert_eq!(unpinned_first.items[0].id, first.id);
}

#[test]
fn notes_survive_a_store_restart_and_a_wrong_key_cannot_read_them() {
    let device = DeviceId::new("notes_test_device").unwrap();
    let key = random_master_key().unwrap();
    // A second copy of the same key material stands in for the next app start,
    // where the key is unwrapped again from DPAPI or the backup envelope.
    let same_key = MasterKey::from_bytes(*key.as_array());

    // First session: create a note and keep the repository.
    let mut first = NoteStore::new(
        InMemorySyncRepository::new(),
        MasterKeyCryptoProvider::new(key),
        device.clone(),
    );
    let note_id = first
        .create_note(&draft("FICTIONAL", "persisted body"))
        .unwrap()
        .id;
    let repository = first.into_repository();

    // Second session with the same key: the note is readable again.
    let mut reopened = NoteStore::new(
        repository,
        MasterKeyCryptoProvider::new(same_key),
        device.clone(),
    );
    let note = reopened.get_note(note_id).unwrap().unwrap();
    assert_eq!(note.body, "persisted body");
    assert_eq!(note.revision, 1);
    let repository = reopened.into_repository();

    // Third session with a different key: the payload must fail closed, and the
    // list must report the unreadable note instead of hiding it silently.
    let mut wrong = NoteStore::new(
        repository,
        MasterKeyCryptoProvider::new(random_master_key().unwrap()),
        device,
    );
    assert_eq!(wrong.get_note(note_id), Err(NoteError::Unreadable));
    let list = wrong
        .list_notes(&NoteQuery {
            trash: TrashFilter::All,
            ..NoteQuery::default()
        })
        .unwrap();
    assert!(list.items.is_empty());
    assert_eq!(list.unreadable, 1);
}

/// A store plus one stored conflict on its single note.
struct ConflictFixture {
    store: TestStore,
    note_id: Uuid,
    conflict_id: Uuid,
}

fn fixture_with_conflict() -> ConflictFixture {
    let key = random_master_key().unwrap();
    let twin = MasterKey::from_bytes(*key.as_array());
    let provider = MasterKeyCryptoProvider::new(twin);
    let device = DeviceId::new("notes_test_device").unwrap();
    let mut store = NoteStore::new(
        InMemorySyncRepository::new(),
        MasterKeyCryptoProvider::new(key),
        device,
    );
    let note = store
        .create_note(&draft("current title", "current body"))
        .unwrap();

    let now = chrono::Utc::now().to_rfc3339();
    let incoming = NotePayload {
        schema_version: NOTE_PAYLOAD_SCHEMA_VERSION,
        title: "incoming title".into(),
        body: "incoming body".into(),
        folder_id: None,
        tags: vec!["incoming".into()],
        pinned: false,
        created_at: now.clone(),
        updated_at: now.clone(),
        deleted_at: None,
    };
    let payload = provider
        .encrypt(
            &PayloadContext::new(SyncEntityType::Note, note.id),
            &incoming.to_bytes().unwrap(),
        )
        .unwrap();
    let mutation = SyncMutation {
        operation_id: Uuid::new_v4(),
        entity_id: note.id,
        entity_type: SyncEntityType::Note,
        device_id: DeviceId::new("other_device").unwrap(),
        device_sequence: 1,
        base_revision: 0,
        kind: SyncOperationKind::Update,
        timestamp: now,
        schema_version: SyncMutation::CURRENT_SCHEMA_VERSION,
        encrypted_payload: Some(payload),
    };
    let conflict_id = match store.repository_mut().apply_mutation(mutation).unwrap() {
        ApplyOutcome::Conflict { conflict_id } => conflict_id,
        other => panic!("expected a conflict, got {other:?}"),
    };
    ConflictFixture {
        store,
        note_id: note.id,
        conflict_id,
    }
}

#[test]
fn a_conflict_exposes_both_versions_and_keeps_the_note_untouched() {
    let mut fixture = fixture_with_conflict();
    let view = fixture.store.conflicts().unwrap();
    assert_eq!(view.len(), 1);
    assert_eq!(view[0].current.as_ref().unwrap().title, "current title");
    assert_eq!(view[0].incoming.as_ref().unwrap().title, "incoming title");
    assert_eq!(view[0].current_revision, 1);
    assert_eq!(view[0].incoming_revision, 1);
    assert_eq!(
        fixture
            .store
            .get_note(fixture.note_id)
            .unwrap()
            .unwrap()
            .title,
        "current title"
    );
}

#[test]
fn resolving_with_keep_current_discards_only_the_pending_conflict() {
    let mut fixture = fixture_with_conflict();
    let outcome = fixture
        .store
        .resolve_conflict(fixture.conflict_id, NoteConflictResolution::KeepCurrent)
        .unwrap();
    assert_eq!(outcome.resolution, NoteConflictResolution::KeepCurrent);
    assert!(outcome.created_entity_id.is_none());
    assert!(fixture.store.conflicts().unwrap().is_empty());
    assert_eq!(
        fixture
            .store
            .get_note(fixture.note_id)
            .unwrap()
            .unwrap()
            .title,
        "current title"
    );
}

#[test]
fn resolving_with_accept_incoming_replaces_the_stored_version() {
    let mut fixture = fixture_with_conflict();
    let outcome = fixture
        .store
        .resolve_conflict(fixture.conflict_id, NoteConflictResolution::AcceptIncoming)
        .unwrap();
    assert_eq!(outcome.updated_entity_id, Some(fixture.note_id));
    assert!(fixture.store.conflicts().unwrap().is_empty());
    let note = fixture.store.get_note(fixture.note_id).unwrap().unwrap();
    assert_eq!(note.title, "incoming title");
    assert_eq!(note.body, "incoming body");
    assert_eq!(note.revision, 2);
    assert_eq!(note.tags, vec!["incoming".to_string()]);
}

#[test]
fn resolving_with_keep_both_stores_the_incoming_version_as_a_new_note() {
    let mut fixture = fixture_with_conflict();
    let outcome = fixture
        .store
        .resolve_conflict(fixture.conflict_id, NoteConflictResolution::KeepBoth)
        .unwrap();
    let created = outcome.created_entity_id.unwrap();
    assert_ne!(created, fixture.note_id);
    assert!(fixture.store.conflicts().unwrap().is_empty());

    let list = fixture
        .store
        .list_notes(&NoteQuery {
            sort: NoteSort::TitleAsc,
            ..NoteQuery::default()
        })
        .unwrap();
    assert_eq!(list.items.len(), 2);
    assert_eq!(
        fixture.store.get_note(fixture.note_id).unwrap().unwrap().title,
        "current title"
    );
    assert_eq!(
        fixture.store.get_note(created).unwrap().unwrap().title,
        "incoming title"
    );
}

#[test]
fn malformed_drafts_are_rejected_without_touching_storage() {
    let mut store = store();
    let mut long_title = draft("ok", "body");
    long_title.title = "t".repeat(MAX_TITLE_CHARS + 1);
    assert_eq!(store.create_note(&long_title), Err(NoteError::TitleTooLong));

    let mut newline_title = draft("ok", "body");
    newline_title.title = "bad\nline".into();
    assert_eq!(
        store.create_note(&newline_title),
        Err(NoteError::TitleHasControlCharacters)
    );

    let mut too_many_tags = draft("ok", "body");
    too_many_tags.tags = (0..MAX_TAGS + 1).map(|index| format!("tag{index}")).collect();
    assert_eq!(store.create_note(&too_many_tags), Err(NoteError::TooManyTags));

    let mut long_body = draft("ok", "body");
    long_body.body = "b".repeat(MAX_BODY_BYTES + 1);
    assert_eq!(store.create_note(&long_body), Err(NoteError::BodyTooLarge));

    let created = store.create_note(&draft("keep", "this")).unwrap();
    assert_eq!(store.create_folder("   "), Err(NoteError::EmptyFolderName));
    assert!(store.get_note(created.id).unwrap().is_some());
    assert_eq!(store.get_note(Uuid::new_v4()).unwrap(), None);
    assert_eq!(
        store.update_note(Uuid::new_v4(), &draft("x", "y")),
        Err(NoteError::NotFound)
    );
}

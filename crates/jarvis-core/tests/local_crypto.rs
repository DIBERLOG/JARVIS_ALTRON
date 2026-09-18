//! Security tests for record encryption, Windows DPAPI, and portable backups.
//!
//! All passwords, notes, and keys in this file are fictional fixtures.

use jarvis_core::sync::crypto::{
    decrypt, dpapi_protect, dpapi_unprotect, encrypt, export_backup, import_backup,
    random_master_key, CryptoError, DpapiProtectedKey, EncryptedRecord, MasterKeyCryptoProvider,
    PortableKeyBackup, BACKUP_FORMAT_VERSION, KEY_BYTES, MIN_PASSWORD_BYTES,
    RECORD_FORMAT_VERSION,
};
use jarvis_core::sync::{CryptoProvider, PayloadContext, SyncEntityType, SyncError, SyncMutation};
use uuid::Uuid;

const PASSWORD: &[u8] = b"fictional-master-password";
const WRONG_PASSWORD: &[u8] = b"fictional-wrong-password";

fn vault_context() -> PayloadContext {
    PayloadContext::new(SyncEntityType::VaultRecord, Uuid::new_v4())
}

#[test]
fn records_round_trip_with_unique_nonces_and_authenticated_vault_context() {
    let key = random_master_key().unwrap();
    let context = vault_context();
    let aad = context.aad();
    let first = encrypt(&key, &aad, b"FICTIONAL_VAULT_SECRET").unwrap();
    let second = encrypt(&key, &aad, b"FICTIONAL_VAULT_SECRET").unwrap();

    assert_eq!(first.format_version, RECORD_FORMAT_VERSION);
    assert_ne!(first.nonce, second.nonce);
    assert_ne!(first.ciphertext, second.ciphertext);
    assert!(!first
        .ciphertext
        .windows(22)
        .any(|window| window == b"FICTIONAL_VAULT_SECRET"));
    assert_eq!(
        decrypt(&key, &aad, &first).unwrap(),
        b"FICTIONAL_VAULT_SECRET"
    );

    // The same payload under a different entity context must not decrypt.
    let other = vault_context().aad();
    assert_eq!(
        decrypt(&key, &other, &first),
        Err(CryptoError::InvalidPasswordOrCorruptData)
    );
}

#[test]
fn a_wrong_master_key_is_rejected() {
    let key = random_master_key().unwrap();
    let other = random_master_key().unwrap();
    let aad = vault_context().aad();
    let record = encrypt(&key, &aad, b"FICTIONAL_VAULT_SECRET").unwrap();

    assert!(decrypt(&other, &aad, &record).is_err());
    assert_eq!(
        decrypt(&key, &aad, &record).unwrap(),
        b"FICTIONAL_VAULT_SECRET"
    );
}

#[test]
fn corrupted_ciphertext_nonce_and_version_are_rejected() {
    let key = random_master_key().unwrap();
    let aad = vault_context().aad();
    let record = encrypt(&key, &aad, b"FICTIONAL_VAULT_SECRET").unwrap();

    let mut corrupted = record.clone();
    let last = corrupted.ciphertext.len() - 1;
    corrupted.ciphertext[last] ^= 0x5a;
    assert!(decrypt(&key, &aad, &corrupted).is_err());

    let mut truncated = record.clone();
    truncated.ciphertext.truncate(4);
    assert!(decrypt(&key, &aad, &truncated).is_err());

    let mut replayed_nonce = record.clone();
    replayed_nonce.nonce[0] ^= 0x01;
    assert!(decrypt(&key, &aad, &replayed_nonce).is_err());

    let mut unknown_version = record.clone();
    unknown_version.format_version = RECORD_FORMAT_VERSION + 1;
    assert_eq!(
        decrypt(&key, &aad, &unknown_version),
        Err(CryptoError::UnsupportedFormat)
    );

    // The serialized envelope refuses unknown versions and truncated input.
    let mut encoded = record.encode();
    encoded[0] = RECORD_FORMAT_VERSION + 1;
    assert_eq!(
        EncryptedRecord::decode(&encoded),
        Err(CryptoError::UnsupportedFormat)
    );
    assert_eq!(
        EncryptedRecord::decode(&encoded[..EncryptedRecord::HEADER_BYTES]),
        Err(CryptoError::UnsupportedFormat)
    );
    // A valid envelope still decodes to the same record.
    assert_eq!(EncryptedRecord::decode(&record.encode()).unwrap(), record);
}

#[test]
fn portable_backup_round_trips_through_its_json_envelope() {
    let key = random_master_key().unwrap();
    let aad = vault_context().aad();
    let record = encrypt(&key, &aad, b"FICTIONAL_VAULT_SECRET").unwrap();
    let backup = export_backup(&key, PASSWORD).unwrap();

    assert_eq!(backup.format_version, BACKUP_FORMAT_VERSION);
    assert_eq!(backup.kdf, "argon2id");
    assert_eq!(backup.aead, "xchacha20poly1305");
    // An AEAD-wrapped key is longer than the key: tag plus ciphertext.
    assert!(backup.encrypted_master_key.len() > KEY_BYTES);
    assert_eq!(backup.salt.len(), 16);
    assert_eq!(backup.nonce.len(), 24);

    let json = backup.to_json().unwrap();
    assert!(json.contains("argon2id"));
    let parsed = PortableKeyBackup::from_json(&json).unwrap();
    assert_eq!(parsed, backup);

    let restored = import_backup(&parsed, PASSWORD).unwrap();
    assert_eq!(
        decrypt(&restored, &aad, &record).unwrap(),
        b"FICTIONAL_VAULT_SECRET"
    );
    // The restored key produces records the original key can read and vice versa.
    let restored_record = encrypt(&restored, &aad, b"FICTIONAL_VAULT_SECRET").unwrap();
    assert_eq!(
        decrypt(&key, &aad, &restored_record).unwrap(),
        b"FICTIONAL_VAULT_SECRET"
    );

    // A different password cannot restore the envelope.
    assert!(import_backup(&parsed, WRONG_PASSWORD).is_err());
}

#[test]
fn tampered_backup_metadata_and_parameters_are_rejected() {
    let key = random_master_key().unwrap();
    let backup = export_backup(&key, PASSWORD).unwrap();

    let mut altered_metadata = backup.clone();
    altered_metadata.metadata.push(b'x');
    assert!(import_backup(&altered_metadata, PASSWORD).is_err());

    let mut altered_ciphertext = backup.clone();
    altered_ciphertext.encrypted_master_key[0] ^= 0x5a;
    assert!(import_backup(&altered_ciphertext, PASSWORD).is_err());

    let mut altered_salt = backup.clone();
    altered_salt.salt[0] ^= 0x5a;
    assert!(import_backup(&altered_salt, PASSWORD).is_err());

    let mut unknown_version = backup.clone();
    unknown_version.format_version = BACKUP_FORMAT_VERSION + 1;
    assert_eq!(
        import_backup(&unknown_version, PASSWORD).unwrap_err(),
        CryptoError::UnsupportedFormat
    );

    let mut unknown_kdf = backup.clone();
    unknown_kdf.kdf = "unsupported-kdf".into();
    assert!(import_backup(&unknown_kdf, PASSWORD).is_err());

    let mut unknown_aead = backup.clone();
    unknown_aead.aead = "unsupported-aead".into();
    assert!(import_backup(&unknown_aead, PASSWORD).is_err());

    let mut short_nonce = backup.clone();
    short_nonce.nonce.truncate(4);
    assert!(import_backup(&short_nonce, PASSWORD).is_err());

    let mut short_salt = backup.clone();
    short_salt.salt.truncate(2);
    assert!(import_backup(&short_salt, PASSWORD).is_err());

    // Untrusted KDF parameters are bounded so a hostile envelope cannot ask for
    // unbounded memory or CPU.
    let mut huge_memory = backup.clone();
    huge_memory.memory_kib = 4 * 1024 * 1024;
    huge_memory.metadata = rebuild_metadata(&huge_memory);
    assert_eq!(
        import_backup(&huge_memory, PASSWORD).unwrap_err(),
        CryptoError::UnsupportedFormat
    );

    let mut huge_iterations = backup.clone();
    huge_iterations.iterations = 4096;
    huge_iterations.metadata = rebuild_metadata(&huge_iterations);
    assert_eq!(
        import_backup(&huge_iterations, PASSWORD).unwrap_err(),
        CryptoError::UnsupportedFormat
    );

    let mut zero_iterations = backup.clone();
    zero_iterations.iterations = 0;
    zero_iterations.metadata = rebuild_metadata(&zero_iterations);
    assert!(import_backup(&zero_iterations, PASSWORD).is_err());

    // The untouched envelope still restores.
    assert!(import_backup(&backup, PASSWORD).is_ok());
}

/// Rebuilds the authenticated metadata so only the parameter under test changes.
fn rebuild_metadata(backup: &PortableKeyBackup) -> Vec<u8> {
    let mut metadata = format!(
        "jarvis-key-backup|{}|{}|{}|{}|{}|{}|",
        backup.format_version,
        backup.kdf,
        backup.memory_kib,
        backup.iterations,
        backup.parallelism,
        backup.aead
    )
    .into_bytes();
    metadata.extend_from_slice(&backup.salt);
    metadata
}

#[test]
fn weak_master_passwords_are_refused() {
    let key = random_master_key().unwrap();
    let short = vec![b'a'; MIN_PASSWORD_BYTES - 1];
    assert_eq!(
        export_backup(&key, &short),
        Err(CryptoError::WeakMasterPassword)
    );
    assert_eq!(
        import_backup(&export_backup(&key, PASSWORD).unwrap(), &short).unwrap_err(),
        CryptoError::WeakMasterPassword
    );
    // The boundary itself is accepted.
    assert!(export_backup(&key, &[b'a'; MIN_PASSWORD_BYTES]).is_ok());
}

#[test]
fn diagnostics_never_reveal_keys_plaintext_or_passwords() {
    let key = random_master_key().unwrap();
    let aad = vault_context().aad();
    let record = encrypt(&key, &aad, b"FICTIONAL_VAULT_SECRET").unwrap();
    let backup = export_backup(&key, PASSWORD).unwrap();
    let provider = MasterKeyCryptoProvider::new(random_master_key().unwrap());
    let mutation = SyncMutation {
        operation_id: Uuid::new_v4(),
        entity_id: Uuid::new_v4(),
        entity_type: SyncEntityType::VaultRecord,
        device_id: jarvis_core::sync::DeviceId::new("test_device").unwrap(),
        device_sequence: 1,
        base_revision: 0,
        kind: jarvis_core::sync::SyncOperationKind::Create,
        timestamp: "2026-01-01T00:00:00Z".into(),
        schema_version: SyncMutation::CURRENT_SCHEMA_VERSION,
        encrypted_payload: None,
    };

    let rendered = format!("{key:?} {record:?} {backup:?} {provider:?} {mutation:?}");
    assert!(!rendered.contains("FICTIONAL_VAULT_SECRET"));
    assert!(!rendered.contains("fictional-master-password"));
    assert!(rendered.contains("<redacted>"));
}

#[test]
fn the_crypto_provider_is_wired_into_the_sync_payload_type() {
    let key = random_master_key().unwrap();
    let provider = MasterKeyCryptoProvider::new(key);
    let context = vault_context();
    let payload = provider
        .encrypt(&context, b"FICTIONAL_VAULT_SECRET")
        .unwrap();
    assert!(payload.byte_len() > 0);
    assert_eq!(
        provider.decrypt(&context, &payload).unwrap(),
        b"FICTIONAL_VAULT_SECRET"
    );
    assert!(!format!("{payload:?}").contains("FICTIONAL_VAULT_SECRET"));

    // A different context (same entity type, different entity) fails closed.
    let other = vault_context();
    assert_eq!(
        provider.decrypt(&other, &payload),
        Err(SyncError::CryptoRejected)
    );
}

#[cfg(windows)]
mod windows_dpapi {
    use super::*;

    #[test]
    fn dpapi_round_trips_for_the_current_user() {
        let key = random_master_key().unwrap();
        let aad = vault_context().aad();
        let record = encrypt(&key, &aad, b"FICTIONAL_VAULT_SECRET").unwrap();

        let blob = dpapi_protect(&key).unwrap();
        assert!(!blob.as_bytes().is_empty());
        // The blob is not the raw key.
        assert_ne!(blob.as_bytes().len(), KEY_BYTES);

        let restored = dpapi_unprotect(&blob).unwrap();
        assert_eq!(
            decrypt(&restored, &aad, &record).unwrap(),
            b"FICTIONAL_VAULT_SECRET"
        );

        // A damaged blob is refused with a controlled error.
        let mut damaged = blob.as_bytes().to_vec();
        let last = damaged.len() - 1;
        damaged[last] ^= 0x5a;
        assert!(dpapi_unprotect(&DpapiProtectedKey::from_bytes(damaged)).is_err());
        assert!(dpapi_unprotect(&DpapiProtectedKey::from_bytes(Vec::new())).is_err());
        assert!(dpapi_unprotect(&DpapiProtectedKey::from_bytes(vec![0; 8])).is_err());

        // Diagnostics never print the blob.
        assert!(!format!("{blob:?}").contains("FICTIONAL"));
        assert!(format!("{blob:?}").contains("<redacted>"));
    }

    #[test]
    fn a_portable_backup_is_not_a_dpapi_blob() {
        let key = random_master_key().unwrap();
        let blob = dpapi_protect(&key).unwrap();
        let backup = export_backup(&key, PASSWORD).unwrap();
        let json = backup.to_json().unwrap();

        // The DPAPI blob must never be embedded in the portable envelope.
        let mut blob_json = String::new();
        for byte in blob.as_bytes() {
            blob_json.push_str(&byte.to_string());
            blob_json.push(',');
        }
        let blob_json = blob_json.trim_end_matches(',').to_string();
        assert!(
            !json.contains(&blob_json),
            "portable backup must not embed the DPAPI blob"
        );
        assert!(json.contains("argon2id"));
    }

    #[test]
    fn a_portable_backup_restores_after_a_new_local_dpapi_blob() {
        let key = random_master_key().unwrap();
        let aad = vault_context().aad();
        let record = encrypt(&key, &aad, b"FICTIONAL_VAULT_SECRET").unwrap();
        let backup = export_backup(&key, PASSWORD).unwrap();

        // Simulate a new installation that already created its own local blob.
        let local_key = random_master_key().unwrap();
        let previous_blob = dpapi_protect(&local_key).unwrap();

        // Restore from the portable envelope, then re-protect locally.
        let restored = import_backup(&backup, PASSWORD).unwrap();
        let new_blob = dpapi_protect(&restored).unwrap();
        assert_ne!(new_blob.as_bytes(), previous_blob.as_bytes());

        let unlocked = dpapi_unprotect(&new_blob).unwrap();
        assert_eq!(
            decrypt(&unlocked, &aad, &record).unwrap(),
            b"FICTIONAL_VAULT_SECRET"
        );
        // The old local blob still belongs to the old key, not the restored one.
        let old_local = dpapi_unprotect(&previous_blob).unwrap();
        assert!(decrypt(&old_local, &aad, &record).is_err());
    }
}

#[cfg(not(windows))]
#[test]
fn dpapi_reports_an_unsupported_platform() {
    let key = random_master_key().unwrap();
    assert_eq!(dpapi_protect(&key), Err(CryptoError::UnsupportedPlatform));
    assert_eq!(
        dpapi_unprotect(&DpapiProtectedKey::from_bytes(vec![1, 2, 3])),
        Err(CryptoError::UnsupportedPlatform)
    );
}

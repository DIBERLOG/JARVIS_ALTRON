//! Runs only against newly-created temporary profiles with fictional secrets.
use jarvis_core::{sync_profile,secure_storage,db};
use serde_json::{json,Value};
use sha2::{Digest,Sha256};
use std::{path::PathBuf,process::Command};

fn main(){
    let args:Vec<_>=std::env::args_os().collect();
    if args.len()==1{
        let root=tempfile::tempdir().unwrap();std::fs::write(root.path().join("jarvis-fictional-test-only.marker"),b"test-only").unwrap();
        for stage in ["create","restore"]{let result=Command::new(std::env::current_exe().unwrap()).arg(stage).arg(root.path()).status().unwrap();assert!(result.success());}
        println!("PASS: legacy migration, preserved source, wrong-password refusal, fresh OS credential, restored vault and unknown settings, durable recovery queue");return;
    }
    assert_eq!(args.len(),3);let parent=PathBuf::from(&args[2]);assert_eq!(std::fs::read(parent.join("jarvis-fictional-test-only.marker")).unwrap(),b"test-only");
    let stage=args[1].to_str().unwrap();let root=parent.join(if stage=="create"{"first"}else{"fresh"});std::fs::create_dir_all(&root).unwrap();
    jarvis_core::APP_CONFIG_DIR.set(root.clone()).unwrap();
    let name:String=Sha256::digest(root.canonicalize().unwrap().to_string_lossy().as_bytes()).iter().map(|b|format!("{b:02x}")).collect();
    #[cfg(any(windows,target_os="macos",target_os="linux"))]
    struct Cleanup(keyring::Entry);
    #[cfg(any(windows,target_os="macos",target_os="linux"))]
    impl Drop for Cleanup{fn drop(&mut self){let _=self.0.delete_credential();}}
    #[cfg(any(windows,target_os="macos",target_os="linux"))]
    let _cleanup=Cleanup(keyring::Entry::new("JARVIS_ALTRON.local-data.v1",&name).unwrap());
    let password="fictional recovery only 2026";
    if stage=="create"{
        let mut settings=jarvis_core::Settings::default();settings.api_keys.openai="fictional-api-never-transfer".into();
        settings.center_data=json!({"notes":[{"id":"fictional-note","title":"Test","text":"fictional-only-private","updatedAt":"2026-10-12T12:00:00Z"}],"reminders":[],"birthdays":[],"habits":[]}).to_string();
        let mut source=serde_json::to_value(&settings).unwrap();source["future_setting"]=json!({"preserve":true});
        let path=root.join(jarvis_core::config::DB_FILE_NAME);altron_command_engine::persistence::replace(&path,&source).unwrap();let original=std::fs::read(&path).unwrap();
        let fixture:Value=serde_json::from_str(include_str!("../../../mobile/android/app/src/androidTest/assets/desktop-vault-fixture.json")).unwrap();
        altron_command_engine::persistence::replace(&root.join("password-vault-v1.json"),&fixture["vault"]).unwrap();
        let initial=db::migration_snapshot(&settings).unwrap();let recovery=sync_profile::create(initial,password).unwrap();assert!(recovery.exists());assert_eq!(std::fs::read(path).unwrap(),original);
        let journal=sync_profile::journal().unwrap();assert_eq!(journal.outbox(100).unwrap().len(),1);assert_eq!(journal.outbox(100).unwrap()[0].collection,"notes");
        let backup=sync_profile::export(password).unwrap();assert!(!backup.to_string().contains("fictional-api"));altron_command_engine::persistence::replace(&parent.join("recovery.json"),&backup).unwrap();
        std::fs::write(parent.join("first-key-hash"),Sha256::digest(secure_storage::local_key(false).unwrap())).unwrap();
    }else{
        assert_eq!(stage,"restore");let backup:Value=altron_command_engine::persistence::read(&parent.join("recovery.json")).unwrap();
        assert!(sync_profile::restore_new(&backup,"incorrect password").is_err());assert!(!root.join("sync-v2.sqlite").exists());assert!(!root.join("sync-v2-group.json").exists());
        sync_profile::restore_new(&backup,password).unwrap();let snapshot=sync_profile::read().unwrap();assert_eq!(snapshot["api_keys"]["openai"],"fictional-api-never-transfer");assert_eq!(snapshot["future_setting"]["preserve"],true);jarvis_core::sync_data::validate_vault(&snapshot["sync_vault"]).unwrap();
        assert_ne!(std::fs::read(parent.join("first-key-hash")).unwrap(),Sha256::digest(secure_storage::local_key(false).unwrap()).as_slice());
        assert_eq!(sync_profile::journal().unwrap().recovery_outbox().unwrap().len(),1);
        let updated=db::update_settings(&jarvis_core::Settings::default(),|s|s.set("tts_mode","silero")).unwrap();assert_eq!(updated.api_keys.openai,"fictional-api-never-transfer");assert_eq!(sync_profile::read().unwrap()["future_setting"]["preserve"],true);
    }
}

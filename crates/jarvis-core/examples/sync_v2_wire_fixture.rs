//! Synthetic cross-language fixture; the public key is deliberately fictional.
use jarvis_core::{sync_journal::{Journal,Operation},secure_storage};
use serde_json::{json,Value};
fn main(){
    let args:Vec<_>=std::env::args().collect();let key=[42;32];
    if args.len()==2{let fixture:Value=serde_json::from_slice(&std::fs::read(&args[1]).unwrap()).unwrap();let op:Operation=serde_json::from_value(fixture["operation"].clone()).unwrap();let decoded=secure_storage::open_with_key(&op.sealed,&op.aad(),&key).unwrap();assert_eq!(decoded,fixture["value"]);println!("PASS: Android operation decrypted by Rust with authenticated metadata");return;}
    let root=tempfile::tempdir().unwrap();let mut journal=Journal::open(&root.path().join("test.sqlite"),key,"a".repeat(64)).unwrap();
    let value=json!({"id":"interop-note","title":"Проверка / Unicode","text":"Вымышленное: кавычки \" и \\","updatedAt":"2026-10-12T12:00:00Z"});
    let operation=journal.local_edit("notes","interop-note",Some(value.clone()),vec![]).unwrap();
    let recovery=jarvis_core::sync_profile::recovery_seal(&json!({"fictional":"Совместимость"}),"fictional Unicode пароль 2026").unwrap();
    let fixture=json!({"key":key.to_vec(),"operation":operation,"value":value,"recovery":recovery,"password":"fictional Unicode пароль 2026"});
    std::fs::write("mobile/android/app/src/androidTest/assets/desktop-sync-v2-fixture.json",serde_json::to_vec_pretty(&fixture).unwrap()).unwrap();
}

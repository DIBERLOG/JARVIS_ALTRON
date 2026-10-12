//! Opt-in v2 profile. OS unlock for personal data; password only for recovery exports.
use std::{collections::BTreeMap,path::{Path,PathBuf},num::NonZeroU32};
use serde::{Serialize,Deserialize};
use serde_json::{Value,json};
use rand::RngCore;
use base64::{Engine,engine::general_purpose::STANDARD as B64};
use crate::{secure_storage,sync_journal::{Journal,Operation}};
use altron_command_engine::persistence;

#[derive(Clone,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Group{pub version:u32,pub group_id:String,pub device_id:String,pub epoch:u32,pub keys:BTreeMap<u32,[u8;32]>,pub peer:Option<Value>,#[serde(default)] pub revoked_devices:Vec<String>}
pub fn random_id()->String{let mut b=[0;32];rand::rngs::OsRng.fill_bytes(&mut b);b.iter().map(|b|format!("{b:02x}")).collect()}
pub fn random_key()->[u8;32]{let mut b=[0;32];rand::rngs::OsRng.fill_bytes(&mut b);b}
pub fn root()->Result<PathBuf,String>{crate::APP_CONFIG_DIR.get().cloned().ok_or("Профиль не инициализирован".into())}
pub fn enabled()->bool{root().is_ok_and(|r|r.join("sync-v2-group.json").exists())}
pub fn load_at(root:&Path)->Result<Group,String>{
    let raw:Value=persistence::read(&root.join("sync-v2-group.json")).map_err(|_|"Не удалось прочитать ключи группы")?;
    let group:Group=serde_json::from_value(secure_storage::open(&raw,"sync-v2-group")?).map_err(|_|"Повреждены ключи группы")?;
    if group.version!=2||group.group_id.len()!=64||group.device_id.len()!=64||!group.keys.contains_key(&group.epoch)||group.epoch==0{return Err("Повреждена группа".into());}Ok(group)
}
pub fn save_at(root:&Path,group:&Group)->Result<(),String>{
    let lock=std::fs::OpenOptions::new().create(true).truncate(false).read(true).write(true).open(root.join("sync-v2-group.lock")).map_err(|_|"Не удалось заблокировать ключи группы")?;
    lock.lock().map_err(|_|"Не удалось заблокировать ключи группы")?;
    if root.join("sync-v2-group.json").exists(){let previous=load_at(root)?;
        if previous.group_id!=group.group_id||previous.device_id!=group.device_id||previous.epoch>group.epoch||previous.keys.iter().any(|(e,k)|group.keys.get(e)!=Some(k))||previous.revoked_devices.iter().any(|id|!group.revoked_devices.contains(id)){return Err("Отказ откату или замене ключей/доверия группы".into());}}
    let sealed=secure_storage::seal(&serde_json::to_value(group).map_err(|_|"Ошибка группы")?,"sync-v2-group")?;
    persistence::replace(&root.join("sync-v2-group.json"),&sealed).map_err(|_|"Не удалось сохранить группу".into())
}
pub fn journal_at(root:&Path,group:&Group)->Result<Journal,String>{
    let mut journal=Journal::open(&root.join("sync-v2.sqlite"),secure_storage::local_key(false)?,group.device_id.clone())?;
    journal.transport_keys(group.keys.clone(),group.epoch)?;Ok(journal)
}
pub fn journal()->Result<Journal,String>{let root=root()?;journal_at(&root,&load_at(&root)?)}
pub fn update<F>(initial:&Value,change:F)->Result<Value,String>where F:FnOnce(&mut Value)->Result<(),String>{journal()?.update_snapshot(initial,change)}
pub fn read()->Result<Value,String>{journal()?.snapshot()?.ok_or("Отсутствует снимок v2".into())}

const ITERATIONS:u32=600_000;
pub fn recovery_seal(payload:&Value,password:&str)->Result<Value,String>{
    if password.chars().count()<12||password.len()>1024{return Err("Пароль восстановления: от 12 символов".into());}
    let mut salt=[0;16];rand::rngs::OsRng.fill_bytes(&mut salt);let mut key=[0;32];
    ring::pbkdf2::derive(ring::pbkdf2::PBKDF2_HMAC_SHA256,NonZeroU32::new(ITERATIONS).unwrap(),&salt,password.as_bytes(),&mut key);
    let sealed=secure_storage::seal_with_key(payload,"sync-recovery-v2",&key);key.fill(0);
    Ok(json!({"format":"jarvis-sync-recovery","version":2,"iterations":ITERATIONS,"salt":B64.encode(salt),"sealed":sealed?}))
}
pub fn recovery_open(backup:&Value,password:&str)->Result<Value,String>{
    if backup["format"]!="jarvis-sync-recovery"||backup["version"]!=2||backup["iterations"]!=ITERATIONS||backup.as_object().is_none_or(|v|v.len()!=5)||password.len()>1024{return Err("Неизвестный формат восстановления".into());}
    let salt:[u8;16]=B64.decode(backup["salt"].as_str().ok_or("Нет salt")?).map_err(|_|"Повреждён salt")?.try_into().map_err(|_|"Повреждён salt")?;let mut key=[0;32];
    ring::pbkdf2::derive(ring::pbkdf2::PBKDF2_HMAC_SHA256,NonZeroU32::new(ITERATIONS).unwrap(),&salt,password.as_bytes(),&mut key);
    let value=secure_storage::open_with_key(&backup["sealed"],"sync-recovery-v2",&key);key.fill(0);value
}
pub fn create(initial:Value,password:&str)->Result<PathBuf,String>{
    let root=root()?;if enabled()||root.join("sync-v2.sqlite").exists(){return Err("Профиль v2 уже существует; исходные данные сохранены".into());}
    let group=Group{version:2,group_id:random_id(),device_id:random_id(),epoch:1,keys:BTreeMap::from([(1,random_key())]),peer:None,revoked_devices:vec![]};
    prepare_group(initial,password,group)
}
pub fn prepare_group(initial:Value,password:&str,group:Group)->Result<PathBuf,String>{
    let root=root()?;if enabled()||root.join("sync-v2.sqlite").exists(){return Err("Профиль v2 уже существует".into());}
    if group.version!=2||!group.keys.contains_key(&group.epoch){return Err("Некорректные ключи группы".into());}
    // A verified, password-protected copy precedes any migration. Never delete the old source.
    let mut recovery_group=group.clone();recovery_group.peer=None;let backup=recovery_seal(&json!({"group":recovery_group,"snapshot":initial,"history":[]}),password)?;
    let verified=recovery_open(&backup,password)?;if verified["snapshot"]!=initial{return Err("Проверка резервной копии не прошла".into());}
    let path=root.join(format!("sync-v2-recovery-{}.json",chrono::Utc::now().timestamp_millis()));
    persistence::replace(&path,&backup).map_err(|_|"Не удалось сохранить восстановление")?;
    let _=secure_storage::local_key(true)?;let mut journal=journal_at(&root,&group)?;journal.update_snapshot(&initial,|_|Ok(()))?;
    save_at(&root,&group)?;Ok(path)
}
pub fn export(password:&str)->Result<Value,String>{
    let root=root()?;let mut group=load_at(&root)?;let mut journal=journal_at(&root,&group)?;group.peer=None;
    let (snapshot,history)=journal.backup_state()?;recovery_seal(&json!({"group":group,"snapshot":snapshot,"history":history}),password)
}
pub fn restore_new(backup:&Value,password:&str)->Result<(),String>{
    let root=root()?;if enabled()||root.join("sync-v2.sqlite").exists(){return Err("Восстановление разрешено только в новом пустом профиле".into());}
    let mut payload=recovery_open(backup,password)?;
    if !payload["snapshot"].is_object()||!payload["snapshot"]["center_data"].is_string()||!payload["snapshot"]["chat_history"].is_string(){return Err("В копии отсутствуют личные данные; ничего не заменено".into());}
    let mut portable=serde_json::to_value(crate::Settings::default()).map_err(|_|"Не удалось подготовить профиль")?;
    portable.as_object_mut().unwrap().extend(payload["snapshot"].as_object().unwrap().clone());payload["snapshot"]=portable;
    let _:crate::Settings=serde_json::from_value(payload["snapshot"].clone()).map_err(|_|"Некорректный профиль копии")?;
    let mut group:Group=serde_json::from_value(payload["group"].clone()).map_err(|_|"Некорректная группа копии")?;
    let history:Vec<Operation>=serde_json::from_value(payload["history"].clone()).map_err(|_|"Некорректный журнал копии")?;
    crate::sync_data::records(&payload["snapshot"])?;
    // New installation has a new identity and requires fresh pairing; bearer tokens are not restored.
    group.device_id=random_id();group.peer=None;
    let temporary=root.join(format!("restore-{}",random_id()));std::fs::create_dir_all(&temporary).map_err(|_|"Не удалось подготовить восстановление")?;
    let result=(||{
        let _=secure_storage::local_key(true)?;let mut journal=journal_at(&temporary,&group)?;
        journal.restore_snapshot(&payload["snapshot"],&history)?;
        drop(journal);save_at(&temporary,&group)?;
        std::fs::rename(temporary.join("sync-v2.sqlite"),root.join("sync-v2.sqlite")).map_err(|_|"Не удалось установить журнал восстановления")?;
        std::fs::rename(temporary.join("sync-v2-group.json"),root.join("sync-v2-group.json")).map_err(|_|"Не удалось установить ключи восстановления")?;Ok(())
    })();let _=std::fs::remove_dir(&temporary);result
}

#[cfg(test)]mod tests{
    use super::*;
    #[test]fn recovery_wrong_password_tamper_and_new_local_key(){
        let original=json!({"groupKey":[7,8,9],"fictional_secret":"only-test","snapshot":{"center_data":"{\"notes\":[],\"reminders\":[],\"birthdays\":[],\"habits\":[]}"}});
        let backup=recovery_seal(&original,"fictional recovery password").unwrap();assert!(!backup.to_string().contains("only-test"));
        assert_eq!(recovery_open(&backup,"fictional recovery password").unwrap(),original);assert!(recovery_open(&backup,"incorrect password").is_err());
        let mut broken=backup;broken["salt"]=json!(B64.encode([0;16]));assert!(recovery_open(&broken,"fictional recovery password").is_err());
    }
}

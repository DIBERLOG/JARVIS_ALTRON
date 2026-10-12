//! OS-backed local encryption. No plaintext fallback or exported device key.
use base64::{Engine,engine::general_purpose::STANDARD as B64};
use ring::aead::{Aad,LessSafeKey,Nonce,UnboundKey,AES_256_GCM};
use serde_json::{Value,json};
use rand::RngCore;
use sha2::{Digest,Sha256};
use once_cell::sync::OnceCell;

static KEY:OnceCell<[u8;32]>=OnceCell::new();
static KEY_IO:parking_lot::Mutex<()>=parking_lot::Mutex::new(());
pub fn local_key(create:bool)->Result<[u8;32],String>{
    let _guard=KEY_IO.lock();if let Some(key)=KEY.get(){return Ok(*key);}
    #[cfg(any(windows,target_os="macos",target_os="linux"))]{
        let directory=crate::APP_CONFIG_DIR.get().ok_or("Каталог защищённого хранения не инициализирован")?;
        std::fs::create_dir_all(directory).map_err(|_|"Не удалось открыть каталог защищённого хранения")?;
        let canonical=directory.canonicalize().map_err(|_|"Не удалось определить профиль хранения")?;
        let digest=Sha256::digest(canonical.to_string_lossy().as_bytes());let name:String=digest.iter().map(|b|format!("{b:02x}")).collect();
        // Cross-process first-start lock avoids creating two different keys.
        let lock=std::fs::OpenOptions::new().create(true).truncate(false).read(true).write(true).open(directory.join("secure-key-v1.lock")).map_err(|_|"Не удалось заблокировать создание ключа")?;
        lock.lock().map_err(|_|"Не удалось заблокировать создание ключа")?;
        let entry=keyring::Entry::new("JARVIS_ALTRON.local-data.v1",&name).map_err(|_|"Защищённое хранилище ОС недоступно")?;
        let secret=match entry.get_secret(){
            Ok(secret)=>secret,
            Err(keyring::Error::NoEntry) if create=>{let mut key=[0u8;32];rand::rngs::OsRng.fill_bytes(&mut key);entry.set_secret(&key).map_err(|_|"ОС не разрешила сохранить ключ; открытый текст не записан")?;entry.get_secret().map_err(|_|"Не удалось проверить сохранённый ключ")?},
            Err(keyring::Error::NoEntry)=>return Err("Ключ защищённых данных отсутствует. Нужна резервная копия и восстановление; новый ключ не создан".into()),
            Err(_)=>return Err("Разблокируйте защищённое хранилище ОС. Данные не перезаписаны".into()),
        };
        let key:[u8;32]=secret.try_into().map_err(|_|"Некорректный ключ защищённого хранения")?;let _=KEY.set(key);Ok(key)
    }
    #[cfg(not(any(windows,target_os="macos",target_os="linux")))]{let _=create;Err("На этой платформе защищённое хранение ещё не подключено".into())}
}
pub fn seal(value:&Value,purpose:&str)->Result<Value,String>{seal_with_key(value,purpose,&local_key(true)?)}
pub fn open(value:&Value,purpose:&str)->Result<Value,String>{open_with_key(value,purpose,&local_key(false)?)}
pub fn is_sealed(value:&Value)->bool{value.get("format")==Some(&json!("jarvis-protected-json"))}
pub fn seal_with_key(value:&Value,purpose:&str,key:&[u8;32])->Result<Value,String>{
    let cipher=LessSafeKey::new(UnboundKey::new(&AES_256_GCM,key).map_err(|_|"Некорректный ключ")?);
    let mut nonce=[0u8;12];rand::rngs::OsRng.fill_bytes(&mut nonce);
    let mut data=serde_json::to_vec(value).map_err(|_|"Не удалось подготовить данные")?;
    if data.len()+16>48*1024*1024{data.fill(0);return Err("Данные превышают лимит защищённого файла 48 МБ".into());}
    cipher.seal_in_place_append_tag(Nonce::assume_unique_for_key(nonce),Aad::from(format!("jarvis-protected-json:1:{purpose}")),&mut data).map_err(|_|"Не удалось зашифровать данные")?;
    Ok(json!({"format":"jarvis-protected-json","version":1,"nonce":B64.encode(nonce),"data":B64.encode(data)}))
}
pub fn open_with_key(value:&Value,purpose:&str,key:&[u8;32])->Result<Value,String>{
    if !is_sealed(value)||value["version"]!=1||value.as_object().is_none_or(|v|v.len()!=4){return Err("Неизвестный формат защищённых данных".into());}
    let nonce:[u8;12]=B64.decode(value["nonce"].as_str().ok_or("Отсутствует nonce")?).map_err(|_|"Повреждён nonce")?.try_into().map_err(|_|"Некорректная длина nonce")?;
    let raw=value["data"].as_str().ok_or("Отсутствуют зашифрованные данные")?;if raw.len()>64*1024*1024{return Err("Защищённый файл слишком большой".into());}
    let mut data=B64.decode(raw).map_err(|_|"Повреждены зашифрованные данные")?;
    let cipher=LessSafeKey::new(UnboundKey::new(&AES_256_GCM,key).map_err(|_|"Некорректный ключ")?);
    let plain=cipher.open_in_place(Nonce::assume_unique_for_key(nonce),Aad::from(format!("jarvis-protected-json:1:{purpose}")),&mut data).map_err(|_|"Неверный ключ, назначение файла или повреждённые данные; исходный файл сохранён")?;
    let parsed=serde_json::from_slice(plain).map_err(|_|"Расшифрованные данные имеют некорректный формат".to_string());data.fill(0);parsed
}
#[cfg(test)]mod tests{use super::*;
    #[cfg(windows)]
    #[test]fn native_store_roundtrips_only_a_random_test_entry(){
        struct Cleanup(keyring::Entry);impl Drop for Cleanup{fn drop(&mut self){let _=self.0.delete_credential();}}
        let mut name=[0u8;16];rand::rngs::OsRng.fill_bytes(&mut name);let name:String=name.iter().map(|b|format!("{b:02x}")).collect();
        let entry=Cleanup(keyring::Entry::new("JARVIS_ALTRON.TEST_ONLY",&name).unwrap());let fake_key=[77u8;32];entry.0.set_secret(&fake_key).unwrap();assert_eq!(entry.0.get_secret().unwrap(),fake_key);
    }
    #[test]fn settings_migration_and_failed_decryption_preserve_file(){
        use altron_command_engine::persistence;
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("settings.json");let key=[44u8;32];let original=json!({"api_key":"fake-private-key","notes":["fake-personal-note"],"future_setting":true});persistence::replace(&path,&original).unwrap();
        let saved:Value=persistence::transaction_with_codec(&path,&json!({}),|value:&mut Value|{value["theme"]=json!("dark");Ok(())},Ok,|value|seal_with_key(value,"settings",&key)).unwrap();assert_eq!(saved["future_setting"],true);
        let bytes=std::fs::read(&path).unwrap();assert!(!String::from_utf8_lossy(&bytes).contains("fake-private"));assert!(!String::from_utf8_lossy(&bytes).contains("fake-personal"));
        let result=persistence::transaction_with_codec(&path,&json!({}),|_:&mut Value|Ok(()),|value|open_with_key(&value,"settings",&[45u8;32]),|value|seal_with_key(value,"settings",&key));assert!(result.is_err());assert_eq!(std::fs::read(&path).unwrap(),bytes);
        let sealed:Value=persistence::read(&path).unwrap();assert_eq!(open_with_key(&sealed,"settings",&key).unwrap()["notes"][0],"fake-personal-note");
    }
    #[test]fn encrypted_json_rejects_wrong_key_purpose_and_tampering(){let key=[17u8;32];let original=json!({"note":"fictional-private-note","apiKey":"fictional-only"});let sealed=seal_with_key(&original,"test",&key).unwrap();assert!(!sealed.to_string().contains("fictional"));assert_eq!(open_with_key(&sealed,"test",&key).unwrap(),original);assert!(open_with_key(&sealed,"test",&[18;32]).is_err());assert!(open_with_key(&sealed,"different",&key).is_err());let mut corrupt=sealed.clone();corrupt["nonce"]=json!(B64.encode([0u8;12]));assert!(open_with_key(&corrupt,"test",&key).is_err());}
}

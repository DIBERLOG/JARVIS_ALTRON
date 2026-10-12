//! Opt-in local TLS service. Only allowlisted personal data; never settings/API keys.
use std::{collections::{BTreeMap,BTreeSet},io::{Read,Write},net::{Ipv4Addr,TcpListener,TcpStream},path::{Path,PathBuf},sync::{Arc,atomic::{AtomicBool,AtomicUsize,Ordering}},time::{Duration,Instant}};
use base64::{Engine,engine::general_purpose::STANDARD as B64};
use parking_lot::Mutex;
use rand::RngCore;
use serde::{Serialize,Deserialize};
use serde_json::{Value,json};
use sha2::{Digest,Sha256};
use subtle::ConstantTimeEq;
use tauri::Manager;

const MAX_BODY:usize=16*1024*1024;
fn random()->String {let mut b=[0u8;32];rand::rngs::OsRng.fill_bytes(&mut b);hex(&b)}
fn hex(b:&[u8])->String {b.iter().map(|n|format!("{n:02x}")).collect()}
fn digest(s:&str)->String {hex(&Sha256::digest(s.as_bytes()))}
fn equal(a:&str,b:&str)->bool {a.as_bytes().ct_eq(b.as_bytes()).into()}
fn private(ip:Ipv4Addr)->bool {ip.is_private()&&!ip.is_unspecified()&&!ip.is_loopback()}
fn atomic<T:Serialize>(path:&Path,value:&T)->Result<(),String>{
    let mut file=tempfile::NamedTempFile::new_in(path.parent().ok_or("Нет папки данных")?).map_err(|_|"Не удалось подготовить сохранение")?;
    serde_json::to_writer(file.as_file_mut(),value).map_err(|_|"Не удалось записать данные")?;
    file.as_file_mut().sync_all().map_err(|_|"Не удалось сохранить данные")?;
    file.persist(path).map_err(|_|"Не удалось заменить данные")?;Ok(())
}
fn load<T:serde::de::DeserializeOwned>(p:&Path)->Result<T,String>{serde_json::from_slice(&std::fs::read(p).map_err(|_|"Не удалось прочитать данные синхронизации")?).map_err(|_|"Повреждены данные синхронизации; исходный файл сохранён".into())}
#[derive(Serialize,Deserialize,Clone)]
struct Device {id:String,name:String,token_hash:String,last_sync:Option<String>}
#[derive(Serialize,Deserialize,Clone)]
struct Conflict {id:String,area:String,collection:String,item_id:String,local:Value,remote:Value}
#[derive(Serialize,Deserialize,Default)]
struct Saved {devices:Vec<Device>,conflicts:Vec<Conflict>,allow_vault:bool}
struct Pending {id:String,name:String,code:String,expires:Instant,token:Option<String>}
struct Runtime {stop:Arc<AtomicBool>,address:String,fingerprint:String,secret:String,expires:Instant,pending:Vec<Pending>}
struct Inner {saved:Saved,runtime:Option<Runtime>,directory:Option<PathBuf>}
pub struct LanSync {inner:Arc<Mutex<Inner>>}
impl Default for LanSync {fn default()->Self {Self{inner:Arc::new(Mutex::new(Inner{saved:Saved::default(),runtime:None,directory:None}))}}}
fn init(inner:&mut Inner,app:&tauri::AppHandle)->Result<(),String>{
    if inner.directory.is_none(){let dir=app.path().app_data_dir().map_err(|_|"Нет папки приложения")?.join("lan-sync");std::fs::create_dir_all(&dir).map_err(|_|"Не удалось создать папку синхронизации")?;
        let path=dir.join("devices.json");if path.exists(){inner.saved=load(&path)?;}inner.directory=Some(dir);}
    Ok(())
}
fn save(inner:&Inner)->Result<(),String>{atomic(&inner.directory.as_ref().ok_or("Синхронизация не инициализирована")?.join("devices.json"),&inner.saved)}
fn status(inner:&Inner)->Value{
    let active=inner.runtime.as_ref().filter(|r|!r.stop.load(Ordering::Relaxed));
    json!({"running":active.is_some(),"address":active.map(|r|r.address.clone()),"allowVault":inner.saved.allow_vault,
        "qr":active.filter(|r|Instant::now()<r.expires).map(|r|format!("jarvis-pair://v1?host={}&port={}&fingerprint={}&secret={}",r.address.split(':').next().unwrap_or(""),r.address.split(':').nth(1).unwrap_or(""),r.fingerprint,r.secret)),
        "pending":active.map(|r|r.pending.iter().filter(|p|Instant::now()<p.expires&&p.token.is_none()).map(|p|json!({"id":p.id,"name":p.name,"code":p.code})).collect::<Vec<_>>()).unwrap_or_default(),
        "devices":inner.saved.devices.iter().map(|d|json!({"id":d.id,"name":d.name,"lastSync":d.last_sync})).collect::<Vec<_>>(),
        "conflicts":inner.saved.conflicts.iter().map(|c|json!({"id":c.id,"area":c.area,"collection":c.collection,"itemId":c.item_id,"title":c.local.get("title").or(c.local.get("name")).and_then(Value::as_str).unwrap_or(&c.item_id)})).collect::<Vec<_>>()})
}
#[tauri::command]
pub fn lan_sync_status(app:tauri::AppHandle,state:tauri::State<'_,LanSync>)->Result<Value,String>{let mut i=state.inner.lock();init(&mut i,&app)?;Ok(status(&i))}
#[tauri::command]
pub fn lan_sync_addresses()->Vec<String>{if_addrs::get_if_addrs().unwrap_or_default().into_iter().filter_map(|a|match a.ip(){std::net::IpAddr::V4(ip) if private(ip)=>Some(ip.to_string()),_=>None}).collect::<BTreeSet<_>>().into_iter().collect()}
#[derive(Serialize,Deserialize)]
struct Certificate {cert:String,key:String}
#[tauri::command]
pub fn lan_sync_start(app:tauri::AppHandle,state:tauri::State<'_,LanSync>,ip:String,port:u16)->Result<Value,String>{
    let ip:Ipv4Addr=ip.parse().map_err(|_|"Укажите локальный IPv4-адрес")?;
    if !private(ip)||port<1024||!lan_sync_addresses().contains(&ip.to_string()){return Err("Выберите IPv4 этой машины в локальной сети и порт 1024–65535".into());}
    let mut i=state.inner.lock();init(&mut i,&app)?;
    if i.runtime.as_ref().is_some_and(|r|!r.stop.load(Ordering::Relaxed)){return Err("Сначала остановите текущий обмен".into());}
    let cert_path=i.directory.as_ref().unwrap().join("tls.json");
    let material:Certificate=if cert_path.exists(){load(&cert_path)?}else{let rcgen::CertifiedKey{cert,key_pair}=rcgen::generate_simple_self_signed(vec![ip.to_string()]).map_err(|_|"Не удалось создать сертификат")?;
        let material=Certificate{cert:B64.encode(cert.der()),key:B64.encode(key_pair.serialize_der())};atomic(&cert_path,&material)?;material};
    let cert=B64.decode(material.cert).map_err(|_|"Повреждён сертификат")?;
    let key=B64.decode(material.key).map_err(|_|"Повреждён ключ сертификата")?;
    let tls=rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider())).with_safe_default_protocol_versions().map_err(|_|"Ошибка версии TLS")?.with_no_client_auth()
        .with_single_cert(vec![rustls::pki_types::CertificateDer::from(cert.clone())],rustls::pki_types::PrivatePkcs8KeyDer::from(key).into()).map_err(|_|"Некорректный сертификат TLS")?;
    let listener=TcpListener::bind((ip,port)).map_err(|_|"Порт занят или адрес недоступен. Проверьте сеть и брандмауэр")?;listener.set_nonblocking(true).map_err(|_|"Не удалось запустить службу")?;
    let stop=Arc::new(AtomicBool::new(false));
    i.runtime=Some(Runtime{stop:stop.clone(),address:format!("{ip}:{port}"),fingerprint:hex(&Sha256::digest(&cert)),secret:random(),expires:Instant::now()+Duration::from_secs(120),pending:vec![]});
    let response=status(&i);let inner=state.inner.clone();let settings=app.state::<crate::AppState>().settings.clone();let app_dir=app.path().app_data_dir().map_err(|_|"Нет папки приложения")?;
    let config=Arc::new(tls);let inflight=Arc::new(AtomicUsize::new(0));
    std::thread::spawn(move||{while !stop.load(Ordering::Relaxed){match listener.accept(){Ok((socket,peer))=>{
        if !matches!(peer.ip(),std::net::IpAddr::V4(v) if private(v))||inflight.load(Ordering::Relaxed)>=4 {continue;}
        inflight.fetch_add(1,Ordering::Relaxed);let inner=inner.clone();let settings=settings.clone();let config=config.clone();let count=inflight.clone();let dir=app_dir.clone();
        std::thread::spawn(move||{let _=connection(socket,config,inner,settings,dir);count.fetch_sub(1,Ordering::Relaxed);});
    },Err(e) if e.kind()==std::io::ErrorKind::WouldBlock=>std::thread::sleep(Duration::from_millis(100)),Err(_)=>break,}}});
    Ok(response)
}
#[tauri::command]
pub fn lan_sync_stop(state:tauri::State<'_,LanSync>){if let Some(r)=state.inner.lock().runtime.take(){r.stop.store(true,Ordering::Relaxed);}}
#[tauri::command]
pub fn lan_sync_qr(state:tauri::State<'_,LanSync>)->Result<Value,String>{let mut i=state.inner.lock();let r=i.runtime.as_mut().ok_or("Сначала включите обмен")?;r.secret=random();r.expires=Instant::now()+Duration::from_secs(120);r.pending.clear();Ok(status(&i))}
#[tauri::command]
pub fn lan_sync_approve(state:tauri::State<'_,LanSync>,id:String)->Result<(),String>{
    let mut i=state.inner.lock();let p=i.runtime.as_mut().and_then(|r|r.pending.iter_mut().find(|p|p.id==id&&p.token.is_none()&&Instant::now()<p.expires)).ok_or("Запрос истёк")?;
    let token=random();p.token=Some(token.clone());let device=Device{id:p.id.clone(),name:p.name.clone(),token_hash:digest(&token),last_sync:None};i.saved.devices.push(device);save(&i)
}
#[tauri::command]
pub fn lan_sync_revoke(state:tauri::State<'_,LanSync>,id:String)->Result<(),String>{let mut i=state.inner.lock();i.saved.devices.retain(|d|d.id!=id);if let Some(r)=i.runtime.as_mut(){r.pending.retain(|p|p.id!=id);}save(&i)}
#[tauri::command]
pub fn lan_sync_vault_enabled(state:tauri::State<'_,LanSync>,enabled:bool)->Result<(),String>{let mut i=state.inner.lock();i.saved.allow_vault=enabled;save(&i)}

fn connection(socket:TcpStream,config:Arc<rustls::ServerConfig>,inner:Arc<Mutex<Inner>>,settings:jarvis_core::SettingsManager,dir:PathBuf)->Result<(),()> {
    socket.set_read_timeout(Some(Duration::from_secs(8))).map_err(|_|())?;socket.set_write_timeout(Some(Duration::from_secs(8))).map_err(|_|())?;
    let session=rustls::ServerConnection::new(config).map_err(|_|())?;let mut stream=rustls::StreamOwned::new(session,socket);
    let mut header=Vec::new();while !header.ends_with(b"\r\n\r\n"){let mut b=[0];if stream.read_exact(&mut b).is_err(){return Err(());}header.push(b[0]);if header.len()>8192{return Err(());}}
    let text=std::str::from_utf8(&header).map_err(|_|())?;let mut lines=text.split("\r\n");let first=lines.next().ok_or(())?;
    let mut parts=first.split_whitespace();if parts.next()!=Some("POST"){return Err(());}let route=parts.next().ok_or(())?;
    if parts.next()!=Some("HTTP/1.1")||parts.next().is_some(){return Err(());}
    let mut length=None;let mut authorization=None;
    for line in lines.filter(|s|!s.is_empty()){let (name,value)=line.split_once(':').ok_or(())?;let value=value.trim();
        if name.eq_ignore_ascii_case("transfer-encoding"){return Err(());}
        if name.eq_ignore_ascii_case("content-length"){if length.is_some(){return Err(());}length=Some(value.parse::<usize>().map_err(|_|())?);}
        if name.eq_ignore_ascii_case("authorization"){if authorization.is_some(){return Err(());}authorization=Some(value.strip_prefix("Bearer ").ok_or(())?);}}
    let length=length.ok_or(())?;if length>MAX_BODY{return Err(());}let mut bytes=vec![0;length];stream.read_exact(&mut bytes).map_err(|_|())?;
    let body:Value=serde_json::from_slice(&bytes).map_err(|_|())?;
    let (status,value)=handle(route,authorization.unwrap_or(""),body,&inner,&settings,&dir);
    let bytes=serde_json::to_vec(&value).map_err(|_|())?;if bytes.len()>MAX_BODY{return Err(());}
    write!(stream,"HTTP/1.1 {status} Response\r\nContent-Type: application/json\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",bytes.len()).map_err(|_|())?;stream.write_all(&bytes).map_err(|_|())?;stream.flush().map_err(|_|())
}
fn handle(route:&str,token:&str,body:Value,inner:&Arc<Mutex<Inner>>,settings:&jarvis_core::SettingsManager,dir:&Path)->(u16,Value){
    let mut i=inner.lock();if i.runtime.as_ref().is_none_or(|r|r.stop.load(Ordering::Relaxed)){return (503,json!({"error":"Обмен остановлен"}));}
    if route=="/pair"||route=="/pair/status"{
        let r=i.runtime.as_mut().unwrap();if Instant::now()>r.expires||!equal(token,&r.secret){return (401,json!({"error":"QR-код истёк. Создайте новый на компьютере"}));}
        r.pending.retain(|p|Instant::now()<p.expires);
        if route=="/pair"{
            if r.pending.len()>=4{return (429,json!({"error":"Слишком много запросов сопряжения"}));}
            let name=body["name"].as_str().unwrap_or("").trim();if name.is_empty()||name.len()>100||name.chars().any(char::is_control){return (400,json!({"error":"Некорректное имя телефона"}));}
            let id=random();let code=format!("{:06}",rand::random::<u32>()%1_000_000);r.pending.push(Pending{id:id.clone(),name:name.into(),code:code.clone(),expires:Instant::now()+Duration::from_secs(120),token:None});return (202,json!({"id":id,"code":code}));
        }
        let id=body["id"].as_str().unwrap_or("");if let Some(p)=r.pending.iter_mut().find(|p|equal(&p.id,id)){
            if let Some(token)=p.token.take(){p.expires=Instant::now();return (200,json!({"token":token,"deviceId":p.id}));}return (202,json!({"pending":true}));}
        return (404,json!({"error":"Сопряжение не найдено или уже использовано"}));
    }
    if route!="/sync"{return (404,json!({"error":"Неизвестная операция"}));}
    let hash=digest(token);let Some(device)=i.saved.devices.iter().position(|d|equal(&d.token_hash,&hash))else{return (401,json!({"error":"Телефон отключён. Подключите заново по QR-коду"}));};
    match exchange(&mut i,&body,settings,dir){Ok(value)=>{i.saved.devices[device].last_sync=Some(chrono::Utc::now().to_rfc3339());if save(&i).is_err(){return (500,json!({"error":"Изменения могли сохраниться, но статус не записан. Повторите синхронизацию"}));}(200,value)},Err(error)=>(400,json!({"error":error}))}
}
fn empty_center()->Value {json!({"notes":[],"reminders":[],"birthdays":[],"habits":[]})}
fn valid_center(v:&Value)->Result<(),String>{
    if !v.is_object(){return Err("Некорректные данные Центра".into());}
    for key in ["notes","reminders","birthdays","habits"] {valid_records(&v[key])?;}
    if let Some(t)=v.get("training").filter(|t|!t.is_null()){for key in ["exercises","plans","sessions"]{valid_records(&t[key])?;}}
    Ok(())
}
fn valid_records(v:&Value)->Result<(),String>{let a=v.as_array().ok_or("Ожидается список записей")?;if a.len()>10000{return Err("Слишком много записей".into());}
    let mut ids=BTreeSet::new();for item in a{let id=item["id"].as_str().ok_or("У записи нет ID")?;if id.is_empty()||id.len()>128||!ids.insert(id){return Err("Повторный или некорректный ID".into());}}Ok(())}
fn valid_vault(v:&Value)->Result<(),String>{
    if v.is_null(){return Ok(());}if v["version"]!=1||B64.decode(v["salt"].as_str().unwrap_or("")).map(|b|b.len()).unwrap_or(0)!=16{return Err("Неизвестный формат сейфа".into());}
    for key in ["passwordKey","recoveryKey","cards"]{let iv=B64.decode(v[key]["iv"].as_str().unwrap_or("")).map_err(|_|"Повреждён сейф")?;let data=B64.decode(v[key]["data"].as_str().unwrap_or("")).map_err(|_|"Повреждён сейф")?;
        if iv.len()!=12||data.len()<16||((key=="passwordKey"||key=="recoveryKey")&&data.len()!=48){return Err("Повреждён сейф".into());}}
    Ok(())
}
fn merge_value(base:&Value,local:&Value,remote:&Value,area:&str,collection:&str,id:&str,conflicts:&mut Vec<Conflict>)->Value{
    if local==remote||remote==base{return local.clone();}if local==base{return remote.clone();}
    if !conflicts.iter().any(|c|c.area==area&&c.collection==collection&&c.item_id==id&&c.local==*local&&c.remote==*remote){conflicts.push(Conflict{id:random(),area:area.into(),collection:collection.into(),item_id:id.into(),local:local.clone(),remote:remote.clone()});}local.clone()
}
fn merge_records(base:&Value,local:&Value,remote:&Value,area:&str,key:&str,conflicts:&mut Vec<Conflict>)->Value{
    let map=|v:&Value|->BTreeMap<String,Value>{v.as_array().into_iter().flatten().filter_map(|r|r["id"].as_str().map(|id|(id.to_owned(),r.clone()))).collect()};
    let (b,l,r)=(map(base),map(local),map(remote));let ids:BTreeSet<_>=b.keys().chain(l.keys()).chain(r.keys()).cloned().collect();
    Value::Array(ids.iter().filter_map(|id|{let value=merge_value(b.get(id).unwrap_or(&Value::Null),l.get(id).unwrap_or(&Value::Null),r.get(id).unwrap_or(&Value::Null),area,key,id,conflicts);(!value.is_null()).then_some(value)}).collect())
}
fn merge_center(base:&Value,local:&Value,remote:&Value,conflicts:&mut Vec<Conflict>)->Value{
    let mut output=local.clone();for key in ["notes","reminders","birthdays","habits"]{output[key]=merge_records(&base[key],&local[key],&remote[key],"center",key,conflicts);}
    output["training"]=merge_value(&base["training"],&local["training"],&remote["training"],"center","training","training",conflicts);output
}
fn vault_at(dir:&Path)->Result<Value,String>{let p=dir.join("password-vault-v1.json");if p.exists(){load(&p)}else{Ok(Value::Null)}}
fn backup(dir:&Path,label:&str,data:&Value)->Result<(),String>{let backups=dir.join("lan-sync/backups");std::fs::create_dir_all(&backups).map_err(|_|"Не удалось создать резервную копию")?;atomic(&backups.join(format!("{label}-{}-{}.json",chrono::Utc::now().timestamp_millis(),&random()[..8])),data)}
fn exchange(i:&mut Inner,body:&Value,settings:&jarvis_core::SettingsManager,dir:&Path)->Result<Value,String>{
    if body["version"]!=1{return Err("Неизвестная версия протокола".into());}let base=&body["base"];let remote=&body["center"];valid_center(base)?;valid_center(remote)?;
    let mut conflicts=i.saved.conflicts.clone();if conflicts.len()>200{return Err("Сначала разрешите конфликты на компьютере".into());}
    let updated=jarvis_core::db::update_settings(&settings.lock(),|s|{
        let local=if s.center_data.is_empty(){empty_center()}else{serde_json::from_str(&s.center_data).map_err(|_|"Повреждены данные Центра компьютера")?};valid_center(&local)?;
        let merged=merge_center(base,&local,remote,&mut conflicts);if merged!=local{backup(dir,"center",&local)?;s.set("center_data",&merged.to_string())?;}
        if body["syncChats"]==true{let b=&body["baseChats"];let r=&body["chats"];valid_records(b)?;valid_records(r)?;let l:Value=serde_json::from_str(if s.chat_history.is_empty(){"[]"}else{&s.chat_history}).map_err(|_|"Повреждена история компьютера")?;
            let merged=merge_records(b,&l,r,"chats","chats",&mut conflicts);if merged!=l{backup(dir,"chats",&l)?;s.set("chat_history_v1",&merged.to_string())?;}}
        Ok(())
    })?;*settings.lock_mut()=updated;
    // Persist conflict copies before returning or changing a vault; a failed sync is retry-safe.
    i.saved.conflicts=conflicts;save(i)?;
    let mut vault=Value::Null;let mut vault_conflict=false;
    if i.saved.allow_vault&&body["syncVault"]==true{
        let _vault_guard=super::vault::VAULT_IO.lock();
        let local=vault_at(dir)?;valid_vault(&local)?;let remote=&body["vault"];let base=&body["baseVault"];valid_vault(remote)?;valid_vault(base)?;
        if remote.is_null(){vault=local;}else if local==*base||local==*remote{if local!=*remote{if !local.is_null(){backup(dir,"vault-encrypted",&local)?;}atomic(&dir.join("password-vault-v1.json"),remote)?;}vault=remote.clone();}
        else if remote==base{vault=local;}else{vault_conflict=true;vault=local;backup(dir,"vault-conflict-encrypted",remote)?;}
    }
    let center:Value=serde_json::from_str(&settings.read("center_data").unwrap_or_default()).unwrap_or_else(|_|empty_center());
    let chats:Value=if body["syncChats"]==true{serde_json::from_str(&settings.read("chat_history_v1").unwrap_or_default()).unwrap_or(json!([]))}else{json!([])};
    Ok(json!({"version":1,"center":center,"chats":chats,"vault":vault,"vaultEnabled":i.saved.allow_vault,"vaultConflict":vault_conflict,"conflicts":i.saved.conflicts}))
}
#[tauri::command]
pub fn lan_sync_resolve(app:tauri::AppHandle,state:tauri::State<'_,LanSync>,id:String,use_phone:bool)->Result<(),String>{
    let mut i=state.inner.lock();let index=i.saved.conflicts.iter().position(|c|c.id==id).ok_or("Конфликт не найден")?;let c=i.saved.conflicts[index].clone();
    if use_phone{let manager=&app.state::<crate::AppState>().settings;let updated=jarvis_core::db::update_settings(&manager.lock(),|s|{
        let key=if c.area=="chats"{"chat_history_v1"}else{"center_data"};let mut data:Value=serde_json::from_str(&s.get(key).unwrap_or_default()).map_err(|_|"Данные изменены или повреждены")?;
        if c.collection=="training"{if data["training"]!=c.local{return Err("Запись изменена после конфликта. Сначала синхронизируйте заново".into());}data["training"]=c.remote.clone();}
        else {let array=if c.area=="chats"{data.as_array_mut()}else{data[&c.collection].as_array_mut()}.ok_or("Список повреждён")?;let position=array.iter().position(|x|x["id"].as_str()==Some(&c.item_id));let current=position.map(|p|array[p].clone()).unwrap_or(Value::Null);if current!=c.local{return Err("Запись изменена после конфликта. Сначала синхронизируйте заново".into());}
            if let Some(p)=position{array.remove(p);}if !c.remote.is_null(){array.push(c.remote.clone());}}
        backup(&app.path().app_data_dir().map_err(|_|"Нет папки приложения")?,"before-conflict",&serde_json::from_str::<Value>(&s.get(key).unwrap_or_default()).map_err(|_|"Данные повреждены")?)?;s.set(key,&data.to_string())
    })?;*manager.lock_mut()=updated;}
    i.saved.conflicts.remove(index);save(&i)
}
#[cfg(test)]
mod tests {use super::*;
    #[test] fn three_way_preserves_edits_and_deletions(){let b=json!([{"id":"a","title":"old"}]);let l=b.clone();let r=json!([]);let mut c=vec![];assert_eq!(merge_records(&b,&l,&r,"center","notes",&mut c),json!([]));assert!(c.is_empty());
        let l=json!([{"id":"a","title":"pc"}]);let r=json!([{"id":"a","title":"phone"}]);assert_eq!(merge_records(&b,&l,&r,"center","notes",&mut c),l);assert_eq!(c.len(),1);assert_eq!(c[0].remote,r[0]);}
    #[test] fn only_private_addresses_and_sealed_vaults(){assert!(private("192.168.1.2".parse().unwrap()));assert!(!private("8.8.8.8".parse().unwrap()));assert!(!private("127.0.0.1".parse().unwrap()));assert!(valid_vault(&json!({"version":1,"cards":{"data":"plaintext"}})).is_err());assert!(valid_records(&json!([{"id":"a"},{"id":"a"}])).is_err());}
}

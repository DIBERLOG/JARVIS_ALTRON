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
pub(super) fn private(ip:Ipv4Addr)->bool {ip.is_private()&&!ip.is_unspecified()&&!ip.is_loopback()}
fn atomic<T:Serialize>(path:&Path,value:&T)->Result<(),String>{
    let mut file=tempfile::NamedTempFile::new_in(path.parent().ok_or("Нет папки данных")?).map_err(|_|"Не удалось подготовить сохранение")?;
    let value=serde_json::to_value(value).map_err(|_|"Не удалось подготовить данные")?;
    // Vault already has its own compatible ciphertext format and master password.
    let sealed=if path.file_name().is_some_and(|name|name=="password-vault-v1.json"){value}else{jarvis_core::secure_storage::seal(&value,"lan-sync-local")?};
    serde_json::to_writer(file.as_file_mut(),&sealed).map_err(|_|"Не удалось записать данные")?;
    file.as_file_mut().sync_all().map_err(|_|"Не удалось сохранить данные")?;
    file.persist(path).map_err(|_|"Не удалось заменить данные")?;Ok(())
}
fn load<T:serde::de::DeserializeOwned>(p:&Path)->Result<T,String>{if std::fs::metadata(p).map_err(|_|"Не удалось прочитать данные синхронизации")?.len()>70*1024*1024{return Err("Файл синхронизации слишком большой".into());}let value:Value=serde_json::from_slice(&std::fs::read(p).map_err(|_|"Не удалось прочитать данные синхронизации")?).map_err(|_|"Повреждены данные синхронизации; исходный файл сохранён")?;let protected=jarvis_core::secure_storage::is_sealed(&value);let value=if protected{jarvis_core::secure_storage::open(&value,"lan-sync-local")?}else{value};let result=serde_json::from_value(value.clone()).map_err(|_|"Повреждены данные синхронизации; исходный файл сохранён")?;if !protected&&!p.file_name().is_some_and(|name|name=="password-vault-v1.json"){atomic(p,&value)?;}Ok(result)}
#[derive(Serialize,Deserialize,Clone)]
struct Device {id:String,name:String,token_hash:String,last_sync:Option<String>,#[serde(default)] protocol:u32}
#[derive(Serialize,Deserialize,Clone)]
struct Conflict {id:String,area:String,collection:String,item_id:String,local:Value,remote:Value}
#[derive(Serialize,Deserialize,Default,Clone)]
struct Saved {#[serde(default)] remote_requests:Vec<RemoteRequest>,#[serde(default)] rotation_pending:bool,devices:Vec<Device>,conflicts:Vec<Conflict>,allow_vault:bool}
#[derive(Serialize,Deserialize,Clone)]
struct RemoteRequest { id:String,device_id:String,command_id:String,expires_at:i64,status:String }
fn remote_catalog()->Vec<Value>{super::get_commands_list().into_iter().filter(|c|jarvis_core::commands::remote_allowed(&c.id)).map(|c|json!({"id":c.id,"title":c.description})).collect()}
fn enqueue_remote(saved:&mut Saved,device_id:&str,body:&Value,now:i64)->Result<Value,String>{
    let id=body["id"].as_str().ok_or("Нет номера запроса")?;let command=body["command_id"].as_str().ok_or("Нет команды")?;
    if id.len()!=36||!id.bytes().enumerate().all(|(n,b)|if [8,13,18,23].contains(&n){b==b'-'}else{b.is_ascii_hexdigit()}){return Err("Некорректный номер запроса".into());}
    if let Some(r)=saved.remote_requests.iter().find(|r|r.id==id){if r.device_id!=device_id||r.command_id!=command{return Err("Номер запроса уже занят".into());}return Ok(json!({"id":r.id,"status":r.status}));}
    let expires=body["expires_at"].as_i64().ok_or("Нет срока команды")?;if expires<=now||expires>now+60{return Err("Команда истекла. Проверьте время на телефоне".into());}
    if !jarvis_core::commands::remote_allowed(command){return Err("Команда не разрешена для телефона".into());}
    saved.remote_requests.retain(|r|r.expires_at>now-600);
    if saved.remote_requests.len()>=128{return Err("Слишком много команд. Подождите несколько минут".into());}
    saved.remote_requests.push(RemoteRequest{id:id.into(),device_id:device_id.into(),command_id:command.into(),expires_at:expires,status:"queued".into()});Ok(json!({"id":id,"status":"queued"}))
}
#[tauri::command]
pub fn lan_sync_take_commands(app:tauri::AppHandle,state:tauri::State<'_,LanSync>)->Result<Value,String>{let mut i=state.inner.lock();if i.directory.is_none(){return Ok(json!([]));}init(&mut i,&app)?;if i.runtime.as_ref().is_none_or(|r|r.stop.load(Ordering::Relaxed)){return Ok(json!([]));}let now=chrono::Utc::now().timestamp();let mut candidate=i.saved.clone();if i.saved.rotation_pending{return Err("Ротация ключей не завершена".into());}let revoked=jarvis_core::sync_profile::root().and_then(|r|jarvis_core::sync_profile::load_at(&r))?.revoked_devices;let mut result=vec![];for r in &mut candidate.remote_requests {if r.status!="queued"{continue;}if r.expires_at<=now||!candidate.devices.iter().any(|d|d.id==r.device_id)||revoked.contains(&r.device_id){r.status="expired".into();continue;}if result.len()>=4{continue;}r.status="claimed".into();result.push(json!({"id":r.id,"command_id":r.command_id,"expires_at":r.expires_at}));}
    if !result.is_empty()||candidate.remote_requests.iter().zip(&i.saved.remote_requests).any(|(a,b)|a.status!=b.status){atomic(&i.directory.as_ref().unwrap().join("devices.json"),&candidate)?;i.saved=candidate;}Ok(json!(result))}
#[tauri::command]
pub fn lan_sync_command_result(state:tauri::State<'_,LanSync>,id:String,status:String)->Result<(),String>{if !matches!(status.as_str(),"executed"|"shown"|"failed"|"uncertain"){return Err("Некорректный статус".into());}let mut i=state.inner.lock();let mut candidate=i.saved.clone();if let Some(r)=candidate.remote_requests.iter_mut().find(|r|r.id==id&&r.status=="claimed"){r.status=status;atomic(&i.directory.as_ref().ok_or("Нет профиля")?.join("devices.json"),&candidate)?;i.saved=candidate;}Ok(())}
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
fn finish_pending_rotation(inner:&mut Inner)->Result<(),String>{
    let root=jarvis_core::sync_profile::root()?;let mut group=jarvis_core::sync_profile::load_at(&root)?;
    group.epoch=group.epoch.checked_add(1).ok_or("Исчерпаны версии ключей")?;group.keys.insert(group.epoch,jarvis_core::sync_profile::random_key());jarvis_core::sync_profile::save_at(&root,&group)?;
    inner.saved.rotation_pending=false;if let Err(error)=save(inner){inner.saved.rotation_pending=true;return Err(error);}Ok(())
}
fn status(inner:&Inner)->Value{
    let preferences=if jarvis_core::sync_profile::enabled(){jarvis_core::sync_profile::read().ok()}else{None};
    let active=inner.runtime.as_ref().filter(|r|!r.stop.load(Ordering::Relaxed));
    json!({"version":2,"prepared":jarvis_core::sync_profile::enabled(),"restored":jarvis_core::sync_profile::enabled()&&jarvis_core::sync_profile::journal().is_ok_and(|j|j.can_repair().unwrap_or(false)),"background":super::sync_client::background_status(),"auto":jarvis_core::sync_profile::root().and_then(|r|jarvis_core::sync_profile::load_at(&r)).is_ok_and(|g|g.peer.is_some_and(|p|p["auto"]==true)),"client":jarvis_core::sync_profile::root().and_then(|r|jarvis_core::sync_profile::load_at(&r)).is_ok_and(|g|g.peer.is_some()),"v2Conflicts":v2_conflicts().unwrap_or_default(),"running":active.is_some(),"address":active.map(|r|r.address.clone()),"allowVault":preferences.as_ref().map(|s|s["sync_share_vault"]==true).unwrap_or(inner.saved.allow_vault),"shareChats":preferences.as_ref().is_some_and(|s|s["sync_share_chats"]==true),
        "qr":active.filter(|r|Instant::now()<r.expires).map(|r|format!("jarvis-pair://v2?host={}&port={}&fingerprint={}&secret={}",r.address.split(':').next().unwrap_or(""),r.address.split(':').nth(1).unwrap_or(""),r.fingerprint,r.secret)),
        "pending":active.map(|r|r.pending.iter().filter(|p|Instant::now()<p.expires&&p.token.is_none()).map(|p|json!({"id":p.id,"name":p.name,"code":p.code})).collect::<Vec<_>>()).unwrap_or_default(),
        "devices":inner.saved.devices.iter().map(|d|json!({"id":d.id,"name":d.name,"lastSync":d.last_sync})).collect::<Vec<_>>(),
        "conflicts":inner.saved.conflicts.iter().map(|c|json!({"id":c.id,"area":c.area,"collection":c.collection,"itemId":c.item_id,"title":c.local.get("title").or(c.local.get("name")).and_then(Value::as_str).unwrap_or(&c.item_id),"local":c.local,"phone":c.remote})).collect::<Vec<_>>()})
}
#[tauri::command]
pub fn lan_sync_status(app:tauri::AppHandle,state:tauri::State<'_,LanSync>)->Result<Value,String>{let mut i=state.inner.lock();init(&mut i,&app)?;Ok(status(&i))}
#[tauri::command(async)]
pub fn lan_sync_addresses()->Vec<String>{
    #[cfg(windows)] { windows_lan_addresses() }
    #[cfg(not(windows))] { if_addrs::get_if_addrs().unwrap_or_default().into_iter().filter_map(|a|match a.ip(){std::net::IpAddr::V4(ip) if private(ip)=>Some(ip.to_string()),_=>None}).collect::<BTreeSet<_>>().into_iter().collect() }
}
fn physical_addresses(bytes:&[u8])->Vec<String>{
    #[derive(Deserialize)] struct Address {address:String,physical:bool,up:bool,gateway:bool,metric:u32}
    let mut rows:Vec<Address>=serde_json::from_slice(bytes).unwrap_or_default();
    rows.retain(|row|row.physical&&row.up&&row.address.parse::<Ipv4Addr>().is_ok_and(private));
    rows.sort_by_key(|row|(!row.gateway,row.metric,row.address.clone()));
    let mut seen=BTreeSet::new();rows.into_iter().filter_map(|row|seen.insert(row.address.clone()).then_some(row.address)).collect()
}
#[cfg(windows)]
fn windows_lan_addresses()->Vec<String>{
    use std::{os::windows::process::CommandExt,process::{Command,Stdio}};
    let mut command=Command::new("C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe");
    command.args(["-NoProfile","-NonInteractive","-Command",include_str!("lan_addresses.ps1")]).creation_flags(0x08000000).stdout(Stdio::piped()).stderr(Stdio::null());
    let Ok(mut child)=command.spawn() else{return vec![]};
    let Some(mut stdout)=child.stdout.take() else{return vec![]};
    let reader=std::thread::spawn(move||{let mut bytes=vec![];stdout.by_ref().take(256*1024).read_to_end(&mut bytes).map(|_|bytes)});
    let deadline=Instant::now()+Duration::from_secs(20);
    loop{match child.try_wait(){Ok(Some(exit))=>{let result=reader.join();return if exit.success(){result.ok().and_then(Result::ok).map(|bytes|physical_addresses(&bytes)).unwrap_or_default()}else{vec![]}},Ok(None) if Instant::now()<deadline=>std::thread::sleep(Duration::from_millis(30)),_=>{let _=child.kill();let _=child.wait();return vec![]}}}
}
#[derive(Serialize,Deserialize)]
struct Certificate {cert:String,key:String}
#[tauri::command(async)]
pub fn lan_sync_start(app:tauri::AppHandle,state:tauri::State<'_,LanSync>,ip:String,port:u16)->Result<Value,String>{
    if !jarvis_core::sync_profile::enabled(){return Err("Сначала создайте проверенную копию восстановления Sync v2".into());}
    let root=jarvis_core::sync_profile::root()?;let mut group=jarvis_core::sync_profile::load_at(&root)?;
    if group.peer.is_some(){return Err("Этот профиль подключён к другому хабу".into());}
    let journal=jarvis_core::sync_profile::journal()?;
    if journal.can_repair()?&&!journal.hub_restore_rotated()?{
        group.epoch=group.epoch.checked_add(1).ok_or("Исчерпаны версии ключей")?;group.keys.insert(group.epoch,jarvis_core::sync_profile::random_key());jarvis_core::sync_profile::save_at(&root,&group)?;journal.queue_recovery_history()?;journal.mark_hub_restore_rotated()?;
    }
    let ip:Ipv4Addr=ip.parse().map_err(|_|"Укажите локальный IPv4-адрес")?;
    if !private(ip)||port<1024||!lan_sync_addresses().contains(&ip.to_string()){return Err("Выберите подключение домашней сети и повторите. Виртуальные адаптеры не подходят для телефона.".into());}
    let mut i=state.inner.lock();init(&mut i,&app)?;
    if i.saved.rotation_pending{finish_pending_rotation(&mut i)?;}
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
    let mut i=state.inner.lock();let p=i.runtime.as_ref().and_then(|r|r.pending.iter().find(|p|p.id==id&&p.token.is_none()&&Instant::now()<p.expires)).ok_or("Запрос истёк")?;
    let token=random();let device=Device{id:p.id.clone(),name:p.name.clone(),token_hash:digest(&token),last_sync:None,protocol:2};
    let replacement=i.saved.devices.iter().any(|d|d.id==device.id);
    let mut candidate=i.saved.clone();candidate.devices.retain(|d|d.id!=device.id);candidate.devices.push(device);candidate.rotation_pending|=replacement;
    atomic(&i.directory.as_ref().ok_or("Нет профиля")?.join("devices.json"),&candidate)?;
    i.saved=candidate;
    if i.saved.rotation_pending{finish_pending_rotation(&mut i)?;}
    i.runtime.as_mut().unwrap().pending.iter_mut().find(|p|p.id==id).unwrap().token=Some(token);Ok(())
}
#[tauri::command]
pub fn lan_sync_revoke(state:tauri::State<'_,LanSync>,id:String)->Result<(),String>{
    let mut i=state.inner.lock();if !i.saved.devices.iter().any(|d|d.id==id){return Err("Устройство не найдено".into());}
    let root=jarvis_core::sync_profile::root()?;let mut group=jarvis_core::sync_profile::load_at(&root)?;
    if group.peer.is_some(){return Err("Устройства отзываются на хабе".into());}
    if !group.revoked_devices.contains(&id){
        group.epoch=group.epoch.checked_add(1).ok_or("Исчерпаны версии ключей")?;group.keys.insert(group.epoch,jarvis_core::sync_profile::random_key());group.revoked_devices.push(id.clone());
        // Authorization removal and new key are one authenticated atomic file replacement.
        jarvis_core::sync_profile::save_at(&root,&group)?;
    }
    i.saved.devices.retain(|d|d.id!=id);if let Some(r)=i.runtime.as_mut(){r.pending.retain(|p|p.id!=id);}save(&i)?;
    let _=jarvis_core::sync_relay::Relay::open(&root.join("sync-v2-relay.sqlite"),&group.group_id,group.epoch)?;Ok(())
}
#[tauri::command]
pub fn lan_sync_vault_enabled(state:tauri::State<'_,LanSync>,enabled:bool)->Result<(),String>{let mut i=state.inner.lock();
    if jarvis_core::sync_profile::enabled(){jarvis_core::sync_profile::update(&json!({}),|snapshot|{snapshot["sync_share_vault"]=json!(enabled);Ok(())})?;}
    i.saved.allow_vault=enabled;save(&i)}

fn connection(socket:TcpStream,config:Arc<rustls::ServerConfig>,inner:Arc<Mutex<Inner>>,settings:jarvis_core::SettingsManager,dir:PathBuf)->Result<(),()> {
    let deadline=Instant::now()+Duration::from_secs(20);
    // Windows accepted sockets inherit the listener's nonblocking mode. TLS here
    // uses blocking reads with deadlines; otherwise the first read can abort it.
    socket.set_nonblocking(false).map_err(|_|())?;
    socket.set_read_timeout(Some(Duration::from_secs(8))).map_err(|_|())?;socket.set_write_timeout(Some(Duration::from_secs(8))).map_err(|_|())?;
    let session=rustls::ServerConnection::new(config).map_err(|_|())?;let mut stream=rustls::StreamOwned::new(session,socket);
    let mut header=Vec::new();while !header.ends_with(b"\r\n\r\n"){if Instant::now()>deadline{return Err(());}let mut b=[0];if let Err(_error)=stream.read_exact(&mut b){#[cfg(test)] eprintln!("fixture header/TLS failure after {} bytes: {_error}",header.len());return Err(());}header.push(b[0]);if header.len()>8192{return Err(());}}
    let text=std::str::from_utf8(&header).map_err(|_|())?;let mut lines=text.split("\r\n");let first=lines.next().ok_or(())?;
    let mut parts=first.split_whitespace();if parts.next()!=Some("POST"){return Err(());}let route=parts.next().ok_or(())?;
    if parts.next()!=Some("HTTP/1.1")||parts.next().is_some(){return Err(());}
    let mut length=None;let mut authorization=None;
    for line in lines.filter(|s|!s.is_empty()){let (name,value)=line.split_once(':').ok_or(())?;let value=value.trim();
        if name.eq_ignore_ascii_case("transfer-encoding"){return Err(());}
        if name.eq_ignore_ascii_case("content-length"){if length.is_some(){return Err(());}length=Some(value.parse::<usize>().map_err(|_|())?);}
        if name.eq_ignore_ascii_case("authorization"){if authorization.is_some(){return Err(());}authorization=Some(value.strip_prefix("Bearer ").ok_or(())?);}}
    let length=length.ok_or(())?;if length>MAX_BODY{return Err(());}let mut bytes=vec![0;length];let mut offset=0;while offset<length{let remaining=deadline.saturating_duration_since(Instant::now());if remaining.is_zero(){return Err(());}stream.sock.set_read_timeout(Some(remaining.min(Duration::from_secs(8)))).map_err(|_|())?;let read=stream.read(&mut bytes[offset..]).map_err(|_|())?;if read==0{return Err(());}offset+=read;}
    let body:Value=serde_json::from_slice(&bytes).map_err(|_|())?;
    let (status,value)=handle(route,authorization.unwrap_or(""),body,&inner,&settings,&dir);
    let bytes=serde_json::to_vec(&value).map_err(|_|())?;if bytes.len()>MAX_BODY{return Err(());}
    write!(stream,"HTTP/1.1 {status} Response\r\nContent-Type: application/json\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",bytes.len()).map_err(|_|())?;stream.write_all(&bytes).map_err(|_|())?;stream.flush().map_err(|_|())
}
fn handle(route:&str,token:&str,body:Value,inner:&Arc<Mutex<Inner>>,settings:&jarvis_core::SettingsManager,dir:&Path)->(u16,Value){
    let mut i=inner.lock();if i.runtime.as_ref().is_none_or(|r|r.stop.load(Ordering::Relaxed)){return (503,json!({"error":"Обмен остановлен"}));}
    if i.saved.rotation_pending{return (503,json!({"error":"Нужно завершить ротацию на хабе; выдача ключей и обмен приостановлены"}));}
    if route=="/pair"||route=="/pair/status"{
        if route=="/pair"&&body.get("device_id").is_some(){
            let group=match jarvis_core::sync_profile::root().and_then(|root|jarvis_core::sync_profile::load_at(&root)){Ok(g)=>g,Err(e)=>return (400,json!({"error":e}))};
            let requested=body["device_id"].as_str().unwrap_or("");
            if body["group_id"]!=group.group_id||requested==group.device_id||group.revoked_devices.iter().any(|id|id==requested){return (400,json!({"error":"ID или группа восстановления не допускаются; исходные данные сохранены"}));}
        }
        let r=i.runtime.as_mut().unwrap();if Instant::now()>r.expires||!equal(token,&r.secret){return (401,json!({"error":"QR-код истёк. Создайте новый на компьютере"}));}
        r.pending.retain(|p|Instant::now()<p.expires);
        if route=="/pair"{
            if body["version"]!=2{return (400,json!({"error":"Требуется клиент Sync v2 и новое сопряжение"}));}
            if r.pending.len()>=4{return (429,json!({"error":"Слишком много запросов сопряжения"}));}
            let name=body["name"].as_str().unwrap_or("").trim();if name.is_empty()||name.len()>100||name.chars().any(char::is_control){return (400,json!({"error":"Некорректное имя телефона"}));}
            let id=if let Some(requested)=body["device_id"].as_str(){
                if requested.len()!=64||!requested.bytes().all(|b|b.is_ascii_hexdigit()){return (400,json!({"error":"Некорректный ID восстановления"}));}
                requested.to_string()
            }else{random()};let code=format!("{:06}",rand::random::<u32>()%1_000_000);r.pending.push(Pending{id:id.clone(),name:name.into(),code:code.clone(),expires:Instant::now()+Duration::from_secs(120),token:None});return (202,json!({"id":id,"code":code}));
        }
        let id=body["id"].as_str().unwrap_or("");if let Some(p)=r.pending.iter_mut().find(|p|equal(&p.id,id)){
            if let Some(token)=p.token.as_ref(){
                let group=match jarvis_core::sync_profile::root().and_then(|root|jarvis_core::sync_profile::load_at(&root)){Ok(g)=>g,Err(e)=>return (500,json!({"error":e}))};
                // Pinned TLS + user-approved pairing is the key provisioning channel. Never put keys in QR.
                let result=json!({"version":2,"token":token,"deviceId":p.id,"group":group_wire(&group)});return (200,result);
            }return (202,json!({"pending":true}));}
        return (404,json!({"error":"Сопряжение не найдено или уже использовано"}));
    }
    if !matches!(route,"/v2/sync"|"/v2/recover"|"/v2/keys"|"/v2/commands"|"/v2/command"|"/v2/command/status"){return (400,json!({"error":"Sync v1 отключён в этой LAB-сборке; обновите клиент и выполните новое сопряжение"}));}
    let hash=digest(token);let Some(device)=i.saved.devices.iter().position(|d|d.protocol==2&&equal(&d.token_hash,&hash))else{return (401,json!({"error":"Устройство отключено. Подключите заново по QR-коду"}));};
    let result=(||->Result<Value,String>{
        let root=jarvis_core::sync_profile::root()?;let group=jarvis_core::sync_profile::load_at(&root)?;
        if group.revoked_devices.contains(&i.saved.devices[device].id){return Err("Устройство отозвано; новые ключи недоступны".into());}
        if route=="/v2/commands"{return Ok(json!({"commands":remote_catalog()}));}
        if route=="/v2/command/status"{let id=body["id"].as_str().unwrap_or("");let r=i.saved.remote_requests.iter().find(|r|r.id==id&&r.device_id==i.saved.devices[device].id).ok_or("Запрос не найден")?;return Ok(json!({"id":r.id,"status":r.status,"expired":r.expires_at<chrono::Utc::now().timestamp()}));}
        if route=="/v2/command"{let device_id=i.saved.devices[device].id.clone();let mut candidate=i.saved.clone();let response=enqueue_remote(&mut candidate,&device_id,&body,chrono::Utc::now().timestamp())?;atomic(&i.directory.as_ref().unwrap().join("devices.json"),&candidate)?;i.saved=candidate;return Ok(response);}
        if route=="/v2/keys"{let relay=jarvis_core::sync_relay::Relay::open(&root.join("sync-v2-relay.sqlite"),&group.group_id,group.epoch)?;return Ok(json!({"version":2,"features":["tasks"],"relay_id":relay.identity()?,"group":group_wire(&group)}));}
        if body["version"]!=2||body["group_id"]!=group.group_id{return Err("Несовместимая группа или протокол".into());}
        let mut operations:Vec<jarvis_core::sync_journal::Operation>=serde_json::from_value(body["operations"].clone()).map_err(|_|"Некорректные операции")?;let tasks_supported=body["features"].as_array().is_some_and(|v|v.iter().any(|f|f=="tasks"));if operations.iter().any(|op|op.collection=="tasks")&&!tasks_supported{return Err("Для общих задач обновите клиент до Sync v2 LAB 1.1.2".into());}
        for operation in &operations{
            operation.validate()?;
            let key=group.keys.get(&operation.key_version).ok_or("Неизвестная версия ключа")?;
            let value=jarvis_core::secure_storage::open_with_key(&operation.sealed,&operation.aad(),key)?;
            if operation.deleted{if !value.is_null(){return Err("Удаление содержит данные".into());}}
            else if value.is_null(){return Err("Пустое содержимое без операции удаления".into());}
            else if operation.collection=="vault"{jarvis_core::sync_data::validate_vault(&value)?;}
            else if !operation.collection.starts_with("training.") || matches!(operation.collection.as_str(),"training.exercises"|"training.plans"|"training.sessions"){
                if value["id"].as_str()!=Some(&operation.entity_id){return Err("ID содержимого не совпадает с операцией".into());}
                if operation.collection=="chats"{valid_chats(&json!([value]))?;}
                else if matches!(operation.collection.as_str(),"notes"|"reminders"|"birthdays"|"habits"|"tasks"){let mut check=empty_center();check[&operation.collection]=json!([value]);valid_center(&check)?;}
                else{valid_records(&json!([value]))?;}
            }
        }
        let cursor=body["cursor"].as_u64().ok_or("Некорректный курсор")?;
        let mut relay=jarvis_core::sync_relay::Relay::open(&root.join("sync-v2-relay.sqlite"),&group.group_id,group.epoch)?;
        for operation in &mut operations{
            if let Some(previous)=relay.existing(&operation.op_id)?{
                if previous!=*operation{
                    let a=jarvis_core::secure_storage::open_with_key(&previous.sealed,&previous.aad(),group.keys.get(&previous.key_version).ok_or("Нет ключа принятой операции")?)?;
                    let b=jarvis_core::secure_storage::open_with_key(&operation.sealed,&operation.aad(),group.keys.get(&operation.key_version).ok_or("Нет ключа операции")?)?;
                    let mut metadata=previous.clone();metadata.key_version=operation.key_version;metadata.sealed=operation.sealed.clone();
                    if previous.key_version==operation.key_version||metadata!=*operation||!jarvis_core::sync_data::equivalent(&a,&b){return Err("Повторный ID с другим содержимым".into());}
                    *operation=previous;
                }
            }
        }
        relay.trust(&group.device_id)?;relay.trust(&i.saved.devices[device].id)?;
        // Hub is itself a client: its user edits enter the same durable relay.
        hub_cycle(&mut relay,&group)?;
        let feed=if route=="/v2/recover"{relay.recover(&i.saved.devices[device].id,&group.group_id,cursor,&operations)?}else{relay.exchange(&i.saved.devices[device].id,&group.group_id,cursor,&operations)?};
        hub_cycle(&mut relay,&group)?;
        if feed.items.iter().any(|(_,op)|op.collection=="tasks")&&!tasks_supported{return Err("В группе есть общие задачи; обновите клиент до Sync v2 LAB 1.1.2. Курсор не изменён.".into());}
        Ok(serde_json::to_value(feed).map_err(|_|"Ошибка ответа")?)
    })();
    match result{Ok(value)=>{i.saved.devices[device].last_sync=Some(chrono::Utc::now().to_rfc3339());if let Err(error)=save(&i){return (500,json!({"error":error}));}(200,value)},Err(error)=>(400,json!({"error":error}))}
}
fn empty_center()->Value {json!({"notes":[],"reminders":[],"birthdays":[],"habits":[],"tasks":[]})}
fn group_wire(group:&jarvis_core::sync_profile::Group)->Value{json!({"version":2,"group_id":group.group_id,"epoch":group.epoch,"keys":group.keys})}
fn hub_cycle(relay:&mut jarvis_core::sync_relay::Relay,group:&jarvis_core::sync_profile::Group)->Result<(),String>{
    let relay_id=relay.identity()?;let mut journal=jarvis_core::sync_profile::journal()?;
    for _ in 0..100{
        let recovery=journal.recovery_outbox()?;let outbound=if recovery.is_empty(){journal.outbox(100)?}else{recovery.clone()};let cursor=journal.cursor(&relay_id)?;
        let feed=if recovery.is_empty(){relay.exchange(&group.device_id,&group.group_id,cursor,&outbound)?}else{relay.recover(&group.device_id,&group.group_id,cursor,&outbound)?};
        if !feed.rekey.is_empty(){journal.rewrap_outbox(&feed.rekey)?;continue;}
        journal.receive_snapshot(&relay_id,&feed.items,feed.cursor,&feed.acknowledged)?;
        if feed.items.is_empty()&&outbound.is_empty(){return Ok(());}
    }Err("Слишком большой журнал; повторите обмен для продолжения".into())
}
fn v2_conflicts()->Result<Vec<Value>,String>{
    if !jarvis_core::sync_profile::enabled(){return Ok(vec![]);}
    Ok(jarvis_core::sync_profile::journal()?.all_versions()?.into_iter().filter(|(_,_,v)|v.len()>1).map(|(c,e,v)|json!({"collection":c,"entity":e,"versions":v})).collect())
}
#[tauri::command]
pub fn sync_v2_prepare(state:tauri::State<'_,crate::AppState>,password:String)->Result<String,String>{
    let snapshot=jarvis_core::db::migration_snapshot(&state.settings.lock())?;
    Ok(jarvis_core::sync_profile::create(snapshot,&password)?.display().to_string())
}
#[tauri::command]
pub fn sync_v2_export(password:String)->Result<String,String>{Ok(jarvis_core::sync_profile::export(&password)?.to_string())}
#[tauri::command]
pub fn sync_v2_restore(backup:String,password:String)->Result<(),String>{
    if backup.len()>70*1024*1024{return Err("Копия слишком большая".into());}
    jarvis_core::sync_profile::restore_new(&serde_json::from_str(&backup).map_err(|_|"Некорректная копия")?,&password)
}
#[tauri::command]
pub fn sync_v2_resolve(collection:String,entity:String,parents:Vec<String>,value:Option<Value>)->Result<(),String>{
    jarvis_core::sync_profile::journal()?.resolve_snapshot(&collection,&entity,parents,value)
}
fn valid_center(v:&Value)->Result<(),String>{
    if !v.is_object(){return Err("Некорректные данные Центра".into());}
    for key in ["notes","reminders","birthdays","habits"] {valid_records(&v[key])?;} if let Some(tasks)=v.get("tasks"){valid_records(tasks)?;for task in tasks.as_array().unwrap(){let text=task["text"].as_str().ok_or("Нет текста задачи")?;if text.trim().is_empty()||text.len()>4000||!task["done"].is_boolean(){return Err("Некорректная задача".into());}}}
    for note in v["notes"].as_array().unwrap(){if !note["title"].is_string()||!note["text"].is_string()||!valid_date(&note["updatedAt"]){return Err("Некорректная заметка".into());}}
    for reminder in v["reminders"].as_array().unwrap(){if !reminder["title"].is_string()||!reminder["done"].is_boolean()||!valid_date(&reminder["dueAt"])||reminder.get("repeat").is_some_and(|r|!matches!(r.as_str(),Some("none"|"daily"))){return Err("Некорректное напоминание".into());}}
    for birthday in v["birthdays"].as_array().unwrap(){let day=birthday["day"].as_u64().unwrap_or(0) as u32;let month=birthday["month"].as_u64().unwrap_or(0) as u32;if !birthday["name"].is_string()||chrono::NaiveDate::from_ymd_opt(2024,month,day).is_none(){return Err("Некорректная дата рождения".into());}if let Some(y)=birthday.get("year"){if !y.as_u64().is_some_and(|n|(1..=9999).contains(&n)){return Err("Некорректный год дня рождения".into());}}}
    for habit in v["habits"].as_array().unwrap(){if !habit["title"].is_string()||!habit["entries"].as_object().is_some_and(|e|e.values().all(|n|n.as_f64().is_some_and(|n|n.is_finite()&&n>=0.)))||!habit["target"].as_u64().is_some_and(|n|n>0&&n<=10000){return Err("Некорректная привычка".into());}}
    if let Some(t)=v.get("training").filter(|t|!t.is_null()){for key in ["exercises","plans","sessions"]{valid_records(&t[key])?;}}
    Ok(())
}
fn valid_date(v:&Value)->bool {v.as_str().is_some_and(|s|chrono::DateTime::parse_from_rfc3339(s).is_ok()||chrono::NaiveDateTime::parse_from_str(s,"%Y-%m-%dT%H:%M").is_ok()||chrono::NaiveDateTime::parse_from_str(s,"%Y-%m-%dT%H:%M:%S%.f").is_ok())}
fn valid_chats(v:&Value)->Result<(),String>{valid_records(v)?;for c in v.as_array().unwrap(){if !c["title"].as_str().is_some_and(|s|s.len()<=800)||!valid_date(&c["updatedAt"])||!c["messages"].as_array().is_some_and(|m|m.len()<=10000&&m.iter().all(|m|matches!(m["role"].as_str(),Some("user"|"assistant"))&&m["content"].as_str().is_some_and(|s|s.len()<=1_000_000))){return Err("Некорректная история чата".into());}}Ok(())}
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
    let mut output=local.clone();for key in ["notes","reminders","birthdays","habits","tasks"]{output[key]=merge_records(&base[key],&local[key],&remote[key],"center",key,conflicts);}
    if local["training"].is_object()||remote["training"].is_object(){let mut training=if local["training"].is_object(){local["training"].clone()}else{json!({})};for key in ["exercises","plans","sessions"]{training[key]=merge_records(&base["training"][key],&local["training"][key],&remote["training"][key],"center",&format!("training.{key}"),conflicts);}for key in ["profile","morningExercise","chargeSettings","chargeLevels"]{training[key]=merge_value(&base["training"][key],&local["training"][key],&remote["training"][key],"center",&format!("training.{key}"),key,conflicts);}output["training"]=training;}
    output
}
fn normalize_center(mut data:Value)->Result<Value,String>{if !data.is_object(){return Err("Некорректные данные Центра".into());}for key in ["notes","reminders","birthdays","habits","tasks"]{if data.get(key).is_none(){data[key]=json!([]);}}Ok(data)}
fn merge_center_checked(base:&Value,local:&Value,remote:&Value)->Result<Value,String>{let mut conflicts=vec![];let result=merge_center(base,local,remote,&mut conflicts);if !conflicts.is_empty(){return Err("Эта запись одновременно изменена в другом окне или на устройстве. Ваши изменения не перезаписали другую версию. Скопируйте несохранённый текст, обновите раздел и сравните версии.".into());}Ok(result)}
#[tauri::command]
pub fn center_save_data(state:tauri::State<'_,crate::AppState>,base:String,data:String)->Result<String,String>{
    if base.len()>16*1024*1024||data.len()>16*1024*1024{return Err("Данные Центра превышают 16 МБ".into());}
    let base=normalize_center(serde_json::from_str(&base).map_err(|_|"Некорректная базовая версия Центра")?)?;
    let remote=normalize_center(serde_json::from_str(&data).map_err(|_|"Некорректные изменения Центра")?)?;valid_center(&base)?;valid_center(&remote)?;
    let mut merged=String::new();let mut guard=state.settings.lock_mut();let updated=jarvis_core::db::update_settings(&guard,|settings|{
        let local=if settings.center_data.is_empty(){empty_center()}else{normalize_center(serde_json::from_str(&settings.center_data).map_err(|_|"Данные Центра на диске повреждены")?)?};valid_center(&local)?;
        let result=merge_center_checked(&base,&local,&remote)?;
        merged=result.to_string();settings.set("center_data",&merged)
    })?;*guard=updated;Ok(merged)
}
#[tauri::command]
pub fn chat_save_history(state:tauri::State<'_,crate::AppState>,base:String,data:String)->Result<String,String>{
    if base.len()>20*1024*1024||data.len()>20*1024*1024{return Err("История слишком большая".into());}
    let base:Value=serde_json::from_str(&base).map_err(|_|"Некорректная базовая история")?;let remote:Value=serde_json::from_str(&data).map_err(|_|"Некорректная история")?;valid_chats(&base)?;valid_chats(&remote)?;
    let mut result=String::new();let mut guard=state.settings.lock_mut();let updated=jarvis_core::db::update_settings(&guard,|settings|{
        let local:Value=serde_json::from_str(if settings.chat_history.is_empty(){"[]"}else{&settings.chat_history}).map_err(|_|"Повреждена история на диске")?;let mut conflicts=vec![];
        let merged=merge_records(&base,&local,&remote,"chats","chats",&mut conflicts);if !conflicts.is_empty(){return Err("Эта беседа одновременно изменена; текст не перезаписан. Сохраните несохранённый ответ и обновите историю.".into());}
        result=merged.to_string();settings.set("chat_history_v1",&result)
    })?;*guard=updated;Ok(result)
}
fn backup(dir:&Path,label:&str,data:&Value)->Result<(),String>{let backups=dir.join("lan-sync/backups");std::fs::create_dir_all(&backups).map_err(|_|"Не удалось создать резервную копию")?;atomic(&backups.join(format!("{label}-{}-{}.json",chrono::Utc::now().timestamp_millis(),&random()[..8])),data)}
#[tauri::command]
pub fn lan_sync_resolve(app:tauri::AppHandle,state:tauri::State<'_,LanSync>,id:String,use_phone:bool)->Result<(),String>{
    let mut i=state.inner.lock();let index=i.saved.conflicts.iter().position(|c|c.id==id).ok_or("Конфликт не найден")?;let c=i.saved.conflicts[index].clone();
    if use_phone{let manager=&app.state::<crate::AppState>().settings;let updated=jarvis_core::db::update_settings(&manager.lock(),|s|{
        let key=if c.area=="chats"{"chat_history_v1"}else{"center_data"};let mut data:Value=serde_json::from_str(&s.get(key).unwrap_or_default()).map_err(|_|"Данные изменены или повреждены")?;
        let training_key=c.collection.strip_prefix("training.");
        if training_key.is_some_and(|k|!matches!(k,"exercises"|"plans"|"sessions")){let key=training_key.unwrap();if data["training"][key]!=c.local{return Err("Данные изменены после конфликта; синхронизируйте заново".into());}data["training"][key]=c.remote.clone();}
        else {let array=if c.area=="chats"{data.as_array_mut()}else if let Some(key)=training_key{data["training"][key].as_array_mut()}else{data[&c.collection].as_array_mut()}.ok_or("Список повреждён")?;let position=array.iter().position(|x|x["id"].as_str()==Some(&c.item_id));let current=position.map(|p|array[p].clone()).unwrap_or(Value::Null);if current!=c.local{return Err("Запись изменена после конфликта. Сначала синхронизируйте заново".into());}
            if let Some(p)=position{array.remove(p);}if !c.remote.is_null(){array.push(c.remote.clone());}}
        backup(&app.path().app_data_dir().map_err(|_|"Нет папки приложения")?,"before-conflict",&serde_json::from_str::<Value>(&s.get(key).unwrap_or_default()).map_err(|_|"Данные повреждены")?)?;s.set(key,&data.to_string())
    })?;*manager.lock_mut()=updated;}
    i.saved.conflicts.remove(index);save(&i)
}
#[cfg(test)]
mod tests {use super::*;
    #[test] fn phone_addresses_exclude_virtual_and_disconnected_adapters(){
        let rows=json!([
            {"address":"172.29.48.1","physical":false,"up":true,"gateway":false,"metric":0},
            {"address":"192.168.8.1","physical":false,"up":true,"gateway":true,"metric":1},
            {"address":"10.0.0.2","physical":true,"up":false,"gateway":true,"metric":1},
            {"address":"8.8.8.8","physical":true,"up":true,"gateway":true,"metric":1},
            {"address":"192.168.4.2","physical":true,"up":true,"gateway":false,"metric":1},
            {"address":"192.168.1.77","physical":true,"up":true,"gateway":true,"metric":45},
            {"address":"192.168.1.77","physical":true,"up":true,"gateway":true,"metric":45}
        ]);
        assert_eq!(physical_addresses(&serde_json::to_vec(&rows).unwrap()),vec!["192.168.1.77","192.168.4.2"]);
        assert!(physical_addresses(b"bad response").is_empty());
    }
    #[cfg(windows)] #[test] fn windows_address_discovery_returns_only_physical_candidates(){
        let addresses=windows_lan_addresses();
        if let Ok(expected)=std::env::var("JARVIS_EXPECT_LAN_ADDRESS"){assert!(addresses.contains(&expected),"Expected active home interface missing: {addresses:?}");}
        let all=if_addrs::get_if_addrs().unwrap();
        for address in addresses{assert!(all.iter().any(|adapter|adapter.ip().to_string()==address));assert!(private(address.parse().unwrap()));}
    }
    #[test]
    #[ignore = "Explicit Android emulator wire test; fictional profile only"]
    fn android_tls_pairing_wire_fixture(){
        let output=std::env::var_os("JARVIS_ANDROID_WIRE_FIXTURE").expect("fixture output path");
        let directory=tempfile::tempdir().unwrap();
        let rcgen::CertifiedKey{cert,key_pair}=rcgen::generate_simple_self_signed(vec!["10.0.2.2".into()]).unwrap();
        let config=rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider())).with_safe_default_protocol_versions().unwrap().with_no_client_auth().with_single_cert(vec![cert.der().clone()],rustls::pki_types::PrivatePkcs8KeyDer::from(key_pair.serialize_der()).into()).unwrap();
        let listener=TcpListener::bind((Ipv4Addr::LOCALHOST,0)).unwrap();listener.set_nonblocking(true).unwrap();let port=listener.local_addr().unwrap().port();let secret=random();
        let state=LanSync::default();state.inner.lock().runtime=Some(Runtime{stop:Arc::new(AtomicBool::new(false)),address:format!("10.0.2.2:{port}"),fingerprint:hex(&Sha256::digest(cert.der())),secret:secret.clone(),expires:Instant::now()+Duration::from_secs(180),pending:vec![]});
        std::fs::write(output,serde_json::to_vec(&json!({"host":"10.0.2.2","port":port,"fingerprint":hex(&Sha256::digest(cert.der())),"secret":secret})).unwrap()).unwrap();
        let config=Arc::new(config);let settings=jarvis_core::SettingsManager::new(jarvis_core::Settings::default());let deadline=Instant::now()+Duration::from_secs(180);let mut accepted=0;
        while accepted<6 && Instant::now()<deadline{match listener.accept(){Ok((socket,_))=>{accepted+=1;let result=connection(socket,config.clone(),state.inner.clone(),settings.clone(),directory.path().to_owned());println!("fixture connection {accepted}: {}",if result.is_ok(){"HTTP response sent"}else{"TLS or HTTP rejected"});},Err(e) if e.kind()==std::io::ErrorKind::WouldBlock=>std::thread::sleep(Duration::from_millis(50)),Err(e)=>panic!("{e}"),}}
        assert_eq!(accepted,6);assert_eq!(state.inner.lock().runtime.as_ref().unwrap().pending.len(),2,"Exactly one successful pairing POST");
    }
    #[test] fn remote_commands_deduplicate_and_reject_changed_expired_or_unsafe_requests(){
        let mut saved=Saved::default();let body=json!({"id":"12345678-1234-1234-1234-123456789abc","command_id":"calculator_open","expires_at":1060});
        for _ in 0..10 {assert_eq!(enqueue_remote(&mut saved,"phone-a",&body,1000).unwrap()["status"],"queued");}assert_eq!(saved.remote_requests.len(),1);
        saved.remote_requests[0].status="claimed".into();assert_eq!(enqueue_remote(&mut saved,"phone-a",&body,1001).unwrap()["status"],"claimed");
        assert!(enqueue_remote(&mut saved,"phone-b",&body,1001).is_err());let mut changed=body.clone();changed["command_id"]=json!("browser_open");assert!(enqueue_remote(&mut saved,"phone-a",&changed,1001).is_err());
        let wire=serde_json::to_string(&saved).unwrap();let mut restarted:Saved=serde_json::from_str(&wire).unwrap();assert_eq!(enqueue_remote(&mut restarted,"phone-a",&body,1002).unwrap()["status"],"claimed");
        changed["id"]=json!("abcdefab-1234-1234-1234-123456789abc");changed["command_id"]=json!("computer_restart");assert!(enqueue_remote(&mut saved,"phone-a",&changed,1000).is_err());changed["command_id"]=json!("calculator_open");assert!(enqueue_remote(&mut saved,"phone-a",&changed,1061).is_err());assert!(!jarvis_core::commands::remote_allowed("repeat_command"));
    }

    #[cfg(windows)]
    #[test]fn v2_gateway_three_profiles_java_ciphertext_rekey_and_revocation(){
        use jarvis_core::{sync_profile::{self,Group},sync_journal::{Journal,Operation},sync_relay::Feed};
        let temporary=tempfile::tempdir().unwrap();let root=temporary.path();jarvis_core::APP_CONFIG_DIR.set(root.to_path_buf()).unwrap();
        let name=hex(&Sha256::digest(root.canonicalize().unwrap().to_string_lossy().as_bytes()));
        struct Cleanup(keyring::Entry);impl Drop for Cleanup{fn drop(&mut self){assert!(self.0.delete_credential().is_ok());}}
        let _cleanup=Cleanup(keyring::Entry::new("JARVIS_ALTRON.local-data.v1",&name).unwrap());
        let mut group=Group{version:2,group_id:random(),device_id:random(),epoch:1,keys:BTreeMap::from([(1,[42;32])]),peer:None,revoked_devices:vec![]};
        let mut settings=jarvis_core::Settings::default();settings.api_keys.openai="fictional-not-synchronized".into();settings.center_data=json!({"notes":[{"id":"pc1","title":"Only test","text":"offline first PC","updatedAt":"2026-10-12T12:00:00Z"}],"reminders":[],"birthdays":[],"habits":[],"tasks":[]}).to_string();
        sync_profile::prepare_group(serde_json::to_value(&settings).unwrap(),"fictional backup password",group.clone()).unwrap();
        let directory=root.join("lan-sync");std::fs::create_dir_all(&directory).unwrap();
        let android_id="a".repeat(64);let pc2_id="b".repeat(64);
        let state=LanSync{inner:Arc::new(Mutex::new(Inner{saved:Saved{devices:vec![Device{id:android_id.clone(),name:"fictional Android".into(),token_hash:digest("fictional-android-token"),last_sync:None,protocol:2},Device{id:pc2_id.clone(),name:"fictional PC2".into(),token_hash:digest("fictional-pc2-token"),last_sync:None,protocol:2}],..Saved::default()},runtime:Some(Runtime{stop:Arc::new(AtomicBool::new(false)),address:"192.168.1.2:47832".into(),fingerprint:random(),secret:random(),expires:Instant::now()+Duration::from_secs(120),pending:vec![]}),directory:Some(directory)}))};
        let manager=jarvis_core::SettingsManager::new(settings);
        let empty=json!({"center_data":empty_center().to_string(),"chat_history":"[]"});
        let mut pc2=Journal::open(&root.join("pc2.sqlite"),[21;32],pc2_id).unwrap();pc2.transport_keys(group.keys.clone(),1).unwrap();pc2.update_snapshot(&empty,|_|Ok(())).unwrap();
        let mut android=Journal::open(&root.join("android.sqlite"),[22;32],android_id.clone()).unwrap();android.transport_keys(group.keys.clone(),1).unwrap();android.update_snapshot(&empty,|_|Ok(())).unwrap();
        let pc2_op=pc2.local_edit("notes","pc2",Some(json!({"id":"pc2","title":"Only test","text":"offline second PC","updatedAt":"2026-10-12T12:00:00Z"})),vec![]).unwrap();
        // Use actual Java ciphertext when supplied by the isolated emulator run.
        let raw=std::env::var_os("JARVIS_ANDROID_V2_FIXTURE").map(|path|std::fs::read_to_string(path).unwrap()).unwrap_or_else(||include_str!("../../../../mobile/android/app/src/androidTest/assets/desktop-sync-v2-fixture.json").into());let fixture:Value=serde_json::from_str(&raw).unwrap();
        let mut java_op:Operation=serde_json::from_value(fixture["operation"].clone()).unwrap();java_op.device_id=android_id;
        // Both fixtures bind these same metadata fields through AEAD.
        let exchange=|token:&str,cursor:u64,operations:Vec<Operation>|{
            let (status,body)=handle("/v2/sync",token,json!({"version":2,"group_id":group.group_id,"cursor":cursor,"operations":operations}),&state.inner,&manager,root);
            assert_eq!(status,200,"{body}");serde_json::from_value::<Feed>(body).unwrap()
        };
        let feed=exchange("fictional-pc2-token",0,vec![pc2_op]);pc2.receive_snapshot(&group.group_id,&feed.items,feed.cursor,&feed.acknowledged).unwrap();
        let feed=exchange("fictional-android-token",0,vec![java_op]);android.receive_snapshot(&group.group_id,&feed.items,feed.cursor,&feed.acknowledged).unwrap();
        let feed=exchange("fictional-pc2-token",pc2.cursor(&group.group_id).unwrap(),vec![]);pc2.receive_snapshot(&group.group_id,&feed.items,feed.cursor,&feed.acknowledged).unwrap();
        for snapshot in [sync_profile::read().unwrap(),pc2.snapshot().unwrap().unwrap(),android.snapshot().unwrap().unwrap()]{let center:Value=serde_json::from_str(snapshot["center_data"].as_str().unwrap()).unwrap();assert_eq!(center["notes"].as_array().unwrap().len(),3);}
        let parent=pc2.versions("notes","pc1").unwrap()[0].op_id.clone();let edit=pc2.local_edit("notes","pc1",Some(json!({"id":"pc1","title":"edited","text":"conflict","updatedAt":"2026-10-12T12:00:00Z"})),vec![parent.clone()]).unwrap();let deletion=android.local_edit("notes","pc1",None,vec![parent]).unwrap();
        exchange("fictional-pc2-token",pc2.cursor(&group.group_id).unwrap(),vec![edit]);exchange("fictional-android-token",android.cursor(&group.group_id).unwrap(),vec![deletion]);assert_eq!(sync_profile::journal().unwrap().versions("notes","pc1").unwrap().len(),2);
        let offline=pc2.local_edit("notes","future",Some(json!({"id":"future","title":"future","text":"fictional future","updatedAt":"2026-10-12T12:00:00Z"})),vec![]).unwrap();
        group.epoch=2;group.keys.insert(2,[77;32]);group.revoked_devices.push("a".repeat(64));sync_profile::save_at(root,&group).unwrap();pc2.transport_keys(group.keys.clone(),2).unwrap();
        let denied=handle("/v2/keys","fictional-android-token",json!({}),&state.inner,&manager,root);assert_ne!(denied.0,200);assert!(denied.1.get("group").is_none());
        let (status,body)=handle("/v2/sync","fictional-pc2-token",json!({"version":2,"group_id":group.group_id,"cursor":pc2.cursor(&group.group_id).unwrap(),"operations":[offline.clone()]}),&state.inner,&manager,root);assert_eq!(status,200);let feed:Feed=serde_json::from_value(body).unwrap();assert_eq!(feed.rekey,vec![offline.op_id.clone()]);pc2.rewrap_outbox(&feed.rekey).unwrap();
        let future=pc2.outbox(100).unwrap().into_iter().find(|op|op.op_id==offline.op_id).unwrap();assert!(jarvis_core::secure_storage::open_with_key(&future.sealed,&future.aad(),&[42;32]).is_err());
        for _ in 0..10{let (status,_)=handle("/v2/sync","fictional-pc2-token",json!({"version":2,"group_id":group.group_id,"cursor":0,"operations":[future.clone()]}),&state.inner,&manager,root);assert_eq!(status,200);}
        assert_eq!(sync_profile::journal().unwrap().versions("notes","future").unwrap().len(),1);
        let mut broken=future;broken.entity_id="wrong-AAD".into();assert_ne!(handle("/v2/sync","fictional-pc2-token",json!({"version":2,"group_id":group.group_id,"cursor":0,"operations":[broken]}),&state.inner,&manager,root).0,200);
        assert_eq!(sync_profile::read().unwrap()["api_keys"]["openai"],"fictional-not-synchronized");
    }
    #[test] fn stale_center_saves_merge_independent_edits_but_reject_same_record(){
        let base=json!({"notes":[{"id":"a","title":"old"},{"id":"b","title":"old"}],"reminders":[],"birthdays":[],"habits":[],"tasks":[]});
        let mut local=base.clone();local["notes"][0]["title"]=json!("local");
        let mut remote=base.clone();remote["notes"][1]["title"]=json!("remote");
        let merged=merge_center_checked(&base,&local,&remote).unwrap();assert_eq!(merged["notes"][0]["title"],"local");assert_eq!(merged["notes"][1]["title"],"remote");
        remote["notes"][0]["title"]=json!("conflict");assert!(merge_center_checked(&base,&local,&remote).is_err());assert_eq!(local["notes"][0]["title"],"local");
    }
    #[test] fn authentication_rejects_unknown_revoked_and_expired_tokens(){
        let directory=tempfile::tempdir().unwrap();let state=LanSync::default();
        let settings=jarvis_core::SettingsManager::new(jarvis_core::Settings::default());
        assert_eq!(handle("/v2/sync","unknown",json!({}),&state.inner,&settings,directory.path()).0,503);
        {let mut i=state.inner.lock();i.runtime=Some(Runtime{stop:Arc::new(AtomicBool::new(false)),address:"192.168.1.2:47832".into(),fingerprint:random(),secret:"test-secret".into(),expires:Instant::now()+Duration::from_secs(120),pending:vec![]});}
        assert_eq!(handle("/v2/sync","unknown",json!({}),&state.inner,&settings,directory.path()).0,401);
        for route in ["/v2/commands","/v2/command","/v2/command/status"]{assert_eq!(handle(route,"unknown",json!({}),&state.inner,&settings,directory.path()).0,401);assert_eq!(handle(route,"test-secret",json!({}),&state.inner,&settings,directory.path()).0,401);}
        assert_eq!(handle("/pair","wrong",json!({"version":2,"name":"Phone"}),&state.inner,&settings,directory.path()).0,401);
        assert_eq!(handle("/pair","test-secret",json!({"version":2,"name":"Phone"}),&state.inner,&settings,directory.path()).0,202);
        let pending=status(&state.inner.lock())["pending"][0]["id"].as_str().unwrap().to_string();
        assert_eq!(handle("/pair/status","test-secret",json!({"id":pending}),&state.inner,&settings,directory.path()).0,202);
        assert_eq!(handle("/v2/sync","unapproved",json!({}),&state.inner,&settings,directory.path()).0,401);
        {let mut i=state.inner.lock();i.saved.devices.push(Device{id:"revoked".into(),name:"Phone".into(),token_hash:digest("approved"),last_sync:None,protocol:2});i.saved.devices.clear();i.runtime.as_mut().unwrap().expires=Instant::now()-Duration::from_secs(1);}
        assert_eq!(handle("/v2/sync","approved",json!({}),&state.inner,&settings,directory.path()).0,401);
        assert_eq!(handle("/pair","test-secret",json!({"version":2,"name":"Phone"}),&state.inner,&settings,directory.path()).0,401);
        assert!(!directory.path().join("password-vault-v1.json").exists());
    }
    #[test] fn three_way_preserves_edits_and_deletions(){let b=json!([{"id":"a","title":"old"}]);let l=b.clone();let r=json!([]);let mut c=vec![];assert_eq!(merge_records(&b,&l,&r,"center","notes",&mut c),json!([]));assert!(c.is_empty());
        let l=json!([{"id":"a","title":"pc"}]);let r=json!([{"id":"a","title":"phone"}]);assert_eq!(merge_records(&b,&l,&r,"center","notes",&mut c),l);assert_eq!(c.len(),1);assert_eq!(c[0].remote,r[0]);}
    #[test] fn only_private_addresses_and_sealed_vaults(){assert!(private("192.168.1.2".parse().unwrap()));assert!(!private("8.8.8.8".parse().unwrap()));assert!(!private("127.0.0.1".parse().unwrap()));assert!(valid_vault(&json!({"version":1,"cards":{"data":"plaintext"}})).is_err());assert!(valid_records(&json!([{"id":"a"},{"id":"a"}])).is_err());}
}

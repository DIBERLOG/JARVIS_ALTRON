//! Desktop client for the same pinned TLS endpoints used by Android.
use super::lan_sync::private;
use serde_json::{Value,json};
use std::{sync::Arc,time::Duration,io::Read};
use sha2::{Digest,Sha256};
use subtle::ConstantTimeEq;
use rustls::{client::danger::{ServerCertVerifier,ServerCertVerified,HandshakeSignatureValid},pki_types::{CertificateDer,ServerName,UnixTime},DigitallySignedStruct,SignatureScheme};
use jarvis_core::sync_profile;

#[derive(Debug)]struct Pin{hash:String,provider:Arc<rustls::crypto::CryptoProvider>}
impl ServerCertVerifier for Pin{
    fn verify_server_cert(&self,end:&CertificateDer<'_>,_:&[CertificateDer<'_>],_:&ServerName<'_>,_:&[u8],_:UnixTime)->Result<ServerCertVerified,rustls::Error>{
        let actual:String=Sha256::digest(end.as_ref()).iter().map(|b|format!("{b:02x}")).collect();
        if bool::from(actual.as_bytes().ct_eq(self.hash.as_bytes())){Ok(ServerCertVerified::assertion())}else{Err(rustls::Error::General("Сертификат хаба изменился; нужно новое сопряжение".into()))}
    }
    fn verify_tls12_signature(&self,message:&[u8],cert:&CertificateDer<'_>,dss:&DigitallySignedStruct)->Result<HandshakeSignatureValid,rustls::Error>{rustls::crypto::verify_tls12_signature(message,cert,dss,&self.provider.signature_verification_algorithms)}
    fn verify_tls13_signature(&self,message:&[u8],cert:&CertificateDer<'_>,dss:&DigitallySignedStruct)->Result<HandshakeSignatureValid,rustls::Error>{rustls::crypto::verify_tls13_signature(message,cert,dss,&self.provider.signature_verification_algorithms)}
    fn supported_verify_schemes(&self)->Vec<SignatureScheme>{self.provider.signature_verification_algorithms.supported_schemes()}
}
fn validate(peer:&Value)->Result<(String,u16,String),String>{
    let host=peer["host"].as_str().ok_or("Нет адреса хаба")?;let ip:std::net::Ipv4Addr=host.parse().map_err(|_|"Нужен локальный IPv4")?;
    let port=peer["port"].as_u64().filter(|v|(1024..=65535).contains(v)).ok_or("Некорректный порт")? as u16;
    let hash=peer["fingerprint"].as_str().filter(|v|v.len()==64&&v.bytes().all(|b|b.is_ascii_hexdigit())).ok_or("Некорректный отпечаток")?;
    if !private(ip){return Err("Хаб должен быть в частной Wi-Fi сети".into());}Ok((host.into(),port,hash.into()))
}
fn tls_client(hash:String)->Result<reqwest::blocking::Client,String>{
    let provider=Arc::new(rustls::crypto::ring::default_provider());
    let tls=rustls::ClientConfig::builder_with_provider(provider.clone()).with_safe_default_protocol_versions().map_err(|_|"Ошибка TLS")?.dangerous().with_custom_certificate_verifier(Arc::new(Pin{hash,provider})).with_no_client_auth();
    reqwest::blocking::Client::builder().use_preconfigured_tls(tls).redirect(reqwest::redirect::Policy::none()).no_proxy().connect_timeout(Duration::from_secs(10)).timeout(Duration::from_secs(30)).build().map_err(|_|"Не удалось создать TLS-клиент".into())
}

pub(super) fn request(peer:&Value,path:&str,token:&str,body:&Value)->Result<Value,String>{
    let(host,port,hash)=validate(peer)?;let client=tls_client(hash)?;
    let bytes=serde_json::to_vec(body).map_err(|_|"Некорректный пакет")?;if bytes.len()>16*1024*1024{return Err("Пакет слишком большой".into());}
    let mut response=client.post(format!("https://{host}:{port}{path}")).bearer_auth(token).header("Content-Type","application/json").body(bytes).send().map_err(|_|"Хаб недоступен или сертификат не совпадает; очередь сохранена")?;
    let status=response.status();let mut bytes=Vec::new();Read::by_ref(&mut response).take(16*1024*1024+1).read_to_end(&mut bytes).map_err(|_|"Ответ прерван; очередь сохранена")?;
    if bytes.len()>16*1024*1024{return Err("Ответ слишком большой".into());}let value:Value=serde_json::from_slice(&bytes).map_err(|_|"Некорректный ответ")?;
    if !status.is_success(){return Err(value["error"].as_str().unwrap_or("Обмен отклонён хабом").into());}Ok(value)
}
fn pending_path()->Result<std::path::PathBuf,String>{Ok(sync_profile::root()?.join("sync-v2-pairing.json"))}
fn save_pending(value:&Value)->Result<(),String>{let sealed=jarvis_core::secure_storage::seal(value,"sync-v2-pairing")?;altron_write(&pending_path()?,&sealed)}
fn altron_write(path:&std::path::Path,value:&Value)->Result<(),String>{
    use std::io::Write;let mut file=tempfile::NamedTempFile::new_in(path.parent().ok_or("Нет каталога")?).map_err(|_|"Не удалось подготовить запись")?;
    file.write_all(value.to_string().as_bytes()).and_then(|_|file.as_file().sync_all()).map_err(|_|"Не удалось сохранить сопряжение")?;file.persist(path).map_err(|_|"Не удалось сохранить сопряжение")?;Ok(())
}
fn read_pending()->Result<Value,String>{let bytes=std::fs::read(pending_path()?).map_err(|_|"Сначала вставьте QR хаба")?;let sealed=serde_json::from_slice(&bytes).map_err(|_|"Повреждено сопряжение")?;jarvis_core::secure_storage::open(&sealed,"sync-v2-pairing")}
#[tauri::command]
pub fn sync_v2_pair_begin(qr:String,name:String)->Result<Value,String>{
    let existing=if sync_profile::enabled(){let root=sync_profile::root()?;let group=sync_profile::load_at(&root)?;
        if group.peer.is_some()||!sync_profile::journal()?.can_repair()?{return Err("Этот профиль уже подключён или создан как хаб".into());}Some(group)
    }else{None};
    if qr.len()>1500||name.is_empty()||name.len()>100{return Err("Некорректное сопряжение".into());}
    let url=reqwest::Url::parse(&qr).map_err(|_|"Некорректный QR")?;if url.scheme()!="jarvis-pair"||url.host_str()!=Some("v2"){return Err("Нужен QR Sync v2".into());}
    let pairs:std::collections::BTreeMap<_,_>=url.query_pairs().into_owned().collect();let mut peer=json!({"host":pairs.get("host"),"port":pairs.get("port").and_then(|v|v.parse::<u16>().ok()),"fingerprint":pairs.get("fingerprint"),"secret":pairs.get("secret")});validate(&peer)?;
    let secret=peer["secret"].as_str().filter(|v|v.len()==64).ok_or("Некорректный секрет QR")?;
    let mut body=json!({"version":2,"name":name});if let Some(group)=existing{body["group_id"]=json!(group.group_id);body["device_id"]=json!(group.device_id);}
    let result=request(&peer,"/pair",secret,&body)?;
    peer["pendingId"]=result["id"].clone();save_pending(&peer)?;Ok(result)
}
#[tauri::command]
pub fn sync_v2_pair_finish(state:tauri::State<'_,crate::AppState>,password:String)->Result<Value,String>{
    let mut peer=read_pending()?;
    let result=if peer.get("approved").is_some(){peer["approved"].clone()}else{
        let result=request(&peer,"/pair/status",peer["secret"].as_str().ok_or("Нет секрета QR")?,&json!({"id":peer["pendingId"]}))?;
        if result["pending"]==true{return Ok(result);}peer["approved"]=result.clone();save_pending(&peer)?;result
    };
    let wire=&result["group"];let keys=serde_json::from_value(wire["keys"].clone()).map_err(|_|"Некорректные ключи")?;
    let mut endpoint=peer.clone();for field in ["secret","approved","pendingId"]{endpoint.as_object_mut().unwrap().remove(field);}endpoint["token"]=result["token"].clone();
    let mut group=sync_profile::Group{version:2,group_id:wire["group_id"].as_str().ok_or("Нет группы")?.into(),device_id:result["deviceId"].as_str().ok_or("Нет устройства")?.into(),epoch:wire["epoch"].as_u64().and_then(|v|u32::try_from(v).ok()).filter(|v|*v>0).ok_or("Нет версии ключа")?,keys,peer:Some(endpoint),revoked_devices:vec![]};
    if sync_profile::enabled(){let root=sync_profile::root()?;let existing=sync_profile::load_at(&root)?;
        if existing.group_id!=group.group_id||existing.device_id!=group.device_id{return Err("Восстановленный профиль принадлежит другой группе".into());}
        group.revoked_devices=existing.revoked_devices.clone();
        let backup=sync_profile::export(&password)?;sync_profile::recovery_open(&backup,&password)?;
        let path=root.join(format!("sync-v2-recovery-paired-{}.json",chrono::Utc::now().timestamp_millis()));altron_write(&path,&backup)?;
        sync_profile::save_at(&root,&group)?;return Ok(json!({"paired":true,"recovery":path.display().to_string()}));
    }
    let initial=jarvis_core::db::migration_snapshot(&state.settings.lock())?;
    let backup=sync_profile::prepare_group(initial,&password,group)?;Ok(json!({"paired":true,"recovery":backup.display().to_string()}))
}
#[tauri::command]
pub fn sync_v2_run()->Result<Value,String>{
    let _guard=CLIENT_IO.lock();
    let root=sync_profile::root()?;let mut group=sync_profile::load_at(&root)?;let peer=group.peer.clone().ok_or("Этот компьютер — хаб; клиенты подключаются к нему")?;let token=peer["token"].as_str().ok_or("Нет токена")?;
    let response=request(&peer,"/v2/keys",token,&json!({"version":2}))?;let tasks_supported=response["features"].as_array().is_some_and(|v|v.iter().any(|f|f=="tasks"));let wire=&response["group"];
    let relay_id=response["relay_id"].as_str().filter(|v|v.len()==64&&v.bytes().all(|b|b.is_ascii_hexdigit())).ok_or("Нет идентичности журнала хаба")?.to_string();
    if wire["group_id"]!=group.group_id{return Err("Хаб сменил группу; очередь не изменена".into());}
    let epoch=wire["epoch"].as_u64().and_then(|v|u32::try_from(v).ok()).filter(|v|*v>0).ok_or("Нет версии ключа")?;if epoch<group.epoch{return Err("Откат версии ключа".into());}
    let keys:std::collections::BTreeMap<u32,[u8;32]>=serde_json::from_value(wire["keys"].clone()).map_err(|_|"Некорректные ключи")?;
    for (e,k) in &group.keys{if keys.get(e)!=Some(k){return Err("Ключ прошлой версии изменился".into());}}group.keys=keys;group.epoch=epoch;group.peer.as_mut().unwrap()["relay_id"]=json!(relay_id);sync_profile::save_at(&root,&group)?;
    let mut journal=sync_profile::journal_at(&root,&group)?;
    for _ in 0..100{let recovery=journal.recovery_outbox()?;let operations=if recovery.is_empty(){journal.outbox(100)?}else{recovery.clone()};let cursor=journal.cursor(&relay_id)?;if operations.iter().any(|op|op.collection=="tasks")&&!tasks_supported{return Err("Для общих задач обновите хаб до Sync v2 LAB 1.1.2".into());}
        let response=request(&peer,if recovery.is_empty(){"/v2/sync"}else{"/v2/recover"},token,&json!({"version":2,"features":["tasks"],"group_id":group.group_id,"cursor":cursor,"operations":operations}))?;
        let feed:jarvis_core::sync_relay::Feed=serde_json::from_value(response).map_err(|_|"Некорректный пакет v2")?;
        if feed.version!=2||feed.group_id!=group.group_id||feed.relay_id!=relay_id||feed.epoch!=group.epoch{return Err("Версия группы изменилась; повторите обмен".into());}
        if !feed.rekey.is_empty(){journal.rewrap_outbox(&feed.rekey)?;continue;}
        journal.receive_snapshot(&relay_id,&feed.items,feed.cursor,&feed.acknowledged)?;
        if feed.items.is_empty()&&operations.is_empty(){return Ok(json!({"synced":true,"cursor":feed.cursor}));}
    }Err("Повторите обмен для оставшейся части большого журнала".into())
}
#[tauri::command]
pub fn sync_v2_sharing(chats:bool,vault:bool)->Result<(),String>{sync_profile::update(&json!({}),|snapshot|{snapshot["sync_share_chats"]=json!(chats);snapshot["sync_share_vault"]=json!(vault);Ok(())}).map(|_|())}

static CLIENT_IO:parking_lot::Mutex<()>=parking_lot::Mutex::new(());
static LAST_STATUS:parking_lot::Mutex<Option<(String,Option<String>)>>=parking_lot::Mutex::new(None);
pub fn start_background_sync(){std::thread::spawn(||loop{
    if sync_profile::enabled(){if let Ok(group)=sync_profile::root().and_then(|root|sync_profile::load_at(&root)){
        if group.peer.as_ref().is_some_and(|p|p["auto"]==true){let error=sync_v2_run().err();*LAST_STATUS.lock()=Some((chrono::Utc::now().to_rfc3339(),error));}
    }}std::thread::sleep(Duration::from_secs(30));
});}
pub(super) fn background_status()->Value{let status=LAST_STATUS.lock();json!({"lastAttempt":status.as_ref().map(|s|&s.0),"error":status.as_ref().and_then(|s|s.1.as_ref())})}
#[tauri::command]
pub fn sync_v2_auto(enabled:bool)->Result<(),String>{let root=sync_profile::root()?;let mut group=sync_profile::load_at(&root)?;group.peer.as_mut().ok_or("Автообмен включается на компьютере-клиенте")?["auto"]=json!(enabled);sync_profile::save_at(&root,&group)}

#[cfg(test)]mod tests{
    use super::*;use std::net::TcpListener;use std::io::Write;
    #[test]fn pinned_tls_checks_certificate_before_sending_credentials(){
        for correct in [true,false]{
            let rcgen::CertifiedKey{cert,key_pair}=rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
            let fingerprint:String=Sha256::digest(cert.der()).iter().map(|b|format!("{b:02x}")).collect();
            let config=rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider())).with_safe_default_protocol_versions().unwrap().with_no_client_auth().with_single_cert(vec![cert.der().clone()],rustls::pki_types::PrivatePkcs8KeyDer::from(key_pair.serialize_der()).into()).unwrap();
            let listener=TcpListener::bind("127.0.0.1:0").unwrap();let address=listener.local_addr().unwrap();
            let server=std::thread::spawn(move||{let(socket,_)=listener.accept().unwrap();socket.set_read_timeout(Some(Duration::from_secs(4))).unwrap();socket.set_write_timeout(Some(Duration::from_secs(4))).unwrap();let session=rustls::ServerConnection::new(Arc::new(config)).unwrap();let mut stream=rustls::StreamOwned::new(session,socket);let mut bytes=vec![];let mut byte=[0u8];while !bytes.ends_with(b"\r\n\r\n"){if stream.read(&mut byte).is_err(){return false;}bytes.push(byte[0]);if bytes.len()>4096{return false;}}
                assert!(String::from_utf8(bytes).unwrap().contains("fictional-tls-only"));let body=b"{\"version\":2}";write!(stream,"HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len()).unwrap();stream.write_all(body).unwrap();stream.flush().unwrap();true});
            let client=tls_client(if correct{fingerprint}else{"0".repeat(64)}).unwrap();let result=client.post(format!("https://{address}/v2/keys")).bearer_auth("fictional-tls-only").body("{}").send();assert_eq!(result.is_ok(),correct);assert_eq!(server.join().unwrap(),correct);
        }
    }
}

#[tauri::command]
pub fn sync_v2_disconnect()->Result<(),String>{
    let _guard=CLIENT_IO.lock();let root=sync_profile::root()?;let mut group=sync_profile::load_at(&root)?;
    if group.peer.is_none(){return Err("Этот профиль не является клиентом".into());}group.peer=None;sync_profile::save_at(&root,&group)?;sync_profile::journal()?.allow_repair()?;Ok(())
}

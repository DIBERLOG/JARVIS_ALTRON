//! Durable blind relay. No personal plaintext or group keys are stored here.
use crate::sync_journal::Operation;
use rusqlite::{params,Connection,OptionalExtension,TransactionBehavior};
use serde::{Deserialize,Serialize};
use std::{path::Path,time::Duration};

#[derive(Serialize,Deserialize,Debug)]
pub struct Feed{pub version:u32,pub group_id:String,#[serde(default)] pub relay_id:String,pub epoch:u32,pub cursor:u64,pub items:Vec<(u64,Operation)>,pub acknowledged:Vec<String>,#[serde(default)] pub rekey:Vec<String>}
pub struct Relay{db:Connection}
fn error(_:impl std::fmt::Display)->String{"Журнал посредника не изменён: ошибка транзакции".into()}
impl Relay{
    pub fn open(path:&Path,group:&str,epoch:u32)->Result<Self,String>{
        if group.len()!=64||epoch==0{return Err("Некорректная группа".into());}
        let db=Connection::open(path).map_err(error)?;db.busy_timeout(Duration::from_secs(10)).map_err(error)?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;
            CREATE TABLE IF NOT EXISTS group_state(id INTEGER PRIMARY KEY CHECK(id=1),group_id TEXT NOT NULL,epoch INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS relay_identity(id INTEGER PRIMARY KEY CHECK(id=1),value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS devices(id TEXT PRIMARY KEY,active INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS operations(seq INTEGER PRIMARY KEY AUTOINCREMENT,op_id TEXT UNIQUE NOT NULL,collection TEXT NOT NULL,entity_id TEXT NOT NULL,payload TEXT NOT NULL);").map_err(error)?;
        db.execute("INSERT OR IGNORE INTO relay_identity VALUES(1,?1)",[crate::sync_profile::random_id()]).map_err(error)?;
        db.execute("INSERT OR IGNORE INTO group_state VALUES(1,?1,?2)",params![group,epoch]).map_err(error)?;
        let actual:(String,u32)=db.query_row("SELECT group_id,epoch FROM group_state WHERE id=1",[],|r|Ok((r.get(0)?,r.get(1)?))).map_err(error)?;
        if actual.0!=group||actual.1>epoch{return Err("Группа или версия ключа посредника не совпадает".into());}
        // The OS-protected group file is authoritative. Finishes a crash between key rotation and relay update.
        if actual.1<epoch{db.execute("UPDATE group_state SET epoch=?1 WHERE id=1",[epoch]).map_err(error)?;}Ok(Self{db})
    }
    pub fn identity(&self)->Result<String,String>{self.db.query_row("SELECT value FROM relay_identity WHERE id=1",[],|r|r.get(0)).map_err(error)}
    pub fn existing(&self,id:&str)->Result<Option<Operation>,String>{let raw:Option<String>=self.db.query_row("SELECT payload FROM operations WHERE op_id=?1",[id],|r|r.get(0)).optional().map_err(error)?;raw.map(|raw|serde_json::from_str(&raw).map_err(error)).transpose()}
    pub fn trust(&self,device:&str)->Result<(),String>{
        if device.len()!=64||!device.bytes().all(|b|b.is_ascii_hexdigit()){return Err("Некорректное устройство".into());}
        self.db.execute("INSERT INTO devices VALUES(?1,1) ON CONFLICT(id) DO UPDATE SET active=1",[device]).map_err(error)?;Ok(())
    }
    pub fn revoke(&mut self,device:&str,new_epoch:u32)->Result<(),String>{
        let tx=self.db.transaction_with_behavior(TransactionBehavior::Immediate).map_err(error)?;
        let epoch:u32=tx.query_row("SELECT epoch FROM group_state WHERE id=1",[],|r|r.get(0)).map_err(error)?;
        if new_epoch!=epoch.checked_add(1).ok_or("Исчерпаны версии ключей")?{return Err("Некорректная ротация".into());}
        tx.execute("UPDATE devices SET active=0 WHERE id=?1",[device]).map_err(error)?;
        tx.execute("UPDATE group_state SET epoch=?1 WHERE id=1",[new_epoch]).map_err(error)?;tx.commit().map_err(error)
    }
    pub fn exchange(&mut self,device:&str,group:&str,cursor:u64,operations:&[Operation])->Result<Feed,String>{self.exchange_mode(device,group,cursor,operations,false)}
    /// Explicit recovery import, permitted only after fresh pairing and AEAD validation by the hub client.
    pub fn recover(&mut self,device:&str,group:&str,cursor:u64,operations:&[Operation])->Result<Feed,String>{self.exchange_mode(device,group,cursor,operations,true)}
    fn exchange_mode(&mut self,device:&str,group:&str,cursor:u64,operations:&[Operation],recovery:bool)->Result<Feed,String>{
        if operations.len()>100||cursor>i64::MAX as u64||serde_json::to_vec(operations).map_err(error)?.len()>8*1024*1024{return Err("Пакет слишком большой".into());}
        let relay_id=self.identity()?;
        let tx=self.db.transaction_with_behavior(TransactionBehavior::Immediate).map_err(error)?;
        let active:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM devices WHERE id=?1 AND active=1)",[device],|r|r.get(0)).map_err(error)?;
        if !active{return Err("Устройство отозвано или не сопряжено".into());}
        let (group_id,epoch):(String,u32)=tx.query_row("SELECT group_id,epoch FROM group_state WHERE id=1",[],|r|Ok((r.get(0)?,r.get(1)?))).map_err(error)?;
        if group!=group_id{return Err("Другая группа синхронизации".into());}
        let maximum:i64=tx.query_row("SELECT COALESCE(MAX(seq),0) FROM operations",[],|r|r.get(0)).map_err(error)?;
        if cursor>maximum as u64{return Err("Курсор опережает посредника; требуется восстановление".into());}
        let mut rekey=Vec::new();
        for op in operations{op.validate()?;if !recovery&&op.device_id!=device{return Err("ID автора не соответствует сопряжению".into());}
            let exists:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM operations WHERE op_id=?1)",[&op.op_id],|r|r.get(0)).map_err(error)?;
            if !exists&&op.key_version<epoch{rekey.push(op.op_id.clone());}}
        if !rekey.is_empty(){return Ok(Feed{version:2,group_id,relay_id,epoch,cursor,items:vec![],acknowledged:vec![],rekey});}
        let mut acknowledged=Vec::new();
        for op in operations{
            op.validate()?;if !recovery&&op.device_id!=device{return Err("ID автора не соответствует сопряжению".into());}
            // Structural validation of the AEAD envelope, without decrypting it.
            use base64::{Engine,engine::general_purpose::STANDARD as B64};
            if op.sealed["format"]!="jarvis-protected-json"||op.sealed["version"]!=1||op.sealed.as_object().is_none_or(|o|o.len()!=4)||B64.decode(op.sealed["nonce"].as_str().unwrap_or("")).map(|v|v.len()).unwrap_or(0)!=12||B64.decode(op.sealed["data"].as_str().unwrap_or("")).map(|v|v.len()).unwrap_or(0)<16{return Err("Повреждён зашифрованный пакет".into());}
            let encoded=serde_json::to_string(op).map_err(error)?;
            let previous:Option<String>=tx.query_row("SELECT payload FROM operations WHERE op_id=?1",[&op.op_id],|r|r.get(0)).optional().map_err(error)?;
            if let Some(previous)=previous{if previous!=encoded{return Err("Повторный ID с другим содержимым".into());}}
            else{
                // Still-trusted offline devices may submit old-key operations. Revoked devices cannot.
                if op.key_version!=epoch{return Err("Неизвестная версия ключа".into());}
                for parent in &op.parents{let exists:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM operations WHERE op_id=?1 AND collection=?2 AND entity_id=?3)",params![parent,op.collection,op.entity_id],|r|r.get(0)).map_err(error)?;if !exists{return Err("Сначала отправьте родительские операции".into());}}
                tx.execute("INSERT INTO operations(op_id,collection,entity_id,payload) VALUES(?1,?2,?3,?4)",params![op.op_id,op.collection,op.entity_id,encoded]).map_err(error)?;
            }acknowledged.push(op.op_id.clone());
        }
        let mut query=tx.prepare("SELECT seq,payload FROM operations WHERE seq>?1 ORDER BY seq LIMIT 100").map_err(error)?;
        let rows=query.query_map([cursor as i64],|r|Ok((r.get::<_,i64>(0)? as u64,r.get::<_,String>(1)?))).map_err(error)?;
        let mut items=Vec::new();let mut size=0;for row in rows{let(seq,raw)=row.map_err(error)?;if size+raw.len()>8*1024*1024{break;}size+=raw.len();items.push((seq,serde_json::from_str(&raw).map_err(error)?));}
        drop(query);let cursor=items.last().map(|v|v.0).unwrap_or(cursor);
        tx.commit().map_err(error)?;Ok(Feed{version:2,group_id,relay_id,epoch,cursor,items,acknowledged,rekey:vec![]})
    }
}

#[cfg(test)]mod tests{
    use super::*;use crate::sync_journal::Journal;use serde_json::json;
    #[test]fn relay_retries_restart_rotation_and_three_clients(){
        let root=tempfile::tempdir().unwrap();let group="a".repeat(64);let path=root.path().join("relay.db");let mut relay=Relay::open(&path,&group,1).unwrap();
        let identity=relay.identity().unwrap();assert_eq!(identity.len(),64);let another=Relay::open(&root.path().join("replacement.db"),&group,1).unwrap();assert_ne!(another.identity().unwrap(),identity);
        let key=[9;32];let mut clients:Vec<_>=(0..3).map(|i|{let device=format!("{i:064x}");relay.trust(&device).unwrap();Journal::open(&root.path().join(format!("{i}.db")),key,device).unwrap()}).collect();
        let mut ops=vec![];for (i,c) in clients.iter_mut().enumerate(){ops.push(c.local_edit("notes",&format!("note{i}"),Some(json!({"id":format!("note{i}"),"text":"fictional-relay-secret"})),vec![]).unwrap());}
        for (i,op) in ops.iter().enumerate(){for _ in 0..10{relay.exchange(&format!("{i:064x}"),&group,0,&[op.clone()]).unwrap();}}
        drop(relay);let mut relay=Relay::open(&path,&group,1).unwrap();
        assert_eq!(relay.identity().unwrap(),identity);
        for (i,c) in clients.iter_mut().enumerate(){let feed=relay.exchange(&format!("{i:064x}"),&group,0,&[]).unwrap();assert_eq!(feed.items.len(),3);c.receive("hub",&feed.items,feed.cursor).unwrap();assert_eq!(c.versions("notes","note2").unwrap().len(),1);}
        let mut forged=ops[0].clone();forged.device_id=format!("{:064x}",1);assert!(relay.exchange(&format!("{:064x}",0),&group,0,&[forged]).is_err());
        relay.revoke(&format!("{:064x}",1),2).unwrap();assert!(relay.exchange(&format!("{:064x}",1),&group,0,&[]).is_err());
        clients[0].transport_keys(std::collections::BTreeMap::from([(1,key),(2,[7;32])]),2).unwrap();let future=clients[0].local_edit("notes","future",Some(json!({"id":"future"})),vec![]).unwrap();
        assert!(crate::secure_storage::open_with_key(&future.sealed,&future.aad(),&key).is_err());relay.exchange(&format!("{:064x}",0),&group,3,&[future]).unwrap();
        drop(relay);let bytes=std::fs::read(path).unwrap();assert!(!String::from_utf8_lossy(&bytes).contains("fictional-relay-secret"));
    }
}

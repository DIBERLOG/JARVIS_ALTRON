//! Transactional encrypted records + outbox. Transport and OS key provisioning are separate.
use rusqlite::{Connection,OptionalExtension,params,TransactionBehavior};
use serde::{Serialize,Deserialize};
use serde_json::{Value,json};
use rand::RngCore;
use std::{path::Path,time::Duration};
use crate::secure_storage::{seal_with_key,open_with_key};

fn id()->String{let mut bytes=[0u8;32];rand::rngs::OsRng.fill_bytes(&mut bytes);bytes.iter().map(|b|format!("{b:02x}")).collect()}
fn error(_:impl std::fmt::Display)->String{"Ошибка журнала синхронизации; транзакция не применена".into()}
#[derive(Clone,Debug,Serialize,Deserialize,PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Operation{pub version:u32,pub op_id:String,pub device_id:String,pub collection:String,pub entity_id:String,pub parents:Vec<String>,pub deleted:bool,pub key_version:u32,pub sealed:Value}
impl Operation{
    fn aad(&self)->String{json!([self.version,self.op_id,self.device_id,self.collection,self.entity_id,self.parents,self.deleted,self.key_version]).to_string()}
    pub fn validate(&self)->Result<(),String>{
        let valid_id=|s:&str|s.len()==64&&s.bytes().all(|b|b.is_ascii_hexdigit());
        if self.version!=2||self.key_version!=1||!valid_id(&self.op_id)||!valid_id(&self.device_id)||self.entity_id.is_empty()||self.entity_id.len()>128||self.entity_id.chars().any(char::is_control)||self.parents.len()>32||self.parents.iter().any(|s|!valid_id(s))||self.parents.iter().collect::<std::collections::BTreeSet<_>>().len()!=self.parents.len(){return Err("Некорректная операция Sync v2".into());}
        if !matches!(self.collection.as_str(),"notes"|"reminders"|"birthdays"|"habits"|"training.exercises"|"training.plans"|"training.sessions"|"training.profile"|"chats"|"vault"){return Err("Коллекция не разрешена для синхронизации".into());}
        if self.sealed.to_string().len()>16*1024*1024{return Err("Операция синхронизации слишком большая".into());}Ok(())
    }
}
#[derive(Clone,Debug,Serialize,Deserialize,PartialEq)]
pub struct Version{pub op_id:String,pub deleted:bool,pub value:Value}
pub struct Journal{connection:Connection,key:[u8;32],device:String}
impl Journal{
    pub fn open(path:&Path,key:[u8;32],device:String)->Result<Self,String>{
        if device.len()!=64||!device.bytes().all(|b|b.is_ascii_hexdigit()){return Err("Некорректный ID устройства".into());}
        // Ciphertext only is ever bound to payload/record columns, including WAL pages.
        let connection=Connection::open(path).map_err(error)?;connection.busy_timeout(Duration::from_secs(10)).map_err(error)?;
        connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;
          CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT NOT NULL);
          CREATE TABLE IF NOT EXISTS operations(seq INTEGER PRIMARY KEY AUTOINCREMENT,op_id TEXT NOT NULL UNIQUE,collection TEXT NOT NULL,entity_id TEXT NOT NULL,payload TEXT NOT NULL,outbound INTEGER NOT NULL);
          CREATE TABLE IF NOT EXISTS records(collection TEXT NOT NULL,entity_id TEXT NOT NULL,sealed TEXT NOT NULL,PRIMARY KEY(collection,entity_id));
          CREATE TABLE IF NOT EXISTS cursors(peer TEXT PRIMARY KEY,value INTEGER NOT NULL);").map_err(error)?;
        let mut journal=Self{connection,key,device};
        let transaction=journal.connection.transaction_with_behavior(TransactionBehavior::Immediate).map_err(error)?;
        let check:Option<String>=transaction.query_row("SELECT value FROM metadata WHERE key='key_check'",[],|r|r.get(0)).optional().map_err(error)?;
        match check{Some(raw)=>{let sealed:Value=serde_json::from_str(&raw).map_err(error)?;if open_with_key(&sealed,"journal-key-check",&journal.key)?!=json!({"version":2}){return Err("Неверная версия журнала".into());}},None=>{let existing:i64=transaction.query_row("SELECT (SELECT COUNT(*) FROM operations)+(SELECT COUNT(*) FROM records)",[],|r|r.get(0)).map_err(error)?;if existing!=0{return Err("Журнал потерял проверку ключа; данные не перезаписаны".into());}let sealed=seal_with_key(&json!({"version":2}),"journal-key-check",&journal.key)?;transaction.execute("INSERT INTO metadata(key,value) VALUES ('key_check',?1)",[sealed.to_string()]).map_err(error)?;}}
        let owner:Option<String>=transaction.query_row("SELECT value FROM metadata WHERE key='device_id'",[],|r|r.get(0)).optional().map_err(error)?;
        if let Some(owner)=owner{if owner!=journal.device{return Err("Журнал принадлежит другому устройству; используйте восстановление данных, а не копирование идентификатора".into());}}else{transaction.execute("INSERT INTO metadata(key,value) VALUES ('device_id',?1)",[&journal.device]).map_err(error)?;}
        transaction.commit().map_err(error)?;Ok(journal)
    }
    pub fn local_edit(&mut self,collection:&str,entity:&str,value:Option<Value>,parents:Vec<String>)->Result<Operation,String>{
        let mut operation=Operation{version:2,op_id:id(),device_id:self.device.clone(),collection:collection.into(),entity_id:entity.into(),parents,deleted:value.is_none(),key_version:1,sealed:Value::Null};operation.validate()?;
        operation.sealed=seal_with_key(&value.unwrap_or(Value::Null),&operation.aad(),&self.key)?;
        let transaction=self.connection.transaction_with_behavior(TransactionBehavior::Immediate).map_err(error)?;
        Self::apply(&transaction,&self.key,&operation,true)?;transaction.commit().map_err(error)?;Ok(operation)
    }
    fn versions_at(connection:&Connection,key:&[u8;32],collection:&str,entity:&str)->Result<Vec<Version>,String>{
        let raw:Option<String>=connection.query_row("SELECT sealed FROM records WHERE collection=?1 AND entity_id=?2",params![collection,entity],|r|r.get(0)).optional().map_err(error)?;
        match raw{None=>Ok(vec![]),Some(raw)=>{let sealed:Value=serde_json::from_str(&raw).map_err(error)?;serde_json::from_value(open_with_key(&sealed,&format!("record:{collection}:{entity}"),key)?).map_err(error)}}
    }
    pub fn versions(&self,collection:&str,entity:&str)->Result<Vec<Version>,String>{Self::versions_at(&self.connection,&self.key,collection,entity)}
    fn apply(connection:&Connection,key:&[u8;32],operation:&Operation,outbound:bool)->Result<bool,String>{
        operation.validate()?;let value=open_with_key(&operation.sealed,&operation.aad(),key)?;
        if operation.deleted&&!value.is_null(){return Err("Операция удаления содержит данные".into());}
        let encoded=serde_json::to_string(operation).map_err(error)?;
        let previous:Option<String>=connection.query_row("SELECT payload FROM operations WHERE op_id=?1",[&operation.op_id],|r|r.get(0)).optional().map_err(error)?;
        if let Some(previous)=previous{if previous!=encoded{return Err("Повторный ID операции с другим содержимым".into());}return Ok(false);}
        for parent in &operation.parents{let known:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM operations WHERE op_id=?1 AND collection=?2 AND entity_id=?3)",params![parent,operation.collection,operation.entity_id],|r|r.get(0)).map_err(error)?;if !known{return Err("Отсутствует базовая операция; сначала загрузите предыдущие изменения".into());}}
        let mut versions=Self::versions_at(connection,key,&operation.collection,&operation.entity_id)?;
        versions.retain(|v|!operation.parents.contains(&v.op_id));versions.push(Version{op_id:operation.op_id.clone(),deleted:operation.deleted,value});versions.sort_by(|a,b|a.op_id.cmp(&b.op_id));
        let sealed=seal_with_key(&serde_json::to_value(versions).map_err(error)?,&format!("record:{}:{}",operation.collection,operation.entity_id),key)?;
        connection.execute("INSERT INTO operations(op_id,collection,entity_id,payload,outbound) VALUES (?1,?2,?3,?4,?5)",params![operation.op_id,operation.collection,operation.entity_id,encoded,outbound]).map_err(error)?;
        connection.execute("INSERT INTO records(collection,entity_id,sealed) VALUES (?1,?2,?3) ON CONFLICT(collection,entity_id) DO UPDATE SET sealed=excluded.sealed",params![operation.collection,operation.entity_id,sealed.to_string()]).map_err(error)?;Ok(true)
    }
    pub fn outbox(&self,limit:usize)->Result<Vec<Operation>,String>{let mut query=self.connection.prepare("SELECT payload FROM operations WHERE outbound=1 ORDER BY seq LIMIT ?1").map_err(error)?;let rows=query.query_map([limit.min(1000) as i64],|row|row.get::<_,String>(0)).map_err(error)?;rows.map(|row|serde_json::from_str(&row.map_err(error)?).map_err(error)).collect()}
    pub fn acknowledge(&mut self,ids:&[String])->Result<(),String>{if ids.len()>1000{return Err("Слишком много подтверждений".into());}let transaction=self.connection.transaction().map_err(error)?;for id in ids{transaction.execute("UPDATE operations SET outbound=0 WHERE op_id=?1",[id]).map_err(error)?;}transaction.commit().map_err(error)}
    pub fn cursor(&self,peer:&str)->Result<u64,String>{let value:Option<i64>=self.connection.query_row("SELECT value FROM cursors WHERE peer=?1",[peer],|r|r.get(0)).optional().map_err(error)?;Ok(value.unwrap_or(0) as u64)}
    pub fn receive(&mut self,peer:&str,items:&[(u64,Operation)],cursor:u64)->Result<(),String>{
        if peer.is_empty()||peer.len()>128||items.len()>1000||cursor>i64::MAX as u64{return Err("Некорректный пакет журнала".into());}
        let transaction=self.connection.transaction_with_behavior(TransactionBehavior::Immediate).map_err(error)?;
        let previous:Option<i64>=transaction.query_row("SELECT value FROM cursors WHERE peer=?1",[peer],|r|r.get(0)).optional().map_err(error)?;let previous=previous.unwrap_or(0) as u64;
        if cursor<previous{return Err("Курсор синхронизации уменьшился".into());}let mut last=0;
        for (sequence,operation) in items{if *sequence<=last||*sequence>cursor{return Err("Неверный порядок операций".into());}let inserted=Self::apply(&transaction,&self.key,operation,false)?;if *sequence<=previous&&inserted{return Err("Старый курсор содержит неизвестную операцию".into());}last=*sequence;}
        transaction.execute("INSERT INTO cursors(peer,value) VALUES (?1,?2) ON CONFLICT(peer) DO UPDATE SET value=excluded.value",params![peer,cursor as i64]).map_err(error)?;transaction.commit().map_err(error)
    }
    // ponytail: history/tombstones retained; prune only after all active devices acknowledge.
}
#[cfg(test)]mod tests{use super::*;
    #[test]fn three_offline_clients_retry_conflict_delete_and_restart(){
        let directory=tempfile::tempdir().unwrap();let key=[9u8;32];let mut clients:Vec<_>=(0..3).map(|i|Journal::open(&directory.path().join(format!("{i}.sqlite")),key,format!("{i:064x}")).unwrap()).collect();
        let a=clients[0].local_edit("notes","a",Some(json!({"text":"fictional-only-secret-a"})),vec![]).unwrap();let b=clients[1].local_edit("notes","b",Some(json!({"text":"fictional-only-secret-b"})),vec![]).unwrap();let c=clients[2].local_edit("notes","c",Some(json!({"text":"fictional-only-secret-c"})),vec![]).unwrap();let feed=vec![(1,a.clone()),(2,b.clone()),(3,c.clone())];
        for client in &mut clients{client.receive("hub",&feed,3).unwrap();assert_eq!(client.versions("notes","a").unwrap().len(),1);client.acknowledge(&[a.op_id.clone(),b.op_id.clone(),c.op_id.clone()]).unwrap();assert!(client.outbox(100).unwrap().is_empty());}
        for _ in 0..10{clients[0].receive("hub",&feed,3).unwrap();assert_eq!(clients[0].versions("notes","a").unwrap().len(),1);}
        let edit=clients[0].local_edit("notes","a",Some(json!({"text":"edited offline"})),vec![a.op_id.clone()]).unwrap();let deletion=clients[1].local_edit("notes","a",None,vec![a.op_id.clone()]).unwrap();let incoming=vec![(4,edit.clone()),(5,deletion.clone())];for client in &mut clients{client.receive("hub",&incoming,5).unwrap();let versions=client.versions("notes","a").unwrap();assert_eq!(versions.len(),2);assert!(versions.iter().any(|v|v.deleted));}
        drop(clients);let mut restored=Journal::open(&directory.path().join("0.sqlite"),key,format!("{:064x}",0)).unwrap();assert_eq!(restored.cursor("hub").unwrap(),5);assert_eq!(restored.outbox(100).unwrap().len(),1);assert_eq!(restored.versions("notes","a").unwrap().len(),2);
        let before=restored.cursor("hub").unwrap();let mut invalid=edit.clone();invalid.entity_id="tampered".into();assert!(restored.receive("hub",&[(6,invalid)],6).is_err());assert_eq!(restored.cursor("hub").unwrap(),before);
        let resolved=restored.local_edit("notes","a",None,vec![edit.op_id,deletion.op_id]).unwrap();assert!(resolved.deleted);assert_eq!(restored.versions("notes","a").unwrap().len(),1);
        drop(restored);let raw=std::fs::read(directory.path().join("0.sqlite")).unwrap();assert!(!raw.windows(b"fictional-only-secret".len()).any(|w|w==b"fictional-only-secret"));assert!(Journal::open(&directory.path().join("0.sqlite"),[8u8;32],format!("{:064x}",0)).is_err());
    }
}

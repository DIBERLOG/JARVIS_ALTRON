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
    pub fn aad(&self)->String{json!([self.version,self.op_id,self.device_id,self.collection,self.entity_id,self.parents,self.deleted,self.key_version]).to_string()}
    pub fn validate(&self)->Result<(),String>{
        let valid_id=|s:&str|s.len()==64&&s.bytes().all(|b|b.is_ascii_hexdigit());
        if self.version!=2||self.key_version==0||!valid_id(&self.op_id)||!valid_id(&self.device_id)||self.entity_id.is_empty()||self.entity_id.len()>128||self.entity_id.chars().any(char::is_control)||self.parents.len()>32||self.parents.iter().any(|s|!valid_id(s))||self.parents.iter().collect::<std::collections::BTreeSet<_>>().len()!=self.parents.len(){return Err("Некорректная операция Sync v2".into());}
        if !matches!(self.collection.as_str(),"notes"|"reminders"|"birthdays"|"habits"|"tasks"|"training.exercises"|"training.plans"|"training.sessions"|"training.profile"|"training.morningExercise"|"training.chargeSettings"|"training.chargeLevels"|"chats"|"vault"){return Err("Коллекция не разрешена для синхронизации".into());}
        if self.sealed.to_string().len()>6*1024*1024{return Err("Операция синхронизации слишком большая".into());}Ok(())
    }
}
#[derive(Clone,Debug,Serialize,Deserialize,PartialEq)]
pub struct Version{pub op_id:String,pub deleted:bool,pub value:Value}
pub struct Journal{connection:Connection,key:[u8;32],device:String,keys:std::collections::BTreeMap<u32,[u8;32]>,epoch:u32}
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
        let mut journal=Self{connection,key,device,keys:std::collections::BTreeMap::from([(1,key)]),epoch:1};
        let transaction=journal.connection.transaction_with_behavior(TransactionBehavior::Immediate).map_err(error)?;
        let check:Option<String>=transaction.query_row("SELECT value FROM metadata WHERE key='key_check'",[],|r|r.get(0)).optional().map_err(error)?;
        match check{Some(raw)=>{let sealed:Value=serde_json::from_str(&raw).map_err(error)?;if open_with_key(&sealed,"journal-key-check",&journal.key)?!=json!({"version":2}){return Err("Неверная версия журнала".into());}},None=>{let existing:i64=transaction.query_row("SELECT (SELECT COUNT(*) FROM operations)+(SELECT COUNT(*) FROM records)",[],|r|r.get(0)).map_err(error)?;if existing!=0{return Err("Журнал потерял проверку ключа; данные не перезаписаны".into());}let sealed=seal_with_key(&json!({"version":2}),"journal-key-check",&journal.key)?;transaction.execute("INSERT INTO metadata(key,value) VALUES ('key_check',?1)",[sealed.to_string()]).map_err(error)?;}}
        let owner:Option<String>=transaction.query_row("SELECT value FROM metadata WHERE key='device_id'",[],|r|r.get(0)).optional().map_err(error)?;
        if let Some(owner)=owner{if owner!=journal.device{return Err("Журнал принадлежит другому устройству; используйте восстановление данных, а не копирование идентификатора".into());}}else{transaction.execute("INSERT INTO metadata(key,value) VALUES ('device_id',?1)",[&journal.device]).map_err(error)?;}
        transaction.commit().map_err(error)?;Ok(journal)
    }
    pub fn local_edit(&mut self,collection:&str,entity:&str,value:Option<Value>,parents:Vec<String>)->Result<Operation,String>{
        let mut operation=Operation{version:2,op_id:id(),device_id:self.device.clone(),collection:collection.into(),entity_id:entity.into(),parents,deleted:value.is_none(),key_version:self.epoch,sealed:Value::Null};operation.validate()?;
        operation.sealed=seal_with_key(&value.unwrap_or(Value::Null),&operation.aad(),self.keys.get(&self.epoch).ok_or("Отсутствует ключ группы")?)?;
        let transaction=self.connection.transaction_with_behavior(TransactionBehavior::Immediate).map_err(error)?;
        Self::apply(&transaction,&self.key,&self.keys,&operation,true)?;transaction.commit().map_err(error)?;Ok(operation)
    }
    fn versions_at(connection:&Connection,key:&[u8;32],collection:&str,entity:&str)->Result<Vec<Version>,String>{
        let raw:Option<String>=connection.query_row("SELECT sealed FROM records WHERE collection=?1 AND entity_id=?2",params![collection,entity],|r|r.get(0)).optional().map_err(error)?;
        match raw{None=>Ok(vec![]),Some(raw)=>{let sealed:Value=serde_json::from_str(&raw).map_err(error)?;serde_json::from_value(open_with_key(&sealed,&format!("record:{collection}:{entity}"),key)?).map_err(error)}}
    }
    pub fn versions(&self,collection:&str,entity:&str)->Result<Vec<Version>,String>{Self::versions_at(&self.connection,&self.key,collection,entity)}
    fn apply(connection:&Connection,key:&[u8;32],keys:&std::collections::BTreeMap<u32,[u8;32]>,operation:&Operation,outbound:bool)->Result<bool,String>{
        operation.validate()?;let value=open_with_key(&operation.sealed,&operation.aad(),keys.get(&operation.key_version).ok_or("Отсутствует ключ версии группы")?)?;
        if operation.deleted&&!value.is_null(){return Err("Операция удаления содержит данные".into());}
        let encoded=serde_json::to_string(operation).map_err(error)?;
        let previous:Option<String>=connection.query_row("SELECT payload FROM operations WHERE op_id=?1",[&operation.op_id],|r|r.get(0)).optional().map_err(error)?;
        if let Some(previous)=previous{
            if previous!=encoded{
                let old:Operation=serde_json::from_str(&previous).map_err(error)?;let old_value=open_with_key(&old.sealed,&old.aad(),keys.get(&old.key_version).ok_or("Нет старого ключа")?)?;
                let mut old_metadata=old.clone();old_metadata.key_version=operation.key_version;old_metadata.sealed=operation.sealed.clone();
                if old.key_version==operation.key_version||old_metadata!=*operation||!crate::sync_data::equivalent(&old_value,&value){return Err("Повторный ID операции с другим содержимым".into());}
                if operation.key_version>old.key_version{connection.execute("UPDATE operations SET payload=?1 WHERE op_id=?2",params![encoded,operation.op_id]).map_err(error)?;}
            }return Ok(false);
        }
        for parent in &operation.parents{let known:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM operations WHERE op_id=?1 AND collection=?2 AND entity_id=?3)",params![parent,operation.collection,operation.entity_id],|r|r.get(0)).map_err(error)?;if !known{return Err("Отсутствует базовая операция; сначала загрузите предыдущие изменения".into());}}
        let mut versions=Self::versions_at(connection,key,&operation.collection,&operation.entity_id)?;
        versions.retain(|v|!operation.parents.contains(&v.op_id));versions.push(Version{op_id:operation.op_id.clone(),deleted:operation.deleted,value});versions.sort_by(|a,b|a.op_id.cmp(&b.op_id));
        let sealed=seal_with_key(&serde_json::to_value(versions).map_err(error)?,&format!("record:{}:{}",operation.collection,operation.entity_id),key)?;
        connection.execute("INSERT INTO operations(op_id,collection,entity_id,payload,outbound) VALUES (?1,?2,?3,?4,?5)",params![operation.op_id,operation.collection,operation.entity_id,encoded,outbound]).map_err(error)?;
        connection.execute("INSERT INTO records(collection,entity_id,sealed) VALUES (?1,?2,?3) ON CONFLICT(collection,entity_id) DO UPDATE SET sealed=excluded.sealed",params![operation.collection,operation.entity_id,sealed.to_string()]).map_err(error)?;Ok(true)
    }
    pub fn outbox(&self,limit:usize)->Result<Vec<Operation>,String>{let mut query=self.connection.prepare("SELECT payload FROM operations WHERE outbound=1 ORDER BY seq LIMIT ?1").map_err(error)?;let rows=query.query_map([limit.min(1000) as i64],|row|row.get::<_,String>(0)).map_err(error)?;let mut output=vec![];let mut size=0;for row in rows{let raw=row.map_err(error)?;if size+raw.len()>7*1024*1024{break;}size+=raw.len();output.push(serde_json::from_str(&raw).map_err(error)?);}Ok(output)}
    pub fn acknowledge(&mut self,ids:&[String])->Result<(),String>{if ids.len()>1000{return Err("Слишком много подтверждений".into());}let transaction=self.connection.transaction().map_err(error)?;for id in ids{transaction.execute("UPDATE operations SET outbound=0 WHERE op_id=?1",[id]).map_err(error)?;}transaction.commit().map_err(error)}
    pub fn cursor(&self,peer:&str)->Result<u64,String>{let value:Option<i64>=self.connection.query_row("SELECT value FROM cursors WHERE peer=?1",[peer],|r|r.get(0)).optional().map_err(error)?;Ok(value.unwrap_or(0) as u64)}
    pub fn receive(&mut self,peer:&str,items:&[(u64,Operation)],cursor:u64)->Result<(),String>{
        if peer.is_empty()||peer.len()>128||items.len()>1000||cursor>i64::MAX as u64{return Err("Некорректный пакет журнала".into());}
        let transaction=self.connection.transaction_with_behavior(TransactionBehavior::Immediate).map_err(error)?;
        let previous:Option<i64>=transaction.query_row("SELECT value FROM cursors WHERE peer=?1",[peer],|r|r.get(0)).optional().map_err(error)?;let previous=previous.unwrap_or(0) as u64;
        if cursor<previous{return Err("Курсор синхронизации уменьшился".into());}let mut last=0;
        for (sequence,operation) in items{if *sequence<=last||*sequence>cursor{return Err("Неверный порядок операций".into());}let inserted=Self::apply(&transaction,&self.key,&self.keys,operation,false)?;if *sequence<=previous&&inserted{return Err("Старый курсор содержит неизвестную операцию".into());}last=*sequence;}
        transaction.execute("INSERT INTO cursors(peer,value) VALUES (?1,?2) ON CONFLICT(peer) DO UPDATE SET value=excluded.value",params![peer,cursor as i64]).map_err(error)?;transaction.commit().map_err(error)
    }
    /// Local encryption key is independent of the shared transport keys.
    pub fn transport_keys(&mut self,keys:std::collections::BTreeMap<u32,[u8;32]>,epoch:u32)->Result<(),String>{
        if !keys.contains_key(&epoch)||epoch==0{return Err("Отсутствует текущий ключ группы".into());}self.keys=keys;self.epoch=epoch;Ok(())
    }
    pub fn snapshot(&self)->Result<Option<Value>,String>{
        let raw:Option<String>=self.connection.query_row("SELECT value FROM metadata WHERE key='snapshot'",[],|r|r.get(0)).optional().map_err(error)?;
        raw.map(|raw|open_with_key(&serde_json::from_str(&raw).map_err(error)?,"sync-snapshot",&self.key)).transpose()
    }
    /// The user's settings, record versions and outbox share a single SQLite commit.
    pub fn update_snapshot<F>(&mut self,initial:&Value,change:F)->Result<Value,String>
    where F:FnOnce(&mut Value)->Result<(),String>{
        let key=self.key;let keys=self.keys.clone();let device=self.device.clone();let epoch=self.epoch;
        let transaction=self.connection.transaction_with_behavior(TransactionBehavior::Immediate).map_err(error)?;
        let raw:Option<String>=transaction.query_row("SELECT value FROM metadata WHERE key='snapshot'",[],|r|r.get(0)).optional().map_err(error)?;
        let before=match raw{Some(raw)=>open_with_key(&serde_json::from_str(&raw).map_err(error)?,"sync-snapshot",&key)?,None=>initial.clone()};
        let mut after=before.clone();change(&mut after)?;
        let old=crate::sync_data::records(&before)?;let new=crate::sync_data::records(&after)?;
        for (collection,entity) in old.keys().chain(new.keys()).collect::<std::collections::BTreeSet<_>>(){
            if (collection=="chats"&&after["sync_share_chats"]!=true)||(collection=="vault"&&after["sync_share_vault"]!=true){continue;}
            let a=old.get(&(collection.clone(),entity.clone()));let b=new.get(&(collection.clone(),entity.clone()));
            let heads=Self::versions_at(&transaction,&key,collection,entity)?;
            if a==b && (!heads.is_empty() || b.is_none()){continue;}
            // Editing a conflict requires explicit resolution; ordinary saves cannot discard a sibling.
            if heads.len()>1{return Err("Сначала разрешите конфликт этой записи в синхронизации".into());}
            let mut operation=Operation{version:2,op_id:id(),device_id:device.clone(),collection:collection.clone(),entity_id:entity.clone(),parents:if a.is_none()&&b.is_some()&&heads.first().is_some_and(|v|Some(&v.value)!=b){vec![]}else{heads.iter().map(|v|v.op_id.clone()).collect()},deleted:b.is_none(),key_version:epoch,sealed:Value::Null};
            operation.validate()?;operation.sealed=seal_with_key(b.unwrap_or(&Value::Null),&operation.aad(),keys.get(&epoch).ok_or("Нет ключа группы")?)?;
            Self::apply(&transaction,&key,&keys,&operation,true)?;
        }
        Self::store_snapshot(&transaction,&key,&after)?;transaction.commit().map_err(error)?;Ok(after)
    }
    fn store_snapshot(connection:&Connection,key:&[u8;32],snapshot:&Value)->Result<(),String>{
        let sealed=seal_with_key(snapshot,"sync-snapshot",key)?;
        connection.execute("INSERT INTO metadata(key,value) VALUES ('snapshot',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[sealed.to_string()]).map_err(error)?;Ok(())
    }
    pub fn all_versions(&self)->Result<Vec<(String,String,Vec<Version>)>,String>{Self::all_versions_at(&self.connection,&self.key)}
    fn all_versions_at(connection:&Connection,key:&[u8;32])->Result<Vec<(String,String,Vec<Version>)>,String>{
        let mut query=connection.prepare("SELECT collection,entity_id FROM records ORDER BY collection,entity_id").map_err(error)?;
        let rows=query.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).map_err(error)?;
        rows.map(|r|{let(c,e)=r.map_err(error)?;let v=Self::versions_at(connection,key,&c,&e)?;Ok((c,e,v))}).collect()
    }
    pub fn receive_snapshot(&mut self,peer:&str,items:&[(u64,Operation)],cursor:u64,acks:&[String])->Result<Value,String>{
        if peer.is_empty()||peer.len()>128||items.len()>1000||acks.len()>1000||cursor>i64::MAX as u64{return Err("Некорректный пакет журнала".into());}
        let transaction=self.connection.transaction_with_behavior(TransactionBehavior::Immediate).map_err(error)?;
        let previous:Option<i64>=transaction.query_row("SELECT value FROM cursors WHERE peer=?1",[peer],|r|r.get(0)).optional().map_err(error)?;let previous=previous.unwrap_or(0) as u64;
        if cursor<previous{return Err("Курсор уменьшился".into());}let mut last=0;
        for (seq,op) in items{if *seq<=last||*seq>cursor||*seq>i64::MAX as u64{return Err("Неверный порядок операций".into());}
            let inserted=Self::apply(&transaction,&self.key,&self.keys,op,false)?;if *seq<=previous&&inserted{return Err("Неизвестная операция в старом курсоре".into());}last=*seq;}
        let raw:String=transaction.query_row("SELECT value FROM metadata WHERE key='snapshot'",[],|r|r.get(0)).map_err(error)?;
        let mut snapshot=open_with_key(&serde_json::from_str(&raw).map_err(error)?,"sync-snapshot",&self.key)?;
        crate::sync_data::project(&mut snapshot,&Self::all_versions_at(&transaction,&self.key)?)?;
        Self::store_snapshot(&transaction,&self.key,&snapshot)?;
        for id in acks{transaction.execute("UPDATE operations SET outbound=0 WHERE op_id=?1",[id]).map_err(error)?;}
        transaction.execute("INSERT INTO cursors(peer,value) VALUES (?1,?2) ON CONFLICT(peer) DO UPDATE SET value=excluded.value",params![peer,cursor as i64]).map_err(error)?;
        transaction.commit().map_err(error)?;Ok(snapshot)
    }
    pub fn resolve_snapshot(&mut self,collection:&str,entity:&str,expected:Vec<String>,value:Option<Value>)->Result<(),String>{
        let transaction=self.connection.transaction_with_behavior(TransactionBehavior::Immediate).map_err(error)?;
        let heads=Self::versions_at(&transaction,&self.key,collection,entity)?;
        let actual:std::collections::BTreeSet<_>=heads.iter().map(|v|v.op_id.clone()).collect();
        if actual!=expected.iter().cloned().collect(){return Err("Конфликт изменился; обновите версии".into());}
        let mut op=Operation{version:2,op_id:id(),device_id:self.device.clone(),collection:collection.into(),entity_id:entity.into(),parents:expected,deleted:value.is_none(),key_version:self.epoch,sealed:Value::Null};op.validate()?;
        op.sealed=seal_with_key(&value.unwrap_or(Value::Null),&op.aad(),self.keys.get(&self.epoch).ok_or("Нет ключа группы")?)?;Self::apply(&transaction,&self.key,&self.keys,&op,true)?;
        let raw:String=transaction.query_row("SELECT value FROM metadata WHERE key='snapshot'",[],|r|r.get(0)).map_err(error)?;
        let mut snapshot=open_with_key(&serde_json::from_str(&raw).map_err(error)?,"sync-snapshot",&self.key)?;
        crate::sync_data::project(&mut snapshot,&Self::all_versions_at(&transaction,&self.key)?)?;Self::store_snapshot(&transaction,&self.key,&snapshot)?;transaction.commit().map_err(error)
    }
    pub fn restore_snapshot(&mut self,snapshot:&Value,history:&[Operation])->Result<(),String>{
        if self.snapshot()?.is_some(){return Err("Восстановление требует пустого журнала".into());}
        crate::sync_data::records(snapshot)?;
        let tx=self.connection.transaction_with_behavior(TransactionBehavior::Immediate).map_err(error)?;
        for op in history{Self::apply(&tx,&self.key,&self.keys,op,false)?;}
        let mut restored=snapshot.clone();crate::sync_data::project(&mut restored,&Self::all_versions_at(&tx,&self.key)?)?;
        Self::store_snapshot(&tx,&self.key,&restored)?;tx.execute("INSERT INTO metadata(key,value) VALUES('restored','true')",[]).map_err(error)?;tx.execute("UPDATE operations SET outbound=2",[]).map_err(error)?;tx.commit().map_err(error)?;
        if history.is_empty(){self.update_snapshot(&restored,|_|Ok(()))?;}Ok(())
    }
    /// Re-encrypt only operations the relay confirms it has never accepted.
    pub fn rewrap_outbox(&mut self,ids:&[String])->Result<(),String>{
        if ids.len()>100{return Err("Слишком много операций ротации".into());}
        let tx=self.connection.transaction_with_behavior(TransactionBehavior::Immediate).map_err(error)?;
        for id in ids{let raw:String=tx.query_row("SELECT payload FROM operations WHERE op_id=?1 AND outbound IN(1,2)",[id],|r|r.get(0)).map_err(error)?;
            let mut op:Operation=serde_json::from_str(&raw).map_err(error)?;
            let value=open_with_key(&op.sealed,&op.aad(),self.keys.get(&op.key_version).ok_or("Нет ключа прошлой версии")?)?;
            if op.key_version>=self.epoch{return Err("Некорректный запрос смены ключа".into());}
            op.key_version=self.epoch;op.sealed=seal_with_key(&value,&op.aad(),self.keys.get(&self.epoch).ok_or("Нет нового ключа")?)?;
            tx.execute("UPDATE operations SET payload=?1 WHERE op_id=?2",params![serde_json::to_string(&op).map_err(error)?,id]).map_err(error)?;
        }tx.commit().map_err(error)
    }
    pub fn queue_recovery_history(&self)->Result<(),String>{self.connection.execute("UPDATE operations SET outbound=2",[]).map_err(error)?;Ok(())}
    pub fn hub_restore_rotated(&self)->Result<bool,String>{self.connection.query_row("SELECT EXISTS(SELECT 1 FROM metadata WHERE key='hub_restore_rotated' AND value='true')",[],|r|r.get(0)).map_err(error)}
    pub fn mark_hub_restore_rotated(&self)->Result<(),String>{self.connection.execute("INSERT OR REPLACE INTO metadata(key,value) VALUES('hub_restore_rotated','true')",[]).map_err(error)?;Ok(())}
    pub fn allow_repair(&self)->Result<(),String>{self.connection.execute("INSERT OR REPLACE INTO metadata(key,value) VALUES('repair','true')",[]).map_err(error)?;Ok(())}
    pub fn can_repair(&self)->Result<bool,String>{self.connection.query_row("SELECT EXISTS(SELECT 1 FROM metadata WHERE key IN ('restored','repair') AND value='true')",[],|r|r.get(0)).map_err(error)}
    pub fn is_restored(&self)->Result<bool,String>{self.connection.query_row("SELECT EXISTS(SELECT 1 FROM metadata WHERE key='restored' AND value='true')",[],|r|r.get(0)).map_err(error)}
    pub fn backup_state(&mut self)->Result<(Value,Vec<Operation>),String>{
        let tx=self.connection.transaction().map_err(error)?;
        let raw:String=tx.query_row("SELECT value FROM metadata WHERE key='snapshot'",[],|r|r.get(0)).map_err(error)?;
        let snapshot=open_with_key(&serde_json::from_str(&raw).map_err(error)?,"sync-snapshot",&self.key)?;
        let history={let mut query=tx.prepare("SELECT payload FROM operations ORDER BY seq").map_err(error)?;let rows=query.query_map([],|r|r.get::<_,String>(0)).map_err(error)?;rows.map(|r|serde_json::from_str(&r.map_err(error)?).map_err(error)).collect::<Result<Vec<Operation>,String>>()?};
        tx.commit().map_err(error)?;Ok((snapshot,history))
    }
    pub fn recovery_outbox(&self)->Result<Vec<Operation>,String>{
        let mut query=self.connection.prepare("SELECT payload FROM operations WHERE outbound=2 ORDER BY seq LIMIT 100").map_err(error)?;
        let rows=query.query_map([],|r|r.get::<_,String>(0)).map_err(error)?;let mut output=vec![];let mut size=0;for row in rows{let raw=row.map_err(error)?;if size+raw.len()>7*1024*1024{break;}size+=raw.len();output.push(serde_json::from_str(&raw).map_err(error)?);}Ok(output)
    }
    pub fn history(&self)->Result<Vec<Operation>,String>{
        let mut q=self.connection.prepare("SELECT payload FROM operations ORDER BY seq").map_err(error)?;
        let rows=q.query_map([],|r|r.get::<_,String>(0)).map_err(error)?;rows.map(|r|serde_json::from_str(&r.map_err(error)?).map_err(error)).collect()
    }
    // ponytail: history/tombstones retained; prune only after all active devices acknowledge.
}
#[cfg(test)]mod tests{use super::*;
    #[test]fn real_snapshot_edits_atomic_outbox_projection_and_recovery(){
        use crate::sync_relay::Relay;
        let dir=tempfile::tempdir().unwrap();let group="f".repeat(64);let group_key=[33;32];
        let initial=json!({"center_data":crate::sync_data::empty_center().to_string(),"chat_history":"[]","api_keys":{"openai":"fictional-never-sync"},"unknown_setting":true});
        let mut clients:Vec<_>=(0..3).map(|i|{let mut j=Journal::open(&dir.path().join(format!("client-{i}.sqlite")),[i as u8+1;32],format!("{i:064x}")).unwrap();j.transport_keys(std::collections::BTreeMap::from([(1,group_key)]),1).unwrap();j.update_snapshot(&initial,|_|Ok(())).unwrap();j}).collect();
        let mut relay=Relay::open(&dir.path().join("relay.sqlite"),&group,1).unwrap();
        for (i,j) in clients.iter_mut().enumerate(){relay.trust(&format!("{i:064x}")).unwrap();j.update_snapshot(&initial,|snapshot|{let mut center:Value=serde_json::from_str(snapshot["center_data"].as_str().unwrap()).unwrap();center["notes"].as_array_mut().unwrap().push(json!({"id":format!("n{i}"),"title":"fictional","text":"offline","updatedAt":"2026-10-12T12:00:00Z"}));snapshot["center_data"]=json!(center.to_string());Ok(())}).unwrap();
            let out=j.outbox(100).unwrap();assert_eq!(out.len(),1);assert_eq!(out[0].collection,"notes");relay.exchange(&format!("{i:064x}"),&group,0,&out).unwrap();}
        for (i,j) in clients.iter_mut().enumerate(){let ids=j.outbox(100).unwrap().iter().map(|o|o.op_id.clone()).collect::<Vec<_>>();let feed=relay.exchange(&format!("{i:064x}"),&group,0,&[]).unwrap();j.receive_snapshot(&group,&feed.items,feed.cursor,&ids).unwrap();assert!(j.outbox(100).unwrap().is_empty());let s=j.snapshot().unwrap().unwrap();assert_eq!(serde_json::from_str::<Value>(s["center_data"].as_str().unwrap()).unwrap()["notes"].as_array().unwrap().len(),3);assert_eq!(s["api_keys"]["openai"],"fictional-never-sync");}
        let before=clients[0].snapshot().unwrap();assert!(clients[0].update_snapshot(&initial,|s|{s["center_data"]=json!("corrupt");Ok(())}).is_err());assert_eq!(clients[0].snapshot().unwrap(),before);assert!(clients[0].outbox(100).unwrap().is_empty());
        let parent=clients[0].versions("notes","n0").unwrap()[0].op_id.clone();let edit=clients[0].local_edit("notes","n0",Some(json!({"id":"n0","text":"concurrent"})),vec![parent.clone()]).unwrap();let deletion=clients[1].local_edit("notes","n0",None,vec![parent]).unwrap();relay.exchange(&format!("{:064x}",0),&group,3,&[edit.clone()]).unwrap();relay.exchange(&format!("{:064x}",1),&group,3,&[deletion.clone()]).unwrap();
        for (i,j) in clients.iter_mut().enumerate(){let feed=relay.exchange(&format!("{i:064x}"),&group,3,&[]).unwrap();j.receive_snapshot(&group,&feed.items,feed.cursor,&[]).unwrap();assert_eq!(j.versions("notes","n0").unwrap().len(),2);let center:Value=serde_json::from_str(j.snapshot().unwrap().unwrap()["center_data"].as_str().unwrap()).unwrap();assert!(!center["notes"].as_array().unwrap().iter().any(|n|n["id"]=="n0"));}
        let history=clients[2].history().unwrap();let snapshot=clients[2].snapshot().unwrap().unwrap();let backup=crate::sync_profile::recovery_seal(&json!({"snapshot":snapshot,"history":history}),"fictional recovery master").unwrap();
        let recovered=crate::sync_profile::recovery_open(&backup,"fictional recovery master").unwrap();let mut fresh=Journal::open(&dir.path().join("fresh.sqlite"),[99;32],"a".repeat(64)).unwrap();fresh.transport_keys(std::collections::BTreeMap::from([(1,group_key)]),1).unwrap();fresh.restore_snapshot(&recovered["snapshot"],&serde_json::from_value::<Vec<Operation>>(recovered["history"].clone()).unwrap()).unwrap();assert_eq!(fresh.versions("notes","n0").unwrap().len(),2);assert_eq!(fresh.recovery_outbox().unwrap().len(),5);
        fresh.resolve_snapshot("notes","n0",vec![edit.op_id,deletion.op_id],None).unwrap();assert_eq!(fresh.versions("notes","n0").unwrap().len(),1);
        let current=fresh.snapshot().unwrap();let cursor=fresh.cursor(&group).unwrap();let mut corrupt=history[0].clone();corrupt.entity_id="tampered".into();assert!(fresh.receive_snapshot(&group,&[(6,corrupt)],6,&fresh.outbox(100).unwrap().iter().map(|o|o.op_id.clone()).collect::<Vec<_>>()).is_err());assert_eq!(fresh.cursor(&group).unwrap(),cursor);assert_eq!(fresh.snapshot().unwrap(),current);assert_eq!(fresh.outbox(100).unwrap().len(),1);
    }
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

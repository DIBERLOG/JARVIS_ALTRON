//! Only personal records cross the sync boundary. Settings and API keys never do.
use serde_json::{json,Value};
use std::collections::BTreeMap;
use crate::sync_journal::Version;

pub const LISTS:&[&str]=&["notes","reminders","birthdays","habits"];
pub const TRAINING_LISTS:&[&str]=&["exercises","plans","sessions"];
pub const TRAINING_FIELDS:&[&str]=&["profile","morningExercise","chargeSettings","chargeLevels"];
/// JSON numbers retain their value when Java renders 70.0 as 70 during rekeying.
pub fn equivalent(a:&Value,b:&Value)->bool{
    if a==b{return true;}
    match (a,b){
        (Value::Array(a),Value::Array(b))=>a.len()==b.len()&&a.iter().zip(b).all(|(a,b)|equivalent(a,b)),
        (Value::Object(a),Value::Object(b))=>a.len()==b.len()&&a.iter().all(|(k,a)|b.get(k).is_some_and(|b|equivalent(a,b))),
        (Value::Number(a),Value::Number(b))=>{
            let integer_float=|i:&serde_json::Number,f:f64|{
                if !f.is_finite()||f.fract()!=0.0{return false;}
                if let Some(n)=i.as_i64(){f>=i64::MIN as f64&&f<(i64::MAX as f64)&&f as i64==n}
                else if let Some(n)=i.as_u64(){f>=0.0&&f<(u64::MAX as f64)&&f as u64==n}else{false}
            };
            if a.is_f64()&&b.is_f64(){a.as_f64()==b.as_f64()}
            else if a.is_f64(){integer_float(b,a.as_f64().unwrap())}
            else if b.is_f64(){integer_float(a,b.as_f64().unwrap())}else{false}
        },_=>false
    }
}
pub fn empty_center()->Value{json!({"notes":[],"reminders":[],"birthdays":[],"habits":[]})}
fn parse(snapshot:&Value,key:&str,default:Value)->Result<Value,String>{
    match snapshot.get(key){
        None=>Ok(default),Some(value)=>{let raw=value.as_str().ok_or_else(||format!("Некорректный тип данных {key}"))?;if raw.is_empty(){Ok(default)}else{serde_json::from_str(raw).map_err(|_|format!("Повреждены данные {key}; синхронизация остановлена"))}}
    }
}
fn add_list(result:&mut BTreeMap<(String,String),Value>,collection:&str,list:&Value)->Result<(),String>{
    let items=list.as_array().ok_or("Ожидается список записей")?;if items.len()>10000{return Err("Слишком много записей".into());}
    for item in items{let id=item["id"].as_str().ok_or("Нет ID записи")?;
        if id.is_empty()||id.len()>128||id.chars().any(char::is_control)||result.insert((collection.into(),id.into()),item.clone()).is_some(){return Err("Некорректный или повторный ID".into());}}
    Ok(())
}
pub fn records(snapshot:&Value)->Result<BTreeMap<(String,String),Value>,String>{
    let center=parse(snapshot,"center_data",empty_center())?;if !center.is_object(){return Err("Некорректный Центр".into());}
    let mut result=BTreeMap::new();for list in LISTS{add_list(&mut result,list,center.get(*list).unwrap_or(&json!([])))?;}
    if let Some(training)=center.get("training").filter(|v|!v.is_null()){
        if !training.is_object(){return Err("Некорректные тренировки".into());}
        for list in TRAINING_LISTS{add_list(&mut result,&format!("training.{list}"),training.get(*list).unwrap_or(&json!([])))?;}
        for field in TRAINING_FIELDS{if let Some(value)=training.get(*field).filter(|v|!v.is_null()){result.insert((format!("training.{field}"),field.to_string()),value.clone());}}
    }
    if snapshot["sync_share_chats"]==true{add_list(&mut result,"chats",&parse(snapshot,"chat_history",json!([]))?)?;}
    // Vault is an already encrypted container and is never opened by this layer.
    if snapshot["sync_share_vault"]==true{if let Some(vault)=snapshot.get("sync_vault").filter(|v|!v.is_null()){validate_vault(vault)?;result.insert(("vault".into(),"container".into()),vault.clone());}}
    Ok(result)
}
pub fn validate_vault(v:&Value)->Result<(),String>{
    use base64::{Engine,engine::general_purpose::STANDARD as B64};
    if v["version"]!=1||B64.decode(v["salt"].as_str().unwrap_or("")).map(|v|v.len()).unwrap_or(0)!=16{return Err("Некорректный зашифрованный сейф".into());}
    for field in ["passwordKey","recoveryKey","cards"]{let block=&v[field];
        let data=B64.decode(block["data"].as_str().unwrap_or("")).map_err(|_|"Некорректный зашифрованный сейф")?;if field!="cards"&&data.len()!=48{return Err("Некорректный зашифрованный ключ сейфа".into());}
        if B64.decode(block["iv"].as_str().unwrap_or("")).map(|v|v.len()).unwrap_or(0)!=12||B64.decode(block["data"].as_str().unwrap_or("")).map(|v|v.len()).unwrap_or(0)<16{return Err("Некорректный зашифрованный сейф".into());}}
    Ok(())
}
pub fn project(snapshot:&mut Value,records:&[(String,String,Vec<Version>)])->Result<(),String>{
    let mut center=parse(snapshot,"center_data",empty_center())?;
    let mut chats=parse(snapshot,"chat_history",json!([]))?;
    for (collection,entity,heads) in records{
        // Deterministic projection; a delete/edit conflict stays deleted until explicit resolution.
        let value=if heads.iter().any(|v|v.deleted){Value::Null}else{heads.first().map(|v|v.value.clone()).unwrap_or(Value::Null)};
        if collection=="vault"{if snapshot["sync_share_vault"]==true{if !value.is_null(){validate_vault(&value)?;}snapshot["sync_vault"]=value;}continue;}
        if let Some(field)=collection.strip_prefix("training.").filter(|f|TRAINING_FIELDS.contains(f)){
            if !center["training"].is_object(){center["training"]=json!({});}center["training"][field]=value;continue;
        }
        let list=if collection=="chats"{if snapshot["sync_share_chats"]!=true{continue;} &mut chats}
        else if let Some(field)=collection.strip_prefix("training."){
            if !center["training"].is_object(){center["training"]=json!({});} &mut center["training"][field]
        }else{&mut center[collection]};
        if list.is_null(){*list=json!([]);}let items=list.as_array_mut().ok_or("Повреждён список проекции")?;
        items.retain(|v|v["id"].as_str()!=Some(entity));if !value.is_null(){if value["id"].as_str()!=Some(entity){return Err("ID содержимого не совпадает с операцией".into());}items.push(value);}
    }
    snapshot["center_data"]=json!(center.to_string());snapshot["chat_history"]=json!(chats.to_string());records_check(snapshot)
}
fn records_check(snapshot:&Value)->Result<(),String>{records(snapshot).map(|_|())}

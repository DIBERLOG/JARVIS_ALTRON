//! Portable mind maps live inside notes, including their selected local attachments.
use serde_json::Value;
use std::collections::BTreeSet;
use base64::{Engine, engine::general_purpose::STANDARD};

pub fn validate(map:&Value)->Result<(),String>{
    let check=||->Option<()>{
        if map["version"].as_u64()!=Some(1)||serde_json::to_vec(map).ok()?.len()>3_400_000{return None;}
        let nodes=map["nodes"].as_array()?;let edges=map["edges"].as_array()?;
        if nodes.is_empty()||nodes.len()>80||edges.len()>160{return None;}
        let text=|v:&Value,max:usize|v.as_str().is_some_and(|s|s.chars().count()<=max);
        let mut ids=BTreeSet::new();
        let valid_ids=|list:&Vec<Value>,max:usize|{let mut ids=BTreeSet::new();list.len()<=max&&list.iter().all(|n|n["id"].as_str().is_some_and(|id|!id.is_empty()&&id.len()<=128&&ids.insert(id)))};
        if !valid_ids(nodes,80)||!valid_ids(edges,160){return None;}
        for node in nodes{
            ids.insert(node["id"].as_str()?);
            if !text(&node["title"],120)||!text(&node["text"],4000)||!matches!(node["kind"].as_str(),Some("idea"|"goal"|"plan"|"images"|"note")){return None;}
            for axis in ["x","y"]{let n=node[axis].as_f64()?;if !n.is_finite()||n.abs()>10000.{return None;}}
            let items=node["items"].as_array()?;let images=node["images"].as_array()?;let files=node["files"].as_array()?;
            if !valid_ids(items,100)||!valid_ids(images,8)||!valid_ids(files,8){return None;}
            for item in items{if !text(&item["text"],1000)||!item["done"].is_boolean(){return None;}}
            for image in images{let src=image["src"].as_str()?;let (prefix,data)=src.split_once(",")?;
                if !text(&image["name"],160)||src.len()>2_800_000||!matches!(prefix,"data:image/jpeg;base64"|"data:image/png;base64"|"data:image/webp;base64"|"data:image/gif;base64")||data.is_empty()||!data.bytes().all(|b|b.is_ascii_alphanumeric()||b"+/=".contains(&b)){return None;}}
            for file in files{let size=file["size"].as_u64()?;let data=file["data"].as_str()?;
                if !text(&file["name"],160)||!(1..=700000).contains(&size)||data.len()>940000||!matches!(file["mime"].as_str(),Some("application/pdf"|"text/plain"|"text/markdown"|"application/vnd.openxmlformats-officedocument.wordprocessingml.document"))||STANDARD.decode(data).ok()?.len()!=size as usize{return None;}}
        }
        let mut pairs=BTreeSet::new();
        for edge in edges{let from=edge["from"].as_str()?;let to=edge["to"].as_str()?;if from==to||!ids.contains(from)||!ids.contains(to)||!pairs.insert(if from<to{(from,to)}else{(to,from)}){return None;}}
        Some(())
    };
    check().ok_or_else(||"Некорректная карта мыслей; исходные данные сохранены".into())
}

#[cfg(test)] mod tests{
    use super::*;use serde_json::json;
    fn fixture()->Value{json!({"version":1,"nodes":[{"id":"a","kind":"idea","title":"Проект","text":"План","x":10,"y":20,"items":[{"id":"step","text":"Сделать","done":false}],"images":[],"files":[{"id":"file","name":"plan.txt","mime":"text/plain","data":"YWJj","size":3}]}],"edges":[]})}
    #[test]fn sync_records_keep_the_entire_map(){let map:Value=serde_json::from_str(include_str!("../../../mobile/android/app/src/androidTest/assets/desktop-mindmap-fixture.json")).unwrap();let center=json!({"notes":[{"id":"mindmap","title":"Project","text":"Summary","type":"mindmap","mindMap":map}],"reminders":[],"birthdays":[],"habits":[]});let records=crate::sync_data::records(&json!({"center_data":center.to_string()})).unwrap();assert_eq!(records[&("notes".into(),"mindmap".into())]["mindMap"],map);}
    #[test]fn shared_android_fixture_is_portable(){let map:Value=serde_json::from_str(include_str!("../../../mobile/android/app/src/androidTest/assets/desktop-mindmap-fixture.json")).unwrap();validate(&map).unwrap();}
    #[test]fn map_payload_and_binary_attachment_roundtrip(){let m=fixture();validate(&m).unwrap();let m:Value=serde_json::from_str(&m.to_string()).unwrap();validate(&m).unwrap();}
    #[test]fn reject_invalid_edges_files_and_remote_images(){let mut m=fixture();m["edges"]=json!([{"id":"e","from":"a","to":"missing"}]);assert!(validate(&m).is_err());m=fixture();m["nodes"][0]["files"][0]["size"]=json!(4);assert!(validate(&m).is_err());m=fixture();m["nodes"][0]["images"]=json!([{"id":"img","name":"x","src":"https://example.invalid/x.png"}]);assert!(validate(&m).is_err());}
}

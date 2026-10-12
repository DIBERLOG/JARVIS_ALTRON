use serde::{de::DeserializeOwned, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::Path,
};

pub fn read<T: DeserializeOwned>(path: &Path) -> io::Result<T> {
    serde_json::from_slice(&fs::read(path)?).map_err(io::Error::other)
}

pub fn replace<T: Serialize>(path: &Path, data: &T) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("Missing storage directory"))?;
    fs::create_dir_all(parent)?;
    let bytes = serde_json::to_vec_pretty(data).map_err(io::Error::other)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(&bytes)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|failure| failure.error)?;
    Ok(())
}

/// Lock, read the latest disk state, validate, then commit atomically.
/// A malformed existing file is never overwritten with defaults.
pub fn transaction<T, F>(path: &Path, initial: &T, update: F) -> Result<T, String>
where
    T: Serialize + DeserializeOwned + Clone,
    F: FnOnce(&mut T) -> Result<(), String>,
{
    transaction_with_codec(path,initial,update,Ok,|value|Ok(value.clone()))
}

/// Same lock/atomic commit contract, with authenticated storage encoding.
pub fn transaction_with_codec<T,F,D,E>(path:&Path,initial:&T,update:F,decode:D,encode:E)->Result<T,String>
where T:Serialize+DeserializeOwned+Clone,F:FnOnce(&mut T)->Result<(),String>,D:FnOnce(serde_json::Value)->Result<serde_json::Value,String>,E:FnOnce(&serde_json::Value)->Result<serde_json::Value,String>{
    let parent = path.parent().ok_or("Missing storage directory")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let guard = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path.with_extension("db.lock"))
        .map_err(|e| e.to_string())?;
    guard.lock().map_err(|e| e.to_string())?;
    let original: serde_json::Value = match read(path) {
        Ok(data) => data,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            serde_json::to_value(initial).map_err(|e| e.to_string())?
        }
        Err(error) => {
            return Err(format!(
                "Existing settings could not be read; preserved unchanged: {error}"
            ))
        }
    };
    let original=decode(original)?;
    let mut value: T = serde_json::from_value(original.clone()).map_err(|e| e.to_string())?;
    update(&mut value)?;
    let mut output = original;
    let updated = serde_json::to_value(&value).map_err(|e| e.to_string())?;
    // Keep fields a newer client added but the current typed client cannot see.
    match (&mut output, updated) {
        (serde_json::Value::Object(existing), serde_json::Value::Object(updated)) => {
            existing.extend(updated)
        }
        (existing, updated) => *existing = updated,
    }
    replace(path, &encode(&output)?).map_err(|e| e.to_string())?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    #[test]
    fn transaction_keeps_changes_from_other_writers() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("app.db");
        let initial = json!({"voice":"s2", "notes":["keep me"]});
        replace(&path, &initial).unwrap();
        transaction(&path, &initial, |data| {
            data["timer"] = json!(300);
            Ok(())
        })
        .unwrap();
        transaction(&path, &initial, |data| {
            data["voice"] = json!("xtts");
            Ok(())
        })
        .unwrap();
        let data: Value = read(&path).unwrap();
        assert_eq!(data["timer"], 300);
        assert_eq!(data["notes"][0], "keep me");
    }
    #[test]
    fn failed_validation_leaves_file_intact() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("app.db");
        let initial = json!({"voice":"s2"});
        replace(&path, &initial).unwrap();
        assert!(transaction(&path, &initial, |data| {
            data["voice"] = json!("bad");
            Err("invalid".into())
        })
        .is_err());
        assert_eq!(read::<Value>(&path).unwrap(), initial);
    }
    #[test]
    fn corrupt_file_is_not_reset() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("app.db");
        fs::write(&path, "broken original data").unwrap();
        assert!(transaction(&path, &json!({}), |_| Ok(())).is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "broken original data");
    }
    #[test]
    fn typed_clients_preserve_unknown_fields() {
        #[derive(Clone, serde::Serialize, serde::Deserialize)]
        struct Client {
            voice: String,
        }
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("app.db");
        replace(
            &path,
            &json!({"voice":"s2", "future_setting":{"enabled":true}}),
        )
        .unwrap();
        transaction(&path, &Client { voice: "s2".into() }, |data| {
            data.voice = "xtts".into();
            Ok(())
        })
        .unwrap();
        let saved: Value = read(&path).unwrap();
        assert_eq!(saved["voice"], "xtts");
        assert_eq!(saved["future_setting"]["enabled"], true);
    }
    #[test]
    fn concurrent_writers_do_not_lose_updates() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("app.db");
        replace(&path, &json!({"count":0})).unwrap();
        let writers: Vec<_> = (0..2)
            .map(|_| {
                let path = path.clone();
                std::thread::spawn(move || {
                    for _ in 0..10 {
                        transaction(&path, &json!({"count":0}), |data| {
                            data["count"] = json!(data["count"].as_u64().unwrap() + 1);
                            Ok(())
                        })
                        .unwrap();
                    }
                })
            })
            .collect();
        for writer in writers {
            writer.join().unwrap();
        }
        assert_eq!(read::<Value>(&path).unwrap()["count"], 20);
    }
}

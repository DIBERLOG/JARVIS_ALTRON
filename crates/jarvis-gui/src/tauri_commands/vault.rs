use tauri::Manager;
pub(super) static VAULT_IO:parking_lot::Mutex<()>=parking_lot::Mutex::new(());
fn path(app:&tauri::AppHandle)->Result<std::path::PathBuf,String>{
    let dir=app.path().app_data_dir().map_err(|_|"Не удалось найти папку хранилища")?;
    std::fs::create_dir_all(&dir).map_err(|_|"Не удалось создать папку хранилища")?;
    Ok(dir.join("password-vault-v1.json"))
}
#[tauri::command]
pub fn password_vault_load(app:tauri::AppHandle)->Result<Option<String>,String>{
    let _guard=VAULT_IO.lock();
    let file=path(&app)?;
    match std::fs::read_to_string(file){Ok(value)=>Ok(Some(value)),Err(error) if error.kind()==std::io::ErrorKind::NotFound=>Ok(None),Err(_)=>Err("Не удалось открыть файл хранилища".into())}
}
#[tauri::command]
pub fn password_vault_save(app:tauri::AppHandle,data:String,expected:Option<String>,check_current:Option<bool>)->Result<(),String>{
    let _guard=VAULT_IO.lock();
    if data.len()>36_000_000{return Err("Хранилище слишком большое".into())}
    let value:serde_json::Value=serde_json::from_str(&data).map_err(|_|"Некорректный формат хранилища")?;
    if value["version"]!=1||!value["cards"]["data"].is_string()||!value["passwordKey"]["data"].is_string()||!value["recoveryKey"]["data"].is_string(){return Err("Некорректный формат хранилища".into())}
    let file=path(&app)?;
    if check_current.unwrap_or(false){
        let current=match std::fs::read_to_string(&file){Ok(v)=>Some(v),Err(e) if e.kind()==std::io::ErrorKind::NotFound=>None,Err(_)=>return Err("Не удалось проверить текущую версию сейфа".into())};
        let parse=|v:&Option<String>|->Result<Option<serde_json::Value>,String>{v.as_ref().map(|s|serde_json::from_str(s).map_err(|_|"Некорректная версия сейфа".to_string())).transpose()};
        if parse(&current)?!=parse(&expected)?{return Err("Сейф изменён после открытия, возможно при синхронизации. Изменения не перезаписаны. Заблокируйте сейф и обновите страницу перед повторным открытием.".into())}
    }
    let temporary=file.with_extension("tmp");
    {use std::io::Write;let mut output=std::fs::File::create(&temporary).map_err(|_|"Не удалось сохранить хранилище")?;output.write_all(data.as_bytes()).and_then(|_|output.sync_all()).map_err(|_|"Не удалось сохранить хранилище")?;}
    std::fs::rename(temporary,file).map_err(|_|"Не удалось заменить файл хранилища. Изменения не сохранены".into())
}

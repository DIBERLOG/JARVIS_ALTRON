//! Shared credential reader; GUI need not enable the audio/IPC server feature.
pub(crate) fn credential_path()->Result<std::path::PathBuf,String>{crate::APP_CONFIG_DIR.get().map(|p|p.join(".ipc-session-v1")).ok_or("IPC credentials unavailable".into())}
pub fn read_credentials()->Result<String,String>{
    use std::io::Read;
    let path=credential_path()?;
    #[cfg(unix)] {use std::os::unix::fs::PermissionsExt;if std::fs::metadata(&path).map_err(|_|"Перезапустите голосовое ядро для защищённого подключения")?.permissions().mode()&0o077!=0{return Err("Небезопасные права файла IPC".into());}}
    let mut token=String::new();std::fs::File::open(path).map_err(|_|"Перезапустите голосовое ядро для защищённого подключения")?.take(128).read_to_string(&mut token).map_err(|_|"Не удалось прочитать параметры IPC")?;
    if token.len()!=64||!token.bytes().all(|b|b.is_ascii_hexdigit()){return Err("Некорректные параметры IPC".into());}Ok(token)
}

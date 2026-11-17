//! Native Fish S2 voice: starts the local audio.cpp server and speaks sentence by sentence.
//! No Python is needed, so an installed copy only needs the downloaded server and model.
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{mpsc, Mutex},
    time::{Duration, Instant},
};

use sha2::{Digest, Sha256};

use crate::components::{self, S2Assets, S2_MODEL_ID};

pub fn available() -> bool { components::s2_assets().is_some() }

/// Minimal HTTP/1.1 over loopback: the server never leaves 127.0.0.1.
fn http(method: &str, path: &str, body: &[u8], timeout: Duration) -> std::io::Result<(u16, Vec<u8>)> {
    let port = components::s2_port();
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2))?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    write!(stream, "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len())?;
    stream.write_all(body)?;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw)?;
    parse_response(&raw).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "bad HTTP response"))
}

fn parse_response(raw: &[u8]) -> Option<(u16, Vec<u8>)> {
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = String::from_utf8_lossy(&raw[..split]).to_ascii_lowercase();
    let status = head.split_whitespace().nth(1)?.parse().ok()?;
    let mut body = raw[split + 4..].to_vec();
    if head.contains("transfer-encoding: chunked") {
        let (mut out, mut rest) = (Vec::new(), body.as_slice());
        loop {
            let line = rest.windows(2).position(|w| w == b"\r\n")?;
            let size = usize::from_str_radix(String::from_utf8_lossy(&rest[..line]).split(';').next()?.trim(), 16).ok()?;
            if size == 0 { break; }
            out.extend_from_slice(rest.get(line + 2..line + 2 + size)?);
            rest = rest.get(line + 4 + size..)?;
        }
        body = out;
    }
    Some((status, body))
}

/// Ok(true): our server answers; Ok(false): nothing listens; Err: the port belongs to something else.
fn ready() -> Result<bool, String> {
    match http("GET", "/v1/models", b"", Duration::from_secs(5)) {
        Ok((200, body)) if String::from_utf8_lossy(&body).contains(S2_MODEL_ID) => Ok(true),
        Ok(_) => Err(format!("port {} is used by another program", components::s2_port())),
        Err(_) => Ok(false),
    }
}

static STARTING: Mutex<()> = Mutex::new(());

fn ensure_server(assets: &S2Assets) -> Result<(), String> {
    if ready()? { return Ok(()); }
    let _guard = STARTING.lock().map_err(|_| "S2 start lock poisoned")?;
    if ready()? { return Ok(()); }
    if assets.device.is_some() {
        let config = serde_json::to_vec_pretty(&components::s2_server_config(assets)).map_err(|e| e.to_string())?;
        std::fs::write(&assets.config, config).map_err(|e| format!("S2 config: {e}"))?;
    }
    let log = std::fs::File::create(assets.workdir.join("server.log")).map_err(|e| e.to_string())?;
    let mut command = Command::new(&assets.server);
    command.arg("--config").arg(&assets.config).arg("--no-ui").current_dir(&assets.workdir)
        .stdin(Stdio::null()).stdout(log.try_clone().map_err(|e| e.to_string())?).stderr(log);
    #[cfg(windows)] {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command.spawn().map_err(|e| format!("S2 server did not start: {e}"))?;
    #[cfg(windows)] job::close_with_this_process(&child);
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        if ready()? { break; }
        // A second JARVIS process may have won the port; its server is just as good.
        if child.try_wait().ok().flatten().is_some() && !ready()? { return Err("S2 server exited; see server.log".into()); }
        if Instant::now() > deadline { let _ = child.kill(); return Err("S2 server startup timed out".into()); }
        std::thread::sleep(Duration::from_millis(300));
    }
    // The job object, not this handle, ends the server with JARVIS; reap it in the background.
    std::thread::spawn(move || { let _ = child.wait(); });
    Ok(())
}

#[cfg(windows)]
mod job {
    use std::{os::windows::io::AsRawHandle, sync::OnceLock};
    use windows_sys::Win32::System::JobObjects::*;

    struct Job(isize);
    // A job handle is a process-wide kernel handle; it is never closed while JARVIS runs.
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}
    static JOB: OnceLock<Option<Job>> = OnceLock::new();

    /// The server dies when the JARVIS process that started it exits, even after a crash.
    pub fn close_with_this_process(child: &std::process::Child) {
        let job = JOB.get_or_init(|| unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() { return None; }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = SetInformationJobObject(handle, JobObjectExtendedLimitInformation,
                &info as *const _ as *const _, std::mem::size_of_val(&info) as u32);
            (ok != 0).then_some(Job(handle as isize))
        });
        if let Some(Job(handle)) = job {
            unsafe { AssignProcessToJobObject(*handle as _, child.as_raw_handle() as _); }
        }
    }
}

/// Sentence-sized pieces of at most `limit` characters; short ones are merged.
pub fn split_text(text: &str, limit: usize) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = text.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        current.push(*c);
        let ends = matches!(c, '.' | '!' | '?' | '…') && chars.get(i + 1).map_or(true, |n| n.is_whitespace());
        if ends { sentences.push(current.trim().to_string()); current.clear(); }
    }
    if !current.trim().is_empty() { sentences.push(current.trim().to_string()); }
    let mut pieces: Vec<String> = Vec::new();
    for mut sentence in sentences.into_iter().filter(|s| !s.is_empty()) {
        while sentence.chars().count() > limit {
            let head: String = sentence.chars().take(limit).collect();
            let comma = [", ", "; ", " — ", ": "].iter().filter_map(|m| head.rfind(m).map(|i| i + 1)).max();
            let cut = comma.filter(|&i| head[..i].chars().count() > limit / 3)
                .or_else(|| head.rfind(' ')).filter(|&i| i > 0).unwrap_or(head.len());
            pieces.push(sentence[..cut].trim().to_string());
            sentence = sentence[cut..].trim().to_string();
        }
        match pieces.last_mut() {
            Some(last) if last.chars().count() + sentence.chars().count() < 90 => { last.push(' '); last.push_str(&sentence); }
            _ if !sentence.is_empty() => pieces.push(sentence),
            _ => {}
        }
    }
    if pieces.is_empty() { vec![text.trim().to_string()] } else { pieces }
}

fn profile(assets: &S2Assets) -> String {
    let stamp = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos());
    format!("native-s2:{}:{}:{}:28:.7:.8:30", assets.model.display(), stamp(&assets.model), stamp(&components::s2_reference()))
}

/// Cached WAV for one piece of text.
fn synthesize(assets: &S2Assets, profile: &str, text: &str) -> Result<PathBuf, String> {
    std::fs::create_dir_all(&assets.cache).map_err(|e| e.to_string())?;
    let key = format!("{:x}", Sha256::digest(format!("{profile}{text}").as_bytes()));
    let output = assets.cache.join(format!("{key}.wav"));
    if output.is_file() { return Ok(output); }
    let body = serde_json::json!({"model": S2_MODEL_ID, "input": text, "seed": 28, "temperature": 0.7,
        "top_p": 0.8, "top_k": 30, "options": {"max_tokens": 1024}});
    let (status, wav) = http("POST", "/v1/audio/speech", body.to_string().as_bytes(), Duration::from_secs(180))
        .map_err(|e| format!("S2 request failed: {e}"))?;
    if status != 200 || !wav.starts_with(b"RIFF") || wav.len() < 1000 {
        return Err(format!("S2 returned no audio (HTTP {status})"));
    }
    let temporary = output.with_extension("tmp");
    std::fs::write(&temporary, wav).map_err(|e| e.to_string())?;
    std::fs::rename(&temporary, &output).map_err(|e| e.to_string())?;
    Ok(output)
}

/// Speaks `text`; the next sentence is synthesized while the current one plays.
/// `cancelled` turns true when the user says «стоп» or a newer reply replaces this one.
pub fn speak(text: &str, cancelled: impl Fn() -> bool + Send + Sync + Clone + 'static) -> Result<(), String> {
    let assets = components::s2_assets().ok_or("S2 is not installed")?;
    ensure_server(&assets)?;
    let profile = profile(&assets);
    let (sender, receiver) = mpsc::sync_channel::<Result<PathBuf, String>>(2);
    let pieces = split_text(text, 180);
    let producer_cancelled = cancelled.clone();
    std::thread::spawn(move || {
        for piece in pieces {
            if producer_cancelled() { return; }
            let result = synthesize(&assets, &profile, &piece);
            let failed = result.is_err();
            if sender.send(result).is_err() || failed { return; }
        }
    });
    for result in receiver {
        let path = result?;
        if cancelled() { return Ok(()); }
        crate::audio::play_sound_cancellable(&path, cancelled.clone());
        if cancelled() { return Ok(()); }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_replies_are_split_into_sentences() {
        let text = "Первая новость. Вторая новость, очень длинная, ".to_string() + &"слово ".repeat(60) + "конец! Ок.";
        let pieces = split_text(&text, 180);
        assert!(pieces.len() >= 3, "{pieces:?}");
        assert!(pieces.iter().all(|p| p.chars().count() <= 180));
        assert_eq!(split_text("Да. Нет.", 180), vec!["Да. Нет."]);
        assert_eq!(split_text("Без точки", 180), vec!["Без точки"]);
    }

    #[test]
    #[ignore = "needs the local S2 server and model; uses the GPU"]
    fn native_server_synthesizes_a_wav() {
        let assets = components::s2_assets().expect("S2 assets");
        ensure_server(&assets).unwrap();
        let text = format!("Проверка встроенного голоса номер {}.", std::process::id());
        let wav = synthesize(&assets, &profile(&assets), &text).unwrap();
        assert!(std::fs::metadata(&wav).unwrap().len() > 10_000);
        let _ = std::fs::remove_file(wav);
    }

    #[test]
    fn chunked_and_plain_responses_parse() {
        assert_eq!(parse_response(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok"), Some((200, b"ok".to_vec())));
        let chunked = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nab\r\n1\r\nc\r\n0\r\n\r\n";
        assert_eq!(parse_response(chunked), Some((200, b"abc".to_vec())));
    }
}

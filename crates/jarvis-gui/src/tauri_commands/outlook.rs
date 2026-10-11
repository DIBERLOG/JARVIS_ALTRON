//! Microsoft Graph public-client integration. Tokens never leave this module or reach disk.
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use reqwest::blocking::Client;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

const SCOPES: &str = "offline_access User.Read Mail.ReadWrite Mail.Send";
#[derive(Default)]
struct Session {
    initialized: bool,
    local: bool,
    profile: String,
    client_id: String,
    device: String,
    device_deadline: Option<Instant>,
    next_poll: Option<Instant>,
    interval: u64,
    access: String,
    refresh: String,
    expires: Option<Instant>,
    account: Value,
    confirmation: Option<(String, Value, Instant)>,
    sequence: u64,
    compose_id: Option<String>,
    compose_message: Option<Value>,
}
static SESSION: Lazy<Mutex<Session>> = Lazy::new(|| Mutex::new(Session::default()));

fn binding_path() -> Result<std::path::PathBuf, String> {
    jarvis_core::APP_DIRS.get().map(|dirs| dirs.config_dir.join("outlook-binding-v1.json"))
        .ok_or_else(|| "Каталог настроек JARVIS недоступен.".into())
}
// Only profile metadata: never credentials, messages or send confirmations.
fn restore_binding(s: &mut Session, value: &Value) -> bool {
    let profile=field(value,"profile");
    let email=field(value,"email");
    if value["version"].as_u64()!=Some(1) || profile.is_empty() || profile.len()>256
        || profile.chars().any(|c| c.is_control() || c=='"')
        || email.contains([';',','])
        || message_body(&json!({"to":email,"subject":"binding","body":"binding"})).is_err() { return false; }
    s.local=true; s.profile=profile.to_owned();
    s.account=json!({"email":email,"name":field(value,"name")});
    true
}
fn save_binding(s: &Session) -> Result<(), String> {
    let path=binding_path()?;
    let temporary=path.with_extension("tmp");
    let binding=json!({"version":1,"profile":s.profile,"email":s.account["email"],"name":s.account["name"]});
    let mut checked=Session::default();
    if !restore_binding(&mut checked,&binding) { return Err("Outlook вернул некорректный профиль. Подключение не сохранено.".into()); }
    std::fs::write(&temporary,binding.to_string()).and_then(|_| std::fs::rename(&temporary,&path))
        .map_err(|_| "Не удалось сохранить подключение Outlook. Попробуйте подключить снова.".into())
}

fn client() -> Result<Client, String> {
    Client::builder().timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none()).build()
        .map_err(|_| "Не удалось подготовить подключение к Microsoft.".into())
}
fn public_status(s: &Session) -> Value {
    json!({"connected": s.local || !s.access.is_empty(), "account": s.account, "provider": if s.local {"local"} else {"graph"},
        "pending": !s.device.is_empty(), "sessionOnly": !s.local, "localSupported": cfg!(windows)})
}
fn field<'a>(v: &'a Value, name: &str) -> &'a str { v[name].as_str().unwrap_or("") }
fn valid_client_id(value: &str) -> bool {
    value.len() == 36 && value.bytes().enumerate().all(|(i, b)|
        if [8,13,18,23].contains(&i) { b == b'-' } else { b.is_ascii_hexdigit() })
}
fn message_body(data: &Value) -> Result<Value, String> {
    let address = field(data, "to").trim();
    let subject = field(data, "subject").trim();
    let body = field(data, "body").trim();
    let addresses: Vec<_> = address.split([';', ',']).map(str::trim).collect();
    if addresses.is_empty() || addresses.len() > 50 || addresses.iter().any(|address|
        address.len() > 254 || address.chars().any(|c| c.is_whitespace() || c.is_control())
        || address.contains(['<','>']) || address.matches('@').count() != 1
        || address.split('@').any(|part| part.is_empty()) || !address.split('@').nth(1).unwrap_or("").contains('.')) {
        return Err("Укажите до 50 полных адресов через точку с запятой.".into());
    }
    if subject.is_empty() || subject.len() > 500 || body.is_empty() || body.len() > 100_000 {
        return Err("Заполните тему и текст письма (текст — до 100 тысяч символов).".into());
    }
    Ok(json!({"subject": subject, "body": {"contentType":"Text", "content": body},
        "toRecipients": addresses.iter().map(|address| json!({"emailAddress":{"address":address}})).collect::<Vec<_>>()}))
}
#[cfg(windows)]
fn local_request(action: &str, data: Value, s: &Session) -> Result<Value, String> {
    use std::{io::{Read, Write}, process::{Command, Stdio}, os::windows::process::CommandExt};
    let mut command = Command::new("C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe");
    command.args(["-NoProfile", "-NonInteractive", "-Sta", "-Command", include_str!("outlook_local.ps1")])
        .creation_flags(0x08000000).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    let mut child = command.spawn().map_err(|_| "Не удалось открыть локальное подключение Outlook.")?;
    let input = json!({"action":action,"email":s.account["email"],"profile":s.profile,"id":data["id"],"message":data});
    child.stdin.take().unwrap().write_all(input.to_string().as_bytes()).map_err(|_| "Запрос Outlook прерван.")?;
    let mut stdout = child.stdout.take().unwrap();
    let reader = std::thread::spawn(move || { let mut bytes=Vec::new(); stdout.by_ref().take(2_000_000).read_to_end(&mut bytes).map(|_| bytes) });
    let deadline = Instant::now()+Duration::from_secs(60);
    loop {
        if child.try_wait().map_err(|_| "Проверка Outlook прервана.")?.is_some() { break; }
        if Instant::now() >= deadline { let _=child.kill(); let _=child.wait(); return Err("Outlook не ответил. Для отправки статус неизвестен: проверьте «Отправленные» перед повторением.".into()); }
        std::thread::sleep(Duration::from_millis(50));
    }
    let bytes = reader.join().map_err(|_| "Ответ Outlook прерван.")?.map_err(|_| "Ответ Outlook не прочитан.")?;
    let value: Value=serde_json::from_slice(&bytes).map_err(|_| "Некорректный ответ локального Outlook.")?;
    match field(&value,"error") {
        "classic_missing" => return Err("Классический Outlook не найден на этом компьютере. Установите классический Outlook; новый Outlook не подходит.".into()),
        "classic_not_ready" => return Err("Классический Outlook запущен, но ещё не готов. Завершите выбор профиля или настройку аккаунта в его окне и повторите команду.".into()),
        "classic_closed" => return Err("Классический Outlook закрыт. Скажите «открой почту», затем повторите действие.".into()),
        "recipient_missing" => return Err("В черновике Outlook пока нет полного адреса. Введите его в поле «Кому», нажмите Tab и скажите «Адрес указал».".into()),
        "recipient_not_found" => return Err("Outlook не нашёл однозначного получателя. Назовите полное имя или продиктуйте адрес.".into()),
        "profile_invalid" => return Err("Не удалось открыть сохранённый профиль Outlook. Откройте его вручную и подключитесь заново.".into()),
        _ => {}
    }
    if value["error"] == "draft_changed" { return Err("Черновик в Outlook изменён после проверки или уже отправлен. JARVIS не отправил его. Проверьте черновик вручную; для нового голосового письма начните заново.".into()); }
    if value["error"].is_string() { return Err("Outlook не выполнил запрос. Оставьте классический Outlook открытым с выбранным профилем и проверьте запрос безопасности. Если отправляли письмо — проверьте «Отправленные» перед повторением.".into()); }
    Ok(value)
}
#[cfg(not(windows))]
fn local_request(_: &str, _: Value, _: &Session) -> Result<Value,String> { Err("Локальный Outlook доступен только в Windows.".into()) }
fn accept_tokens(s: &mut Session, tokens: &Value) -> Result<(), String> {
    let access = field(tokens, "access_token");
    if access.is_empty() { return Err("Microsoft не предоставил доступ. Повторите вход.".into()); }
    s.access = access.to_owned();
    if let Some(refresh) = tokens["refresh_token"].as_str() { s.refresh = refresh.to_owned(); }
    let lifetime = tokens["expires_in"].as_u64().unwrap_or(3600).clamp(60, 86400);
    s.expires = Some(Instant::now() + Duration::from_secs(lifetime.saturating_sub(30)));
    Ok(())
}
fn ensure_access(s: &mut Session, http: &Client) -> Result<(), String> {
    if s.access.is_empty() { return Err("Сначала подключите Outlook в личном центре.".into()); }
    if s.expires.is_some_and(|expiry| Instant::now() >= expiry) {
        let response = http.post("https://login.microsoftonline.com/common/oauth2/v2.0/token")
            .form(&[("client_id", s.client_id.as_str()), ("grant_type", "refresh_token"),
                ("refresh_token", s.refresh.as_str()), ("scope", SCOPES)])
            .send().map_err(|_| "Нет связи с Microsoft. Проверьте интернет.".to_string())?;
        if !response.status().is_success() {
            s.access.clear(); s.refresh.clear(); s.confirmation = None;
            return Err("Срок подключения истёк. Войдите в Outlook снова.".into());
        }
        let tokens: Value = response.json().map_err(|_| "Некорректный ответ Microsoft.".to_string())?;
        accept_tokens(s, &tokens)?;
    }
    Ok(())
}
fn graph_url(parts: &[&str]) -> Result<reqwest::Url, String> {
    let mut url = reqwest::Url::parse("https://graph.microsoft.com/v1.0/").map_err(|_| "Ошибка адреса Microsoft.")?;
    url.path_segments_mut().map_err(|_| "Ошибка адреса Microsoft.")?.pop_if_empty().extend(parts);
    Ok(url)
}
fn graph_error(status: reqwest::StatusCode) -> String {
    match status.as_u16() {
        401 => "Microsoft отклонил доступ. Отключите аккаунт и войдите снова.",
        403 => "Не хватает разрешений Microsoft. Для рабочей почты может потребоваться согласие администратора.",
        404 => "Письмо больше не найдено. Обновите список.",
        429 => "Microsoft ограничил частоту запросов. Подождите немного.",
        _ => "Microsoft не выполнил запрос. Попробуйте позже.",
    }.into()
}
fn take_confirmation(s: &mut Session, ticket: &str) -> Result<Value, String> {
    let (expected, message, expiry) = s.confirmation.take()
        .ok_or("Сначала проверьте письмо и подтвердите отправку.")?;
    if ticket != expected || Instant::now() >= expiry {
        return Err("Подтверждение устарело. Проверьте письмо заново.".into());
    }
    Ok(message)
}
fn get_json(http: &Client, s: &Session, url: reqwest::Url) -> Result<Value, String> {
    let response = http.get(url).bearer_auth(&s.access)
        .header("Prefer", "outlook.body-content-type=\"text\"").send()
        .map_err(|_| "Не удалось связаться с Outlook. Проверьте интернет.".to_string())?;
    if !response.status().is_success() { return Err(graph_error(response.status())); }
    response.json().map_err(|_| "Не удалось прочитать ответ Outlook.".into())
}
fn request(action: &str, data: Value) -> Result<Value, String> {
    // Serialisation also prevents logout/refresh races and duplicate confirmation consumption.
    let mut s = SESSION.lock();
    if !s.initialized {
        if cfg!(windows) { if let Ok(path)=binding_path() {
            if let Ok(raw)=std::fs::read_to_string(path) {
                if let Ok(value)=serde_json::from_str::<Value>(&raw) { restore_binding(&mut s,&value); }
            }
        } }
        s.initialized=true;
    }
    if action == "status" { return Ok(public_status(&s)); }
    if action == "disconnect" {
        let path=binding_path()?;
        match std::fs::remove_file(path) {
            Ok(()) => (),
            Err(e) if e.kind()==std::io::ErrorKind::NotFound => (),
            Err(_) => return Err("Не удалось удалить сохранённую привязку Outlook.".into()),
        }
        *s = Session { initialized:true, ..Default::default() }; return Ok(public_status(&s));
    }
    if action == "cancel_send" { s.confirmation = None; return Ok(json!({"cancelled":true})); }
    if action == "open_classic" { return local_request("launch",Value::Null,&s); }
    if action == "connect_local" {
        if s.local || !s.access.is_empty() { return Err("Сначала отключите текущий аккаунт.".into()); }
        let result=local_request("connect", Value::Null, &s)?;
        let connected=Session { initialized:true, local:true, account:result["account"].clone(), profile:field(&result,"profile").to_owned(), ..Default::default() };
        save_binding(&connected)?;
        *s=connected;
        return Ok(public_status(&s));
    }
    if s.local {
        return match action {
            "recipient_suggestions" | "resolve_recipient" => {
                let name=field(&data,"name").trim();
                if name.len()>200 || name.chars().any(char::is_control) || name.contains([';',',','<','>']) {
                    return Err("Назовите одного получателя, сэр.".into());
                }
                if action=="resolve_recipient" && name.is_empty() { return Err("Назовите имя получателя.".into()); }
                let result=local_request(action,json!({"name":name}),&s)?;
                let candidates=result["candidates"].as_array().ok_or("Outlook не вернул список получателей.")?;
                if candidates.len()>5 || candidates.iter().any(|candidate|
                    field(candidate,"name").len()>500 || field(candidate,"email").contains([';',','])
                    || message_body(&json!({"to":candidate["email"],"subject":"recipient","body":"recipient"})).is_err()) {
                    return Err("Outlook вернул некорректный адрес. Введите получателя вручную.".into());
                }
                Ok(result)
            }
            "compose" => {
                let result=local_request("compose",Value::Null,&s)?;
                s.compose_id=Some(field(&result,"nativeId").to_owned());
                s.compose_message=None;
                s.confirmation=None;
                Ok(result)
            }
            "compose_read" => {
                let id=s.compose_id.as_ref().ok_or("Сначала скажите «Напиши письмо», чтобы открыть черновик JARVIS.")?;
                let result=local_request("compose_read",json!({"composeId":id}),&s)?;
                let draft=&result["draft"];
                // Incomplete drafts are allowed, but recipient syntax and size remain bounded.
                let mut snapshot=message_body(&json!({"to":draft["to"],"subject":"draft","body":"draft"}))?;
                if field(draft,"subject").len()>1000 || field(draft,"body").len()>100000 {
                    return Err("Черновик слишком большой для голосового управления.".into());
                }
                snapshot["subject"]=draft["subject"].clone();
                snapshot["body"]["content"]=draft["body"].clone();
                s.compose_message=Some(snapshot);
                s.confirmation=None;
                Ok(result)
            }
            "prepare_send" => {
                let mut message=message_body(&data)?;
                if data["native"].as_bool()==Some(true) {
                    let mut preview_message=message.clone();
                    if let Some(id)=&s.compose_id { preview_message["composeId"]=json!(id); }
                    if let Some(previous)=&s.compose_message { preview_message["previous"]=previous.clone(); }
                    let preview=local_request("preview",preview_message,&s)?;
                    let id=field(&preview,"nativeId");
                    if id.is_empty() { return Err("Outlook не открыл черновик. Отправка не разрешена.".into()); }
                    message["nativeId"]=json!(id);
                    s.compose_id=Some(id.to_owned());
                    s.compose_message=Some(message.clone());
                }
                s.sequence+=1;
                let ticket=format!("local-{}-{}",s.sequence,std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());
                s.confirmation=Some((ticket.clone(),message,Instant::now()+Duration::from_secs(120)));
                Ok(json!({"ticket":ticket,"expiresIn":120}))
            }
            "send" => { let message=take_confirmation(&mut s,field(&data,"ticket"))?; s.compose_id=None; local_request("send",message,&s) }
            "draft" => local_request("draft",message_body(&data)?,&s),
            "inbox" | "message" | "show_inbox" | "show_message" => local_request(action,data,&s),
            _ => Err("Неизвестная команда локального Outlook.".into()),
        };
    }
    let http = client()?;
    if action == "connect" {
        let id = field(&data, "clientId").trim();
        if !valid_client_id(id) { return Err("Вставьте Application (client) ID из регистрации Microsoft — не секретный ключ.".into()); }
        if !s.access.is_empty() { return Err("Сначала отключите текущий аккаунт.".into()); }
        let response = http.post("https://login.microsoftonline.com/common/oauth2/v2.0/devicecode")
            .form(&[("client_id", id), ("scope", SCOPES)]).send()
            .map_err(|_| "Не удалось начать вход Microsoft. Проверьте интернет.".to_string())?;
        if !response.status().is_success() { return Err("Microsoft отклонил регистрацию. Проверьте Client ID, поддержку обоих типов аккаунтов и включённый public client flow.".into()); }
        let result: Value = response.json().map_err(|_| "Некорректный ответ Microsoft.".to_string())?;
        if field(&result, "device_code").is_empty() || field(&result,"user_code").is_empty() {
            return Err("Microsoft не выдал код входа.".into());
        }
        *s = Session { initialized:true, ..Default::default() }; s.client_id = id.to_owned(); s.device = field(&result, "device_code").to_owned();
        s.interval = result["interval"].as_u64().unwrap_or(5).clamp(5,60);
        let lifetime = result["expires_in"].as_u64().unwrap_or(900).clamp(60,1800);
        s.device_deadline = Some(Instant::now() + Duration::from_secs(lifetime));
        s.next_poll = Some(Instant::now() + Duration::from_secs(s.interval));
        return Ok(json!({"code": result["user_code"], "interval": s.interval, "expiresIn": lifetime,
            "verificationUrl": "https://microsoft.com/devicelogin"}));
    }
    if action == "poll" {
        if s.device.is_empty() { return Ok(public_status(&s)); }
        if s.device_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            s.device.clear(); return Err("Код входа истёк. Получите новый код.".into());
        }
        if s.next_poll.is_some_and(|next| Instant::now() < next) { return Ok(public_status(&s)); }
        s.next_poll = Some(Instant::now() + Duration::from_secs(s.interval));
        let response = http.post("https://login.microsoftonline.com/common/oauth2/v2.0/token")
            .form(&[("client_id", s.client_id.as_str()), ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("device_code", s.device.as_str())]).send()
            .map_err(|_| "Нет связи с Microsoft. Проверьте интернет.".to_string())?;
        let successful = response.status().is_success();
        let tokens: Value = response.json().map_err(|_| "Некорректный ответ Microsoft.".to_string())?;
        if !successful {
            match field(&tokens,"error") {
                "authorization_pending" => return Ok(public_status(&s)),
                "slow_down" => { s.interval = (s.interval + 5).min(60); s.next_poll = Some(Instant::now()+Duration::from_secs(s.interval)); return Ok(public_status(&s)); }
                _ => { s.device.clear(); return Err("Вход отменён или код истёк. Получите новый код.".into()); }
            }
        }
        accept_tokens(&mut s, &tokens)?; s.device.clear();
        let account = get_json(&http, &s, graph_url(&["me"])? )?;
        s.account = json!({"name":account["displayName"],"email":account["mail"].as_str().or(account["userPrincipalName"].as_str()).unwrap_or("")});
        return Ok(public_status(&s));
    }
    ensure_access(&mut s, &http)?;
    match action {
        "inbox" => {
            let mut url = graph_url(&["me","mailFolders","inbox","messages"])?;
            url.query_pairs_mut().append_pair("$top","50").append_pair("$orderby","receivedDateTime desc")
                .append_pair("$select","id,subject,from,receivedDateTime,bodyPreview,isRead");
            let result = get_json(&http,&s,url)?;
            Ok(json!({"messages":result["value"], "hasMore":result["@odata.nextLink"].is_string()}))
        }
        "message" => {
            let id = field(&data,"id");
            if id.is_empty() || id.len()>2048 { return Err("Выберите письмо из списка.".into()); }
            let mut url = graph_url(&["me","messages",id])?;
            url.query_pairs_mut().append_pair("$select","id,subject,from,receivedDateTime,body,isRead,toRecipients");
            get_json(&http,&s,url)
        }
        "prepare_send" => {
            let message = message_body(&data)?;
            s.sequence += 1;
            let ticket = format!("{}-{}",s.sequence, std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());
            s.confirmation = Some((ticket.clone(), message, Instant::now()+Duration::from_secs(120)));
            Ok(json!({"ticket":ticket,"expiresIn":120}))
        }
        "send" => {
            let ticket = field(&data,"ticket");
            // Consume before HTTP: even a timeout cannot cause an automatic duplicate send.
            let message = take_confirmation(&mut s,ticket)?;
            let response = http.post(graph_url(&["me","sendMail"])?).bearer_auth(&s.access)
                .json(&json!({"message":message,"saveToSentItems":true})).send()
                .map_err(|_| "Статус отправки неизвестен. Проверьте «Отправленные» в Outlook перед повторной отправкой.".to_string())?;
            if response.status().as_u16()!=202 { return Err(graph_error(response.status())); }
            Ok(json!({"accepted":true}))
        }
        "draft" => {
            let message = message_body(&data)?;
            let response = http.post(graph_url(&["me","messages"])?).bearer_auth(&s.access).json(&message).send()
                .map_err(|_| "Статус сохранения неизвестен. Проверьте черновики Outlook перед повторением.".to_string())?;
            if !response.status().is_success() { return Err(graph_error(response.status())); }
            Ok(json!({"saved":true}))
        }
        _ => Err("Неизвестная команда Outlook.".into()),
    }
}

#[tauri::command]
pub async fn outlook_request(action: String, data: Option<Value>) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || request(&action,data.unwrap_or(Value::Null)))
        .await.map_err(|_| "Запрос Outlook прерван.".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn restores_only_valid_profile_metadata() {
        let mut s=Session::default();
        assert!(restore_binding(&mut s,&json!({"version":1,"profile":"Outlook","email":"angel@jarvis.test","access":"SECRET"})));
        assert!(s.local); assert_eq!(s.profile,"Outlook"); assert!(s.access.is_empty()); assert!(s.confirmation.is_none());
        assert_eq!(public_status(&s)["sessionOnly"],false);
        for (profile,email) in [("bad\"profile","a@test.test"),("Outlook","a@test.test;b@test.test"),("Outlook","")] {
            assert!(!restore_binding(&mut Session::default(),&json!({"version":1,"profile":profile,"email":email})));
        }
    }
    #[test] fn multiple_recipients_are_preserved_and_bounded() {
        let message=message_body(&json!({"to":"test@jarvis.test; second@jarvis.test","subject":"Тест","body":"Текст"})).unwrap();
        assert_eq!(message["toRecipients"].as_array().unwrap().len(),2);
        assert_eq!(message["toRecipients"][1]["emailAddress"]["address"],"second@jarvis.test");
        let too_many=vec!["test@jarvis.test";51].join(";");
        assert!(message_body(&json!({"to":too_many,"subject":"Тест","body":"Текст"})).is_err());
    }
    #[test] fn rejects_secret_and_bad_ids() {
        assert!(valid_client_id("01234567-89ab-cdef-0123-456789abcdef"));
        assert!(!valid_client_id("client-secret"));
        assert!(!valid_client_id("https://evil.test"));
    }
    #[test] fn validates_one_recipient_and_plain_text() {
        assert!(message_body(&json!({"to":"a@example.com","subject":"Тест","body":"<script>не HTML</script>"})).is_ok());
        assert!(message_body(&json!({"to":"a@example.com; b@example.com","subject":"Тест","body":"Текст"})).is_ok());
        for address in ["","a@example.com,","A <a@example.com>","a@","a@example.com\r\nBcc:x@test.com"] {
            assert!(message_body(&json!({"to":address,"subject":"Тест","body":"Текст"})).is_err());
        }
    }
    #[test] fn graph_ids_cannot_change_host_or_path() {
        let url = graph_url(&["me","messages","https://evil.test/a?x=1"]).unwrap();
        assert_eq!(url.host_str(),Some("graph.microsoft.com"));
        assert!(url.as_str().contains("https:%2F%2Fevil.test%2Fa%3Fx=1"));
    }
    #[test] fn status_never_exposes_tokens() {
        let s = Session { access:"SECRET_ACCESS".into(),refresh:"SECRET_REFRESH".into(),device:"SECRET_DEVICE".into(),..Default::default() };
        let public=public_status(&s).to_string(); assert!(!public.contains("SECRET"));
    }
    #[test] fn confirmation_is_single_use_and_expires() {
        let mut s=Session::default();
        s.confirmation=Some(("ticket".into(),json!({"body":"original"}),Instant::now()+Duration::from_secs(120)));
        assert_eq!(take_confirmation(&mut s,"ticket").unwrap()["body"],"original");
        assert!(take_confirmation(&mut s,"ticket").is_err());
        s.confirmation=Some(("ticket".into(),Value::Null,Instant::now()-Duration::from_secs(1)));
        assert!(take_confirmation(&mut s,"ticket").is_err());
        s.confirmation=Some(("ticket".into(),Value::Null,Instant::now()+Duration::from_secs(120)));
        assert!(take_confirmation(&mut s,"another").is_err());
    }
}

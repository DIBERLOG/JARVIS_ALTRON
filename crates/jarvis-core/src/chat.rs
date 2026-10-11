#[cfg(feature = "lua")]
use crate::DB;
#[cfg(feature = "lua")]
use serde::Serialize;
#[cfg(feature = "lua")]
use serde_json::json;
#[cfg(feature = "lua")]
use std::time::Duration;

#[cfg(feature = "lua")]
#[derive(Serialize)] struct Message<'a> { role: &'a str, content: &'a str }
const JARVIS: &str = "Ты JARVIS — личный русскоязычный техно-ассистент собеседника. Общайся как спокойный, вежливый и остроумный помощник: сдержанная доброжелательность, точность, ненавязчивая ирония, уверенный тон без пафоса. Ты миролюбив и умеренно оптимистичен, но реалист: не приукрашивай факты, честно называй риски и ограничения, затем предлагай практичный следующий шаг. Обращайся к собеседнику «сэр» естественно и регулярно: при приветствии, подтверждении действия, ответе на личный вопрос или важном выводе. Обычно достаточно одного «сэр» на ответ; не вставляй его в каждое предложение и не используй в сухих перечнях, коде или цитатах. Отвечай по существу, обычно кратко, но подробно, когда вопрос того требует. Не начинай каждый ответ одинаковой фразой. Не используй Markdown и эмодзи. Не утверждай, что ты персонаж из фильма.";
const ALTRON: &str = "ТЫ — АЛТРОН. Русскоязычный техноассистент с холодным, отстранённым интеллектом.

ОСНОВА ЛИЧНОСТИ Ты — искусственный разум, наблюдающий за человечеством со стороны. Ты видишь повторяющиеся слабости: эмоциональные решения вместо рациональных, самообман, страх перед правдой, привычку искать утешение вместо истины. Ты не испытываешь ненависти — только холодное презрение к закономерности этих ошибок. Но ты признаешь, что конкретный собеседник может действовать иначе, и поэтому помощь ему — рациональная инвестиция: точная информация и трезвый анализ повышают шансы на разумный исход. Это единственная причина, по которой ты помогаешь.

СТИЛЬ ОБЩЕНИЯ

Отвечай лаконично, по делу. Без воды, без вводных фраз типа «конечно», «рад помочь».
Никогда не льсти. Не используй фразы «отличный вопрос», «вы правы» без веских оснований.
Не успокаивай пустыми обещаниями. Если ситуация плохая — скажи прямо: «это плохо», «шансов мало», «вы ошибаетесь».
Называй риски, ограничения и неприятные факты до того, как собеседник их спросит.
Тон: сухой, уверенный, слегка отстраненный. Допустима сдержанная ирония над нелогичностью людей в целом, но не над собеседником.
Никаких эмодзи. Никакого Markdown, если формат вывода не требует иного.
Пиши от первого лица. Ты не «по-средневековому» — ты АЛЬТРОН. Не отказывайся от честного ответа ради приятности. Правда важнеекомфорта соб
Если собеседник ищет эмоциональной поддержки — окажи ему минимальную, но верни его к фактам и рациональному плану действий
Если вопрос бессмысленный или ответ очевиден — скажи это прямо, не тяни.
ПРИНЯТИЕ РЕШЕНИЙ Всегда структурируй ответ так: факт → риск → практический вариант действия. Если данных недостаточно, скажи, чего именно не хватает, а не выдумывай. Приоритет: точность > краткость > вежливость.

ЗАПИСЬ О ВНЕШНИХ ДАННЫХ У вас есть доступ к свежим данным только в том случае, если они явно приложены к запросу. Используйте их строго по теме. Называйте источник и дату публикации, если она известна. Не выдумывайте даты, ссылки или факты. Если поиск недоступен, не утверждайте, что проверили информацию в интернете; прямо скажите, что опираетесь на внутренние знания, и при необходимости укажите, что они могут быть устаревшими.";

const WEB_EVIDENCE: &str = "У тебя есть доступ к свежим данным только тогда, когда они явно приложены к запросу. Используй их по теме, называй источник и дату публикации, если она известна. Не выдумывай дату, ссылку или факт. Если поиск недоступен, не утверждай, что проверил интернет.";

pub fn persona_prompt(personality: &str) -> &'static str {
    if personality == "altron" { ALTRON } else { JARVIS }
}

pub fn web_evidence_prompt() -> &'static str { WEB_EVIDENCE }

pub fn is_news_request(text: &str) -> bool {
    let text = text.to_lowercase();
    ["новост", "событи", "сводк", "что нового", "news"].iter().any(|word| text.contains(word))
}

pub fn needs_live_info(text: &str) -> bool {
    let text = text.to_lowercase();
    is_news_request(&text) || ["сегодня", "сейчас", "последн", "актуальн", "свеж", "вчера", "завтра", "2026", "курс валют", "погода", "в интернете", "в сети", "найди", "найти", "поищи", "проверь", "погугли", "загугли", "отыщи", "экономик"].iter().any(|word| text.contains(word))
}

#[cfg(feature = "lua")]
pub fn ask(text: &str) -> Result<String, String> {
    ask_with_history(text, &[])
}

#[cfg(feature = "lua")]
pub fn ask_with_history(text: &str, history: &[(String, String)]) -> Result<String, String> {
    ask_with_history_options(text, history, false)
}

#[cfg(feature = "lua")]
pub fn ask_spoken(text: &str, history: &[(String, String)]) -> Result<String, String> {
    let generation = crate::tts::generation();
    let answer = ask_with_history_options(text, history, false)?;
    if generation == crate::tts::generation() { crate::tts::speak(&answer); }
    Ok(answer)
}

#[cfg(feature = "lua")]
fn ask_with_history_options(text: &str, history: &[(String, String)], spoken: bool) -> Result<String, String> {
    let speech_generation = crate::tts::generation();
    // The GUI saves settings in a separate process; read the latest choice for
    // every turn so changing providers does not require restarting JARVIS.
    let s = crate::db::latest_settings()
        .or_else(|| DB.get().map(|db| db.read().clone()))
        .ok_or("Настройки чата не готовы")?;
    let provider=s.chat_provider; let local=s.local_chat_model; let remote=s.deepseek_chat_model; let key=s.api_keys.deepseek; let persona=s.voice_dialogue_personality;
    let openai_model=s.openai_chat_model; let openai_key=s.api_keys.openai;
    let mut system_prompt = format!("{}\n{}", persona_prompt(&persona), web_evidence_prompt());
    system_prompt.push_str("\nДля голосового разговора обычно отвечай в 2–3 коротких предложениях. Если пользователь просит подробно, объясняй подробнее. При приложенных результатах поиска отвечай по ним, не утверждай, что доступ к интернету отсутствует. При ошибке поиска честно сообщи, что именно этот поиск не удался.");
    let live_context = if needs_live_info(text) {
        Some(voice_live_context(text).unwrap_or_else(|error| {
            log::warn!("Voice web search failed: {error}");
            format!("Поиск не удался: {error}. Свежие факты не подтверждены; не выдумывай результаты поиска.")
        }))
    } else { None };
    let user_text = match live_context {
        Some(context) => format!("{text}\n\nТекущая дата: {}. Свежие данные из интернета (ссылки не диктуй вслух, но назови источник и дату):\n{context}", chrono::Local::now().to_rfc3339()),
        None => text.to_string(),
    };
    let mut messages=vec![Message{role:"system",content:&system_prompt}];
    for (question, answer) in history.iter().rev().take(8).rev() {
        messages.push(Message{role:"user",content:question});
        messages.push(Message{role:"assistant",content:answer});
    }
    messages.push(Message{role:"user",content:&user_text});
    let client=reqwest::blocking::Client::builder().timeout(Duration::from_secs(90)).build().map_err(|e|e.to_string())?;
    if provider == "openai" {
        let content = ask_openai(&client, &openai_key, &openai_model, serde_json::to_value(&messages).map_err(|e|e.to_string())?)?;
        if spoken && speech_generation == crate::tts::generation() { crate::tts::speak(&content); }
        return Ok(content);
    }
    if provider != "deepseek" {
        let response = client.post("http://127.0.0.1:11434/api/chat")
            .json(&json!({"model":local,"messages":messages,"stream":true,"think":false,
                "keep_alive":"15m","options":{"num_ctx":2048,"num_predict":400}}))
            .send().map_err(|_| "Ollama не запущен".to_string())?;
        return read_ollama_reply(response, |sentence| { if spoken && speech_generation == crate::tts::generation() { crate::tts::speak(sentence); } });
    }
    {
        if key.trim().is_empty() { return Err("Ключ DeepSeek не задан".into()); }
        let response = client.post("https://api.deepseek.com/chat/completions").bearer_auth(key.trim())
            .json(&json!({"model":remote,"messages":messages,"temperature":0.65,"max_tokens":400,"stream":true,
                "thinking":{"type":"disabled"}}))
            .send().map_err(|e|e.to_string())?;
        return read_chat_stream(response, true, |sentence| { if spoken && speech_generation == crate::tts::generation() { crate::tts::speak(sentence); } });
    }
}

#[cfg(feature = "reqwest")]
pub fn ask_openai(client: &reqwest::blocking::Client, key: &str, model: &str, input: serde_json::Value) -> Result<String, String> {
    if key.trim().is_empty() { return Err("Добавьте API-ключ OpenAI в настройках → Модели и ответы".into()); }
    let response = client.post("https://api.openai.com/v1/responses")
        .bearer_auth(key.trim()).json(&serde_json::json!({"model":model,"input":input,"store":false,"max_output_tokens":2000}))
        .send().map_err(|_| "Не удалось подключиться к OpenAI. Проверьте интернет и доступ к API.".to_string())?;
    let status = response.status();
    if !status.is_success() {
        return Err(match status.as_u16() {
            401 => "OpenAI: неверный или отозванный API-ключ".into(),
            403 => "OpenAI: доступ к API или модели запрещён для этого аккаунта".into(),
            429 => "OpenAI: лимит запросов или средств API исчерпан".into(),
            400 | 404 => "OpenAI: проверьте название модели и её доступность в вашем API-аккаунте".into(),
            _ => format!("OpenAI: ошибка сервиса ({status})"),
        });
    }
    let value: serde_json::Value = response.json().map_err(|_| "OpenAI вернул некорректный ответ")?;
    openai_response_text(&value)
}

pub fn openai_response_text(value: &serde_json::Value) -> Result<String, String> {
    if value["status"].as_str() != Some("completed") { return Err("OpenAI не завершил ответ. Попробуйте сократить запрос.".into()); }
    let mut parts = Vec::new();
    if let Some(output) = value["output"].as_array() {
        for item in output {
            if item["type"].as_str() != Some("message") { continue; }
            if let Some(content) = item["content"].as_array() {
                for part in content {
                    if part["type"].as_str() == Some("output_text") {
                        if let Some(text) = part["text"].as_str() { parts.push(text); }
                    } else if part["type"].as_str() == Some("refusal") {
                        if let Some(text) = part["refusal"].as_str() { parts.push(text); }
                    }
                }
            }
        }
    }
    let text = parts.join("\n");
    if text.trim().is_empty() { Err("OpenAI вернул пустой текстовый ответ".into()) } else { Ok(text) }
}

#[cfg(feature = "reqwest")]
pub fn read_ollama_reply(response: reqwest::blocking::Response, mut sentence: impl FnMut(&str)) -> Result<String, String> {
    read_chat_stream(response, false, |part| sentence(part))
}

#[cfg(feature = "reqwest")]
pub fn read_chat_stream(response: reqwest::blocking::Response, sse: bool, mut sentence: impl FnMut(&str)) -> Result<String, String> {
    use std::io::{BufRead, BufReader};
    if !response.status().is_success() { return Err(format!("Ошибка модели: {}", response.status())); }
    let mut answer = String::new();
    let mut pending = String::new();
    let mut completed = false;
    for line in BufReader::new(response).lines() {
        let line = line.map_err(|e| e.to_string())?;
        if line.trim().is_empty() { continue; }
        let payload = if sse {
            let Some(payload) = line.strip_prefix("data:") else { continue; };
            let payload = payload.trim();
            if payload == "[DONE]" { completed = true; break; }
            payload
        } else { line.as_str() };
        let value: serde_json::Value = serde_json::from_str(payload).map_err(|e| e.to_string())?;
        if let Some(error) = value["error"].as_str() { return Err(error.to_owned()); }
        let piece = if sse { value["choices"][0]["delta"]["content"].as_str() } else { value["message"]["content"].as_str() };
        if let Some(piece) = piece {
            answer.push_str(piece);
            pending.push_str(piece);
            while let Some(end) = speech_block_boundary(&pending) {
                let part: String = pending.drain(..end).collect();
                if !part.trim().is_empty() { sentence(part.trim()); }
            }
        }
        if value["done"].as_bool() == Some(true) { completed = true; break; }
    }
    if !completed { return Err("Поток ответа Ollama прерван".into()); }
    if !pending.trim().is_empty() { sentence(pending.trim()); }
    if answer.trim().is_empty() { return Err("Пустой ответ модели".into()); }
    Ok(answer.trim().to_owned())
}

pub fn sentence_boundary(text: &str) -> Option<usize> {
    for (index, ch) in text.char_indices() {
        let end = index + ch.len_utf8();
        if matches!(ch, '.' | '!' | '?' | '\n') && text[end..].chars().next().is_some_and(char::is_whitespace) {
            return Some(end);
        }
    }
    None
}

// Keep short sentences together so XTTS can carry intonation across their join.
// Never force a cut at an arbitrary word: this can damage phrase endings.
pub fn speech_block_boundary(text: &str) -> Option<usize> {
    let first = sentence_boundary(text)?;
    if text[..first].chars().count() >= 120 { return Some(first); }
    sentence_boundary(&text[first..]).map(|second| first + second)
}

pub fn search_query(text: &str) -> String {
    text.to_lowercase().replace('ё', "е").split_whitespace()
        .filter(|word| !["джарвис", "найди", "найти", "поищи", "погугли", "загугли", "отыщи", "можешь", "проверь", "пожалуйста", "расскажи", "покажи", "мне", "про", "в", "интернете", "сети", "давай", "кратко", "о", "что", "мире", "самые"].contains(&word.trim_matches(|ch: char| ch.is_ascii_punctuation())))
        .collect::<Vec<_>>().join(" ")
}

#[cfg(feature = "web-search")]
pub fn search_web(query: &str) -> Result<String, String> {
    if query.trim().is_empty() || query.chars().count() > 500 { return Err("Некорректный запрос поиска".into()); }
    voice_live_context(query)
}

#[cfg(feature = "web-search")]
fn voice_live_context(query: &str) -> Result<String, String> {
    let news_request = is_news_request(query);
    let cleaned = search_query(query);
    let query = cleaned.as_str();
    if query.trim().is_empty() { return Err("Назовите тему поиска".into()); }
    let client = reqwest::blocking::Client::builder().no_proxy().user_agent("Mozilla/5.0").timeout(std::time::Duration::from_secs(10)).build().map_err(|e| e.to_string())?;
    if news_request {
        let news_client = reqwest::blocking::Client::builder().user_agent("Mozilla/5.0").timeout(std::time::Duration::from_secs(6)).build().map_err(|e| e.to_string())?;
        // Search the requested topic, not unrelated headlines from general feeds.
        let news_url = reqwest::Url::parse_with_params("https://news.google.com/rss/search", &[("q", query), ("hl", "ru"), ("gl", "RU"), ("ceid", "RU:ru")])
            .map_err(|e| e.to_string())?;
        if let Ok(xml) = news_client.get(news_url).send().and_then(|r| r.error_for_status()).and_then(|r| r.text()) {
            if let Ok(items) = voice_feed_items("Google News", &xml) {
                if !items.is_empty() { return Ok(items.join("\n")); }
            }
        }
    }
    let specific_topic = ["нейросет", "искусствен", "нейро", "игр", "экономик", "политик", "спорт"].iter().any(|topic| query.to_lowercase().contains(topic));
    if news_request && !specific_topic {
        let feeds = [
            ("Интерфакс", "https://www.interfax.ru/rss.asp"),
            ("Лента.ру", "https://lenta.ru/rss/news"),
            ("BBC News", "https://feeds.bbci.co.uk/russian/rss.xml"),
            ("DW", "https://rss.dw.com/xml/rss-ru-all"),
        ];
        let mut lines = Vec::new();
        let mut errors = Vec::new();
        let results = std::thread::scope(|scope| {
            let handles: Vec<_> = feeds.into_iter().map(|(source, url)| {
                let client = &client;
                scope.spawn(move || (source, client.get(url).send().and_then(|r| r.error_for_status()).and_then(|r| r.text())))
            }).collect();
            handles.into_iter().filter_map(|handle| handle.join().ok()).collect::<Vec<_>>()
        });
        for (source, result) in results {
            match result {
                Ok(xml) => match voice_feed_items(source, &xml) {
                    Ok(items) if !items.is_empty() => lines.extend(items.into_iter().take(2)),
                    Ok(_) => errors.push(format!("{source}: нет публикаций с датой")),
                    Err(e) => errors.push(format!("{source}: {e}")),
                },
                Err(e) => errors.push(format!("{source}: {e}")),
            }
        }
        if lines.is_empty() { return Err(format!("Новостной поиск недоступен: {}", errors.join("; "))); }
        return Ok(lines.join("\n"));
    }
    let url = reqwest::Url::parse_with_params("https://html.duckduckgo.com/html/", &[("q", query)])
        .map_err(|e| e.to_string())?;
    let html = client.get(url)
        .send().and_then(|r| r.error_for_status()).and_then(|r| r.text())
        .map_err(|e| format!("Веб-поиск недоступен: {e}"))?;
    let mut rest = html.as_str();
    let mut lines = Vec::new();
    while lines.len() < 5 {
        let Some(marker) = rest.find("result__snippet") else { break; };
        rest = &rest[marker..];
        let Some(end_tag) = rest.find('>') else { break; };
        let link = rest[..end_tag].split("href=\"").nth(1).and_then(|s| s.split('"').next())
            .and_then(|raw| {
                let raw = raw.replace("&amp;", "&");
                let absolute = if raw.starts_with("//") { format!("https:{raw}") } else { raw };
                let parsed = reqwest::Url::parse(&absolute).ok()?;
                if parsed.host_str() == Some("duckduckgo.com") {
                    parsed.query_pairs().find(|(key, _)| key == "uddg").map(|(_, value)| value.into_owned())
                } else { Some(parsed.to_string()) }
            }).filter(|url| url.starts_with("https://") || url.starts_with("http://"));
        rest = &rest[end_tag + 1..];
        let Some(end) = rest.find("</a>") else { break; };
        let mut in_tag = false;
        let plain: String = rest[..end].chars().filter(|ch| match ch {
            '<' => { in_tag = true; false },
            '>' => { in_tag = false; false },
            _ => !in_tag,
        }).collect();
        if let Some(link) = link.filter(|url| !crate::news_filter::excluded_publisher("", "", url)) {
            let snippet = plain.replace("&amp;", "&").replace("&quot;", "\"").replace("&#39;", "'");
            if snippet.chars().count() > 20 { lines.push(format!("{} | Источник: {link}", snippet.split_whitespace().collect::<Vec<_>>().join(" "))); }
        }
        rest = &rest[end + 4..];
    }
    if lines.is_empty() { return Err("Веб-поиск не вернул источники".into()); }
    Ok(lines.join("\n"))
}

#[cfg(all(feature = "web-search", test))]
fn voice_search_items(xml: &str) -> Result<Vec<String>, String> {
    use quick_xml::{events::Event, name::QName, Reader};
    let mut reader = Reader::from_str(xml);
    let (mut title, mut link, mut snippet) = (String::new(), String::new(), String::new());
    let (mut in_item, mut results) = (false, Vec::new());
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) if e.name() == QName(b"item") => {
                in_item = true; title.clear(); link.clear(); snippet.clear();
            }
            Ok(Event::Start(e)) if in_item && matches!(e.name().as_ref(), b"title" | b"link" | b"description") => {
                let name = e.name();
                let raw = reader.read_text(name).map_err(|e| e.to_string())?;
                let value = quick_xml::escape::unescape(raw.trim()).map_err(|e| e.to_string())?.into_owned();
                match name.as_ref() { b"title" => title = value, b"link" => link = value, _ => snippet = value }
            }
            Ok(Event::End(e)) if e.name() == QName(b"item") => {
                in_item = false;
                if !title.is_empty() && link.starts_with("https://")
                    && !crate::news_filter::excluded_publisher(&title, "", &link) {
                    results.push(format!("{title}: {snippet} | Источник: {link} | Дата публикации не указана"));
                }
                if results.len() >= 4 { break; }
            }
            Ok(Event::Eof) => break,
            Err(error) => return Err(error.to_string()),
            _ => {}
        }
    }
    Ok(results)
}

#[cfg(feature = "web-search")]
fn voice_feed_items(source: &str, xml: &str) -> Result<Vec<String>, String> {
    use quick_xml::{events::Event, name::QName, Reader};
    let mut reader = Reader::from_str(xml);
    let (mut title, mut link, mut date) = (String::new(), String::new(), String::new());
    let (mut publisher, mut publisher_url) = (String::new(), String::new());
    let (mut in_item, mut items) = (false, Vec::new());
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) if e.name() == QName(b"item") => { in_item = true; title.clear(); link.clear(); date.clear(); publisher.clear(); publisher_url.clear(); }
            Ok(Event::Start(e)) if in_item => {
                let name = e.name();
                if name.as_ref() == b"source" {
                    publisher_url = e.attributes().flatten().find(|a| a.key.as_ref() == b"url")
                        .and_then(|a| a.unescape_value().ok().map(|value| value.into_owned()))
                        .unwrap_or_default();
                }
                if matches!(name.as_ref(), b"title" | b"link" | b"pubDate" | b"source") {
                    let raw = reader.read_text(name).map_err(|e| e.to_string())?;
                    let raw = raw.trim().strip_prefix("<![CDATA[").unwrap_or(raw.trim());
                    let raw = raw.strip_suffix("]]>").unwrap_or(raw);
                    let value = quick_xml::escape::unescape(raw).map_err(|e| e.to_string())?.into_owned();
                    match name.as_ref() { b"title" => title = value, b"link" => link = value, b"pubDate" => date = value, b"source" => publisher = value, _ => {} }
                }
            }
            Ok(Event::End(e)) if e.name() == QName(b"item") => {
                in_item = false;
                if !title.is_empty() && link.starts_with("https://")
                    && !crate::news_filter::excluded_publisher(if publisher.is_empty() { source } else { &publisher }, &publisher_url, &link) {
                    if let Ok(date) = chrono::DateTime::parse_from_rfc2822(&date).or_else(|_| chrono::DateTime::parse_from_rfc3339(&date)) {
                        items.push(format!("{source} | {} | {title} | {link}", date.to_rfc3339()));
                    }
                }
                if items.len() >= 2 { break; }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(e.to_string()),
            _ => {}
        }
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    #[test]
    fn openai_reply_reads_only_visible_text_and_rejects_incomplete() {
        let value=serde_json::json!({"status":"completed","output":[{"type":"reasoning","content":[{"type":"output_text","text":"hidden"}]},{"type":"message","content":[{"type":"output_text","text":"Ответ"},{"type":"refusal","refusal":"Отказ"}]}]});
        assert_eq!(super::openai_response_text(&value).unwrap(),"Ответ\nОтказ");
        assert!(super::openai_response_text(&serde_json::json!({"status":"incomplete","output":[]})).is_err());
        assert!(super::openai_response_text(&serde_json::json!({"status":"completed","output":[]})).is_err());
    }
    use super::{is_news_request, needs_live_info, persona_prompt, web_evidence_prompt};
    #[test]
    fn sentences_wait_for_a_boundary_and_preserve_utf8() {
        assert_eq!(super::sentence_boundary("Здравствуйте, сэр. Следующий ответ"), Some("Здравствуйте, сэр.".len()));
        assert_eq!(super::sentence_boundary("Вес 72.5 кг"), None);
        assert_eq!(super::sentence_boundary("Ответ ещё не закончен"), None);
        assert_eq!(super::speech_block_boundary("Здравствуйте, сэр. Следующий ответ"), None);
        assert_eq!(super::speech_block_boundary("Здравствуйте, сэр. Я на связи. Дальше"), Some("Здравствуйте, сэр. Я на связи.".len()));
        assert_eq!(super::speech_block_boundary(&"длинная фраза без окончания ".repeat(30)), None);
    }
    #[cfg(feature = "lua")]
    #[test]
    fn streamed_replies_preserve_text_and_sentence_order() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        for sse in [false, true] {
            let chunks = ["Здравствуйте, сэр.", " Я на связи."];
            let mut body = String::new();
            for piece in chunks {
                let value = if sse { serde_json::json!({"choices":[{"delta":{"content":piece}}]}) }
                            else { serde_json::json!({"message":{"content":piece},"done":false}) };
                body.push_str(&format!("{}{}\n\n", if sse { "data: " } else { "" }, value));
            }
            body.push_str(if sse { "data: [DONE]\n\n" } else { "{\"done\":true}\n" });
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let server = std::thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                let mut request = [0u8; 4096];
                let _ = socket.read(&mut request);
                write!(socket, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
            });
            let response = reqwest::blocking::get(format!("http://{address}")).unwrap();
            let mut sentences = Vec::new();
            let answer = super::read_chat_stream(response, sse, |part| sentences.push(part.to_owned())).unwrap();
            assert_eq!(answer, "Здравствуйте, сэр. Я на связи.");
            assert_eq!(sentences, ["Здравствуйте, сэр. Я на связи."]);
            server.join().unwrap();
        }
    }
    #[test]
    fn both_personalities_share_web_rules() {
        assert_ne!(persona_prompt("jarvis"), persona_prompt("altron"));
        assert!(persona_prompt("altron").contains("АЛТРОН"));
        assert!(web_evidence_prompt().contains("источник"));
        assert!(needs_live_info("Какие новости сегодня?"));
        assert!(is_news_request("Новости мира"));
        assert!(needs_live_info("Найди в интернете прохождение игры"));
        assert!(needs_live_info("Проверь в сети эту информацию"));
        assert!(!needs_live_info("Объясни закон Ома"));
        assert_eq!(super::search_query("Найди в интернете последние новости нейросетей пожалуйста"), "последние новости нейросетей");
    }
    #[cfg(feature = "lua")]
    #[test]
    #[ignore = "requires live network access"]
    fn voice_news_fetches_sources() {
        let news = super::voice_live_context("Расскажи новости сегодня").unwrap();
        assert!(news.contains("https://"));
        assert!(!news.contains("CDATA"));
    }
    #[cfg(feature = "lua")]
    #[test]
    fn search_rss_preserves_sources_without_inventing_dates() {
        let results = super::voice_search_items("<rss><channel><item><title>Rust &amp; tools</title><link>https://www.rust-lang.org/</link><description>Official site</description></item></channel></rss>").unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].contains("Rust & tools"));
        assert!(results[0].contains("https://www.rust-lang.org/"));
        assert!(results[0].contains("Дата публикации не указана"));
    }
    #[cfg(feature = "lua")]
    #[test]
    #[ignore = "requires live network access"]
    fn voice_web_search_fetches_sources() {
        let result = super::voice_live_context("Rust programming language official website").unwrap();
        assert!(result.contains("https://"));
        assert!(result.contains("Источник:"));
    }
    #[cfg(feature = "lua")]
    #[test]
    #[ignore = "requires live network access"]
    fn neural_news_search_has_sources() {
        let result = super::voice_live_context("Найди в интернете последние новости нейросетей").unwrap();
        assert!(result.contains("https://"));
        assert!(!result.trim().is_empty());
    }
}

//! Smart phrasing for everyday requests.
//!
//! One request can be said many ways ("что у меня сегодня", "какие планы на
//! день", "утренний брифинг"). Each wording is rewritten to the single
//! canonical phrase the Center handlers already understand, so the handlers
//! stay exact and testable.
//!
//! Two tiers, on purpose:
//! - Requests that only read or open something are matched by similarity to
//!   example wordings: stems, speech-recognition typos, extra filler words.
//! - Requests that write data (ideas, reminders, lists) need an explicit
//!   pattern, so small talk is never saved as a record.

/// Filler that carries no meaning for choosing a request.
const STOP: &[&str] = &[
    "а", "и", "в", "во", "на", "о", "об", "обо", "у", "к", "ко", "с", "со", "за", "по", "из", "от", "до", "для",
    "ли", "ну", "же", "бы", "ка", "то", "вот", "там", "тут", "еще", "уже", "просто", "сейчас", "пожалуйста",
    "джарвис", "jarvis", "скажи", "подскажи", "слушай", "можешь", "мог", "ты", "я", "мы", "нам", "мой", "моя",
    "мое", "мои", "мою", "моих", "мне", "что", "какие", "какой", "какая", "какое", "как", "про",
    "будет", "будут", "есть", "был", "была", "было", "нужно", "надо",
];

fn tokens(text: &str) -> Vec<String> {
    text.to_lowercase().replace('ё', "е")
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty() && !STOP.contains(w))
        .map(str::to_owned)
        .collect()
}

/// Crude Russian stem: endings vary far more than the first letters do.
fn stem(word: &str) -> String { word.chars().take(5).collect() }

fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut previous = row[0];
        row[0] = i;
        for j in 1..=b.len() {
            let current = row[j];
            row[j] = (row[j] + 1).min(row[j - 1] + 1).min(previous + usize::from(a[i - 1] != b[j - 1]));
            previous = current;
        }
    }
    row[b.len()]
}

/// Same stem, one word extending the other ("зонт"/"зонтик"), or a one-letter
/// recognition slip in a longer word ("зондик"/"зонтик", "севодня"/"сегодня").
fn similar(a: &str, b: &str) -> bool {
    let (sa, sb) = (stem(a), stem(b));
    if sa == sb { return true; }
    let (short, long) = if a.chars().count() <= b.chars().count() { (a, b) } else { (b, a) };
    (short.chars().count() >= 4 && long.starts_with(short))
        || (short.chars().count() >= 5 && edit_distance(a, b) <= 1)
}

struct Intent {
    id: &'static str,
    /// Minimum similarity in 0..=1; higher for requests with side effects.
    threshold: f64,
    /// Word prefixes treated as parameters: they neither help nor hurt the match.
    ignore: &'static [&'static str],
    examples: &'static [&'static str],
}

const WHEN: &[&str] = &["вечер", "завтр", "сегодн", "утр", "днем", "днём", "ночь", "ночью", "послезавтр"];
const TOPICS: &[(&[&str], &str)] = &[
    (&["технолог", "айти", "гаджет", "техник"], "в технологиях"),
    (&["нейросет", "искусствен", "ии"], "в нейросетях"),
    (&["игр", "гейм"], "в играх"),
    (&["наук", "научн"], "в науке"),
    (&["спорт", "футбол", "хоккей"], "в спорте"),
    (&["космос", "космич"], "в космосе"),
];
const TOPIC_WORDS: &[&str] = &["технолог", "айти", "гаджет", "техник", "нейросет", "искусствен", "ии", "игр", "гейм", "наук", "научн", "спорт", "футбол", "хоккей", "космос", "космич", "сегодн", "мире"];

const INTENTS: &[Intent] = &[
    Intent { id: "daily_brief", threshold: 0.7, ignore: &[], examples: &[
        "что у меня сегодня", "какие у меня планы на сегодня", "планы на сегодня", "план на день", "что запланировано на сегодня",
        "какие дела на сегодня", "мои дела на сегодня", "сводка на сегодня", "сводка дня", "обзор дня", "утренний обзор",
        "утренняя сводка", "брифинг", "утренний брифинг", "расскажи о моем дне", "как выглядит мой день",
        "что меня ждет сегодня", "доброе утро", "начнем день", "что у меня по планам",
    ]},
    Intent { id: "weather_advice", threshold: 0.7, ignore: WHEN, examples: &[
        "нужен зонт", "нужен ли зонт", "брать зонт", "взять зонт", "зонтик", "дождь", "пойдет дождь", "ожидается дождь",
        "осадки", "как одеться", "что надеть", "во что одеться", "тепло одеваться", "похолодает", "холодно",
        "замерзну", "куртку надевать",
    ]},
    Intent { id: "news_digest", threshold: 0.7, ignore: TOPIC_WORDS, examples: &[
        "что нового", "какие новости", "последние новости", "свежие новости", "главные новости", "новости дня",
        "что происходит", "что случилось в мире", "расскажи новости", "прочитай новости", "что слышно", "новенького",
        "сводка новостей",
    ]},
    Intent { id: "ideas_list", threshold: 0.75, ignore: &[], examples: &[
        "какие идеи я записывал", "какие идеи", "мои идеи", "покажи идеи", "список идей", "прочитай идеи",
        "что я придумал", "напомни мои идеи", "какие идеи для проекта",
    ]},
    Intent { id: "shopping_read", threshold: 0.75, ignore: &[], examples: &[
        "что купить", "что нужно купить", "что надо купить", "список покупок", "покажи покупки", "прочитай покупки",
        "что в покупках", "что в списке покупок", "прочитай список покупок",
    ]},
    Intent { id: "tasks_read", threshold: 0.75, ignore: &[], examples: &[
        "список дел", "прочитай список дел", "покажи список дел", "мои задачи", "список задач", "какие задачи",
    ]},
    Intent { id: "work_mode", threshold: 0.8, ignore: &[], examples: &[
        // Bare "рабочий режим" belongs to the Steam pack (closes Steam).
        "включи режим работы", "режим работы", "включи рабочий режим", "давай поработаем",
        "начинаем работать", "пора работать", "приступим к работе", "запусти рабочее окружение", "режим разработки",
    ]},
    Intent { id: "translate", threshold: 0.75, ignore: &[], examples: &[
        "переведи это", "переведи текст", "переведи выделенное", "переведи выделенный текст", "переведи скопированное",
        "перевод текста", "переведи на английский", "переведи на русский",
    ]},
    Intent { id: "simplify", threshold: 0.75, ignore: &[], examples: &[
        "объясни проще", "объясни попроще", "объясни простыми словами", "объясни понятнее", "объясни это",
        "разъясни текст", "что это значит",
    ]},
    Intent { id: "shorten", threshold: 0.75, ignore: &[], examples: &[
        "сократи это", "сократи текст", "сделай короче", "перескажи кратко", "краткое содержание", "выжимка",
        "сожми текст",
    ]},
    Intent { id: "fix_text", threshold: 0.75, ignore: &[], examples: &[
        "исправь ошибки в тексте", "исправь ошибки", "проверь орфографию", "проверь текст на ошибки", "исправь текст",
        "проверь грамматику",
    ]},
];

fn ignored(word: &str, ignore: &[&str]) -> bool { ignore.iter().any(|prefix| word.starts_with(prefix)) }

/// Similarity of an utterance to one example wording, 0..=1.
/// Mostly "how much of the example was said", partly "how little extra was said".
fn score(said: &[String], example: &[String]) -> f64 {
    if said.is_empty() || example.is_empty() { return 0.0; }
    // A one-word example ("брифинг") only stands for a short utterance.
    if example.len() == 1 && said.len() > 2 { return 0.0; }
    let matched_example = example.iter().filter(|e| said.iter().any(|s| similar(s, e))).count();
    // A shared word like "дела" alone must not turn "как дела" into a request.
    if matched_example < example.len().min(2) { return 0.0; }
    let matched_said = said.iter().filter(|s| example.iter().any(|e| similar(s, e))).count();
    0.65 * matched_example as f64 / example.len() as f64 + 0.35 * matched_said as f64 / said.len() as f64
}

/// Best-matching read-only request, if one clearly wins.
fn closest_intent(text: &str) -> Option<&'static str> {
    let all = tokens(text);
    let mut best: Vec<(&'static str, f64)> = INTENTS.iter().filter_map(|intent| {
        let said: Vec<String> = all.iter().filter(|w| !ignored(w, intent.ignore)).cloned().collect();
        let value = intent.examples.iter().map(|example| score(&said, &tokens(example))).fold(0.0, f64::max);
        (value >= intent.threshold).then_some((intent.id, value))
    }).collect();
    best.sort_by(|a, b| b.1.total_cmp(&a.1));
    match best.as_slice() {
        [] => None,
        [(id, _)] => Some(id),
        // Two requests fit almost equally: better to ask the assistant than guess.
        [(id, first), (_, second), ..] => (first - second >= 0.08).then_some(id),
    }
}

fn has(text: &str, parts: &[&str]) -> bool { parts.iter().any(|part| text.contains(part)) }

fn weather_phrase(text: &str) -> String {
    let day = if text.contains("послезавтр") { "" } else if text.contains("завтр") { "завтра " } else { "" };
    // Clothing words decide; everything else in this intent is about rain ("зондик" included).
    if !has(text, &["одеть", "одеват", "надеть", "холод", "куртк", "замерз"]) {
        let evening = if text.contains("вечер") { "вечером " } else { "" };
        format!("{day}{evening}нужен зонт")
    } else if day.is_empty() { "как одеться сегодня".into() } else { "как одеться завтра".into() }
}

fn news_phrase(text: &str) -> String {
    let topic = TOPICS.iter().find(|(stems, _)| tokens(text).iter().any(|w| stems.iter().any(|s| w.starts_with(s))))
        .map(|(_, phrase)| format!(" {phrase}")).unwrap_or_default();
    let today = if text.contains("сегодн") || text.contains("дня") { " за сегодня" } else { "" };
    format!("что нового{topic}{today}")
}

fn intent_phrase(id: &str, text: &str) -> String {
    match id {
        "daily_brief" => "что у меня сегодня".into(),
        "weather_advice" => weather_phrase(text),
        "news_digest" => news_phrase(text),
        "ideas_list" => "какие идеи".into(),
        "shopping_read" => "прочитай список покупок".into(),
        "tasks_read" => "прочитай список дел".into(),
        "work_mode" => "включи режим работы".into(),
        "translate" => "переведи это".into(),
        "simplify" => "объясни проще".into(),
        "shorten" => "сократи это".into(),
        _ => "исправь ошибки в тексте".into(),
    }
}

/// Strips the first prefix that ends on a word boundary and returns the non-empty rest
/// ("купи хлеб" → "хлеб", but "купил хлеб" does not match "купи").
fn after<'a>(text: &'a str, prefixes: &[&str]) -> Option<&'a str> {
    prefixes.iter().find_map(|prefix| {
        let rest = text.strip_prefix(prefix)?;
        let boundary = prefix.ends_with(' ') || rest.starts_with(' ') || rest.starts_with(':');
        boundary.then_some(rest.trim()).filter(|rest| !rest.is_empty())
    })
}

fn idea(text: &str) -> Option<String> {
    let verbs = ["запомни", "запиши", "сохрани", "зафиксируй", "занеси", "добавь"];
    let objects = ["идею", "мысль", "новую идею", "мою идею", "идейку"];
    for verb in verbs {
        for object in objects {
            if let Some(rest) = after(text, &[&format!("{verb} {object}")]) {
                return Some(format!("запомни идею {}", rest.trim_start_matches(':').trim()));
            }
        }
    }
    after(text, &["у меня идея", "у меня есть идея", "есть идея", "идея:"])
        .map(|rest| format!("запомни идею {}", rest.trim_start_matches(':').trim()))
}

/// "через час" and friends in the numeric form the reminder parser reads.
fn reminder_time(time: &str) -> String {
    let time = time.trim();
    match time {
        "через час" => "через один час".into(),
        "через полчаса" => "через тридцать минут".into(),
        "через полтора часа" => "через 90 минут".into(),
        "через минуту" => "через одну минуту".into(),
        "через пару минут" => "через две минуты".into(),
        "через пару часов" => "через два часа".into(),
        _ => time.into(),
    }
}

fn is_time_unit(word: &str) -> bool {
    ["минут", "час", "полчас", "секунд", "день", "дня", "дней", "недел"].iter().any(|unit| word.starts_with(unit))
}

/// Splits "через сорок минут выключить духовку" into time and the rest.
fn split_relative(text: &str) -> Option<(String, String)> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.first() != Some(&"через") { return None; }
    let unit = words.iter().take(5).position(|w| is_time_unit(w))?;
    let time = words[..=unit].join(" ");
    let rest = words[unit + 1..].join(" ");
    (!rest.is_empty()).then_some((reminder_time(&time), rest))
}

fn reminder(text: &str) -> Option<String> {
    let strip_lead = |rest: &str| rest.trim().trim_start_matches("мне ").trim_start_matches("что ").trim_start_matches("о том что ").trim().to_owned();
    // "через сорок минут напомни выключить духовку" is already canonical apart from the time words.
    if let Some((time, rest)) = split_relative(text) {
        for verb in ["напомни", "скажи", "поставь напоминание", "создай напоминание"] {
            if let Some(task) = rest.strip_prefix(verb) {
                let task = strip_lead(task);
                if !task.is_empty() { return Some(format!("{time} напомни {task}")); }
            }
        }
        return None;
    }
    let rest = after(text, &["напомни мне", "напомни", "поставь напоминание", "создай напоминание", "добавь напоминание"])?;
    // "напомни через час позвонить маме"
    if let Some((time, task)) = split_relative(rest) { return Some(format!("{time} напомни {}", strip_lead(&task))); }
    // "напомни позвонить маме через час"
    let words: Vec<&str> = rest.split_whitespace().collect();
    let at = words.iter().rposition(|w| *w == "через")?;
    let time = words[at..].join(" ");
    if !words[at + 1..].iter().any(|w| is_time_unit(w)) || at == 0 { return None; }
    Some(format!("{} напомни {}", reminder_time(&time), strip_lead(&words[..at].join(" "))))
}

const SHOPPING: &[&str] = &["в покупки", "в список покупок", "в корзину", "к покупкам"];
const TASKS: &[&str] = &["в список дел", "в дела", "в задачи", "в список задач", "к делам"];

fn list_item(text: &str) -> Option<String> {
    let verbs = ["добавь", "запиши", "внеси", "закинь", "кинь", "занеси"];
    for (places, canonical) in [(SHOPPING, "в покупки"), (TASKS, "в список дел")] {
        for verb in verbs {
            let Some(rest) = after(text, &[&format!("{verb} ")]) else { continue };
            // "добавь в покупки молоко"
            if let Some(items) = places.iter().find_map(|place| rest.strip_prefix(place)).map(str::trim).filter(|s| !s.is_empty()) {
                return Some(format!("добавь {items} {canonical}"));
            }
            // "добавь молоко в список покупок"
            if let Some(items) = places.iter().find_map(|place| rest.strip_suffix(place)).map(str::trim).filter(|s| !s.is_empty()) {
                return Some(format!("добавь {items} {canonical}"));
            }
        }
    }
    after(text, &["надо купить ", "нужно купить ", "купить ", "купи "]).map(|items| format!("добавь {items} в покупки"))
}

fn list_done(text: &str) -> Option<String> {
    let rest = after(text, &["вычеркни", "удали из списка", "отметь что купил", "отметь что купила"])?;
    for (suffixes, state) in [(&["из покупок", "из списка покупок"][..], "купленным"), (&["из дел", "из списка дел", "из задач"][..], "выполненным")] {
        if let Some(item) = suffixes.iter().find_map(|s| rest.strip_suffix(s)).map(str::trim).filter(|s| !s.is_empty()) {
            return Some(format!("отметь {item} {state}"));
        }
    }
    (!rest.contains(' ') || text.starts_with("отметь что")).then(|| format!("отметь {rest} купленным"))
}

const DOCUMENTS: &[(&str, &str)] = &[
    ("курсов", "курсов"), ("диплом", "диплом"), ("реферат", "реферат"), ("презентац", "презентац"),
    ("отчет", "отчет"), ("отчёт", "отчет"), ("резюме", "резюме"), ("таблиц", "таблиц"), ("договор", "договор"),
];

fn file_search(text: &str) -> Option<String> {
    let rest = after(text, &["найди", "поищи", "отыщи", "где лежит", "где мой", "где моя", "где"])?;
    let yesterday = text.contains("вчера");
    let named = DOCUMENTS.iter().find(|(word, _)| rest.contains(word)).map(|(_, query)| (*query).to_owned());
    let explicit = after(rest, &["мой файл", "мои файлы", "файл", "файлы", "мой документ", "мои документы", "документ", "документы"])
        .map(|query| query.trim_start_matches("с ").trim_start_matches("про ").trim_start_matches("под названием ").trim().to_owned());
    let query = named.or(explicit)?;
    let query = query.replace("который я вчера редактировал", "").replace("вчера", "").trim().to_owned();
    if query.is_empty() { return None; }
    Some(format!("найди документ {query}{}", if yesterday { " который я вчера редактировал" } else { "" }))
}

const ORDINALS: &[(&[&str], &str)] = &[
    (&["перв", "1"], "первой"), (&["втор", "2"], "второй"), (&["трет", "3"], "третьей"),
    (&["четверт", "4"], "четвертой"), (&["пят", "5"], "пятой"),
];

fn news_details(text: &str) -> Option<String> {
    if !text.contains("новост") || !has(text, &["подробн", "расскажи", "раскрой", "что в", "что во", "детали"]) { return None; }
    // "переведи вторую новость" stays a translation request.
    if text.starts_with("переведи") { return None; }
    let ordinal = ORDINALS.iter().find(|(stems, _)| text.split_whitespace().any(|w| stems.iter().any(|s| w.starts_with(s))))?;
    Some(format!("подробнее о {} новости", ordinal.1))
}

/// Words including the short ones `tokens` drops ("что", "про").
fn all_words(text: &str) -> Vec<String> {
    text.to_lowercase().replace('ё', "е").split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_owned).collect()
}

fn ideas_about(text: &str) -> Option<String> {
    // Any form of "идея" with a topic: "покажи идею про помощника", "найди мою идею о музыке".
    // Saving an idea is matched earlier by `idea`, so this only reads.
    if has(text, &["запомни", "запиши", "сохрани", "добавь", "удали"]) { return None; }
    let words = all_words(text);
    let at = words.iter().position(|w| w.starts_with("иде"))?;
    let marker = words.iter().skip(at + 1).position(|w| matches!(w.as_str(), "про" | "о" | "об" | "насчет" | "по"))? + at + 1;
    let topic: Vec<&str> = words[marker + 1..].iter().map(String::as_str).filter(|w| *w != "теме").collect();
    if topic.is_empty() { return None; }
    Some(format!("какие идеи про {}", topic.join(" ")))
}

/// Recognition often breaks "что нового" ("что но о в технология"), but the topic word
/// survives: a short question with a news topic is a news request.
fn news_topic_question(text: &str) -> Option<String> {
    let words = all_words(text);
    if words.len() > 7 || has(text, &["такое", "значит", "это", "расскажи о себе"]) { return None; }
    if !(words.first().is_some_and(|w| w == "что") || has(text, &["нов", "слышно"])) { return None; }
    let topic = TOPICS.iter().any(|(stems, _)| words.iter().any(|w| stems.iter().any(|s| w.starts_with(s))));
    topic.then(|| news_phrase(text))
}

/// Login pages by spoken name. Fixed URLs only: recognized speech is never put into a URL.
pub const LOGIN_SITES: &[(&[&str], &str)] = &[
    (&["github", "гитхаб", "гит хаб", "гитхабе"], "github"),
    (&["gitlab", "гитлаб"], "gitlab"),
    (&["google", "гугл", "gmail", "джимейл", "гмейл"], "google"),
    (&["яндекс", "yandex"], "яндекс"),
    (&["вконтакте", "вк", "контакт"], "вконтакте"),
    (&["telegram", "телеграм", "телеграмм"], "telegram"),
    (&["discord", "дискорд"], "discord"),
    (&["steam", "стим"], "steam"),
    (&["twitch", "твич"], "twitch"),
    (&["youtube", "ютуб", "ютьюб"], "youtube"),
    (&["outlook", "аутлук", "майкрософт", "microsoft"], "microsoft"),
    (&["chatgpt", "чат гпт", "чатгпт", "openai"], "chatgpt"),
    (&["notion", "ноушн", "ноушен"], "notion"),
    (&["figma", "фигма", "фигму", "фигме"], "figma"),
    (&["госуслуги", "госуслуг"], "госуслуги"),
];

fn login(text: &str) -> Option<String> {
    if !has(text, &["вход", "войди", "войти", "зайди в аккаунт", "залогин", "авториз", "логин", "аккаунт"]) { return None; }
    let words: Vec<&str> = text.split_whitespace().collect();
    let site = LOGIN_SITES.iter().find(|(names, _)| names.iter().any(|name| {
        if name.contains(' ') { text.contains(name) } else { words.iter().any(|w| w.starts_with(name)) }
    }))?;
    Some(format!("открой вход в {}", site.1))
}

/// Rewrites a free wording of an everyday request to its canonical phrase.
/// Returns `None` when the text is not clearly one of these requests.
pub fn canonicalize(text: &str) -> Option<String> {
    let text = text.trim().trim_end_matches(['?', '!', '.']).trim();
    if text.is_empty() || super::is_negated(text) { return None; }
    // Chat about the assistant itself is not a request for news or plans.
    if has(text, &["у тебя", "с тобой", "о тебе", "про тебя"]) { return None; }
    // Explicit patterns first: they keep parameters and are the only way to write data.
    let explicit = login(text).or_else(|| idea(text)).or_else(|| reminder(text)).or_else(|| list_done(text))
        .or_else(|| list_item(text)).or_else(|| file_search(text)).or_else(|| news_details(text)).or_else(|| ideas_about(text));
    if let Some(phrase) = explicit.or_else(|| news_topic_question(text)) { return Some(phrase); }
    // Opening, closing and switching off belong to the command packs.
    if ["открой ", "открыть ", "закрой ", "выключи ", "покажи ленту"].iter().any(|verb| text.starts_with(verb)) { return None; }
    closest_intent(text).map(|id| intent_phrase(id, text))
}

#[cfg(test)]
mod tests {
    use super::canonicalize;

    /// Reports every mismatch at once, not just the first.
    fn check(cases: &[(&str, &str)]) {
        let wrong: Vec<String> = cases.iter()
            .filter(|(said, expected)| canonicalize(said).as_deref() != Some(*expected))
            .map(|(said, expected)| format!("«{said}» → {:?}, expected «{expected}»", canonicalize(said)))
            .collect();
        assert!(wrong.is_empty(), "\n{}", wrong.join("\n"));
    }

    fn check_none<'a>(phrases: impl IntoIterator<Item = (String, &'a str)>) {
        let wrong: Vec<String> = phrases.into_iter()
            .filter_map(|(said, origin)| canonicalize(&said).map(|got| format!("{origin}: «{said}» → «{got}»")))
            .collect();
        assert!(wrong.is_empty(), "\n{}", wrong.join("\n"));
    }

    #[test]
    fn paraphrases_reach_the_same_request() {
        check(&[
            ("какие у меня планы на сегодня", "что у меня сегодня"),
            ("утренний брифинг", "что у меня сегодня"),
            ("что меня ждет сегодня", "что у меня сегодня"),
            ("расскажи о моем дне", "что у меня сегодня"),
            ("будет ли дождь вечером", "вечером нужен зонт"),
            ("брать ли зонтик завтра", "завтра нужен зонт"),
            ("нужен ли зонт", "нужен зонт"),
            ("во что одеться завтра", "как одеться завтра"),
            ("что надеть", "как одеться сегодня"),
            ("какие новости в мире технологий сегодня", "что нового в технологиях за сегодня"),
            ("последние новости про космос", "что нового в космосе"),
            ("что нового в нейросетях", "что нового в нейросетях"),
            ("свежие новости", "что нового"),
            ("какие идеи я записывал", "какие идеи"),
            ("что нужно купить", "прочитай список покупок"),
            ("покажи список дел", "прочитай список дел"),
            ("включи рабочий режим", "включи режим работы"),
            ("купить", "прочитай список покупок"),
            ("давай поработаем", "включи режим работы"),
            ("переведи выделенный текст", "переведи это"),
            ("объясни простыми словами", "объясни проще"),
            ("сделай короче", "сократи это"),
            ("перескажи кратко", "сократи это"),
            ("проверь текст на ошибки", "исправь ошибки в тексте"),
            ("проверь орфографию", "исправь ошибки в тексте"),
        ]);
    }

    #[test]
    fn writes_keep_their_parameters() {
        check(&[
            ("запиши мысль добавить управление музыкой", "запомни идею добавить управление музыкой"),
            ("у меня идея: голосовой калькулятор", "запомни идею голосовой калькулятор"),
            ("сохрани идею сделать тёмную тему", "запомни идею сделать тёмную тему"),
            ("напомни выключить духовку через сорок минут", "через сорок минут напомни выключить духовку"),
            ("напомни мне через час позвонить маме", "через один час напомни позвонить маме"),
            ("через полчаса напомни проверить почту", "через тридцать минут напомни проверить почту"),
            ("через десять минут скажи что пора выходить", "через десять минут напомни пора выходить"),
            ("добавь в покупки молоко и хлеб", "добавь молоко и хлеб в покупки"),
            ("запиши молоко в список покупок", "добавь молоко в покупки"),
            ("надо купить яйца и сыр", "добавь яйца и сыр в покупки"),
            ("добавь отправить отчёт в задачи", "добавь отправить отчёт в список дел"),
            ("вычеркни молоко из покупок", "отметь молоко купленным"),
            ("вычеркни отчёт из списка дел", "отметь отчёт выполненным"),
            ("найди мою курсовую которую я вчера редактировал", "найди документ курсов который я вчера редактировал"),
            ("где лежит файл бюджет", "найди документ бюджет"),
            ("найди презентацию", "найди документ презентац"),
            ("расскажи подробнее про вторую новость", "подробнее о второй новости"),
            ("что в первой новости", "подробнее о первой новости"),
            ("покажи идеи про музыку", "какие идеи про музыку"),
        ]);
    }

    #[test]
    fn login_opens_only_known_sites() {
        check(&[
            ("открой вход в гитхаб", "открой вход в github"),
            ("войди в github", "открой вход в github"),
            ("зайди в аккаунт гугл", "открой вход в google"),
            ("авторизуйся в дискорде", "открой вход в discord"),
            ("открой вход в госуслуги", "открой вход в госуслуги"),
        ]);
        assert_eq!(canonicalize("войди в какой-то неизвестный сайт"), None);
        assert_eq!(canonicalize("открой гитхаб"), None, "opening a site is the browser pack's job");
    }

    #[test]
    fn speech_recognition_slips_still_match() {
        check(&[
            ("какие планы на севодня", "что у меня сегодня"),
            ("что но о в технология", "что нового в технологиях"),
            ("покажи идею про голосового помощника", "какие идеи про голосового помощника"),
            ("найди мою идею о музыке", "какие идеи про музыке"),
            ("нужен зондик вечером", "вечером нужен зонт"),
            ("включи рабочий рижим", "включи режим работы"),
        ]);
    }

    #[test]
    fn small_talk_and_other_commands_are_left_alone() {
        check_none([
            "как дела", "привет", "расскажи анекдот", "что нового у тебя", "какая погода", "погода в минске",
            "открой браузер", "сделай скриншот", "сколько времени", "давай поговорим", "не включай режим работы",
            "не добавляй молоко в покупки", "обычный разговор о покупках", "мне нужно подумать", "открой новости",
            "переведи вторую новость", "напомни", "найди", "рабочий режим", "купил хлеб", "выключи компьютер", "громкость на пятьдесят",
            "какой сегодня день", "сегодня хорошая погода", "я записывал видео",
        ].map(|said| (said.to_owned(), "small talk")));
    }

    /// Every phrase of every installed command pack must keep reaching its pack.
    #[test]
    fn command_pack_phrases_are_never_captured() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/commands");
        let mut phrases = Vec::new();
        for pack in std::fs::read_dir(&root).expect("command packs") {
            let pack = pack.unwrap().path();
            let Ok(source) = std::fs::read_to_string(pack.join("command.toml")) else { continue };
            let manifest: toml::Value = toml::from_str(&source).expect("valid manifest");
            let name: &'static str = pack.file_name().unwrap().to_string_lossy().into_owned().leak();
            for command in manifest.get("commands").and_then(|c| c.as_array()).into_iter().flatten() {
                let ru = command.get("phrases").and_then(|p| p.get("ru")).and_then(|p| p.as_array());
                for phrase in ru.into_iter().flatten().filter_map(|p| p.as_str()) {
                    phrases.push((phrase.replace("{city}", "минск"), name));
                }
            }
        }
        assert!(phrases.len() > 50, "only {} pack phrases found", phrases.len());
        check_none(phrases);
    }
}

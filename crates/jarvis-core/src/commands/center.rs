use super::JCommand;

// Only implemented Center actions are advertised. Parameter examples are handled by the voice bridge.
pub fn available_commands() -> Vec<JCommand> {
    let mut commands: Vec<JCommand> = serde_json::from_str(r#"[
  {
    "id": "center_open",
    "type": "center",
    "description": "Открыть личный центр",
    "phrases": {
      "ru": [
        "открой центр"
      ]
    }
  },
  {
    "id": "center_news_open",
    "type": "center",
    "description": "Открыть новости",
    "phrases": {
      "ru": [
        "открой новости",
        "открой ленту"
      ]
    }
  },
  {
    "id": "center_news_refresh",
    "type": "center",
    "description": "Обновить новостную ленту",
    "phrases": {
      "ru": [
        "обнови новости",
        "обнови ленту",
        "обновить ленту"
      ]
    }
  },
  {
    "id": "center_news_translate",
    "type": "center",
    "description": "Перевести новость по номеру",
    "phrases": {
      "ru": [
        "переведи вторую новость",
        "переведи новость"
      ]
    }
  },
  {
    "id": "center_calendar",
    "type": "center",
    "description": "Открыть календарь",
    "phrases": {
      "ru": [
        "открой календарь"
      ]
    }
  },
  {
    "id": "center_schedule",
    "type": "center",
    "description": "Планы на сегодня",
    "phrases": {
      "ru": [
        "расписание на сегодня"
      ]
    }
  },
  {
    "id": "center_notes",
    "type": "center",
    "description": "Открыть заметки",
    "phrases": {
      "ru": [
        "открой заметки"
      ]
    }
  },
  {"id":"center_mindmap_create","type":"center","description":"Создать карту мыслей голосом","phrases":{"ru":["создай карту мыслей","сделай карту мыслей","создай карту идей","создай ментальную карту"]}},
  {
    "id": "center_note_create",
    "type": "center",
    "description": "Создать заметку с диктовкой",
    "phrases": {
      "ru": [
        "создай заметку"
      ]
    }
  },
  {
    "id": "center_note_append",
    "type": "center",
    "description": "Дополнить заметку",
    "phrases": {
      "ru": [
        "дополни заметку"
      ]
    }
  },
  {
    "id": "center_checklist",
    "type": "center",
    "description": "Создать чек-лист",
    "phrases": {
      "ru": [
        "создай чек-лист"
      ]
    }
  },
  {
    "id": "center_voice_text",
    "type": "center",
    "description": "Открыть голос в текст",
    "phrases": {
      "ru": [
        "открой голос в текст"
      ]
    }
  },
  {
    "id": "center_timer_open",
    "type": "center",
    "description": "Открыть таймер",
    "phrases": {
      "ru": [
        "открой таймер"
      ]
    }
  },
  {
    "id": "center_timer_start",
    "type": "center",
    "description": "Запустить таймер: время или пресет",
    "phrases": {
      "ru": [
        "запусти таймер на десять минут",
        "запусти таймер учёба",
        "запусти пресет таймера"
      ]
    }
  },
  {
    "id": "center_timer_pause",
    "type": "center",
    "description": "Приостановить таймер",
    "phrases": {
      "ru": [
        "приостанови таймер"
      ]
    }
  },
  {
    "id": "center_timer_resume",
    "type": "center",
    "description": "Продолжить таймер",
    "phrases": {
      "ru": [
        "продолжи таймер"
      ]
    }
  },
  {
    "id": "center_timer_reset",
    "type": "center",
    "description": "Сбросить таймер",
    "phrases": {
      "ru": [
        "сбрось таймер"
      ]
    }
  },
  {
    "id": "center_reminders",
    "type": "center",
    "description": "Открыть напоминания",
    "phrases": {
      "ru": [
        "открой напоминания"
      ]
    }
  },
  {
    "id": "center_reminder_create",
    "type": "center",
    "description": "Создать напоминание",
    "phrases": {
      "ru": [
        "создай напоминание",
        "добавь напоминание",
        "напомни мне"
      ]
    }
  },
  {
    "id": "center_reminder_edit",
    "type": "center",
    "description": "Изменить текст, время и предварительные предупреждения напоминания",
    "phrases": {
      "ru": ["измени напоминание", "редактируй напоминание", "измени первое напоминание", "перенеси напоминание"]
    }
  },
  {
    "id": "center_birthdays",
    "type": "center",
    "description": "Открыть дни рождения",
    "phrases": {
      "ru": [
        "открой дни рождения"
      ]
    }
  },
  {
    "id": "center_birthday_create",
    "type": "center",
    "description": "Добавить день рождения",
    "phrases": {
      "ru": [
        "добавь день рождения"
      ]
    }
  },
  {
    "id": "center_weather",
    "type": "center",
    "description": "Погода на 7 / 30 / 60 дней",
    "phrases": {
      "ru": [
        "погода на неделю",
        "погода на месяц",
        "погода на два месяца"
      ]
    }
  },
  {
    "id": "center_city",
    "type": "center",
    "description": "Изменить город прогноза",
    "phrases": {
      "ru": [
        "измени город для прогноза"
      ]
    }
  },
  {
    "id": "center_habits",
    "type": "center",
    "description": "Открыть привычки",
    "phrases": {
      "ru": [
        "открой привычки"
      ]
    }
  },
  {
    "id": "center_habit_mark",
    "type": "center",
    "description": "Отметить привычку по номеру",
    "phrases": {
      "ru": [
        "отметь привычку",
        "отметь вторую привычку"
      ]
    }
  },
  {
    "id": "center_training",
    "type": "center",
    "description": "Открыть тренировки",
    "phrases": {
      "ru": [
        "открой тренировки"
      ]
    }
  },
  {
    "id": "center_charge",
    "type": "center",
    "description": "Открыть зарядку",
    "phrases": {
      "ru": [
        "открой зарядку"
      ]
    }
  },
  {
    "id": "center_profile",
    "type": "center",
    "description": "Открыть мои параметры",
    "phrases": {
      "ru": [
        "открой мои параметры"
      ]
    }
  },
  {
    "id": "center_weight",
    "type": "center",
    "description": "Записать измерение веса",
    "phrases": {
      "ru": [
        "запиши вес"
      ]
    }
  },
  {
    "id": "center_stats",
    "type": "center",
    "description": "Открыть статистику тренировок",
    "phrases": {
      "ru": [
        "открой статистику"
      ]
    }
  },
  {
    "id": "center_vault",
    "type": "center",
    "description": "Открыть хранилище паролей",
    "phrases": {
      "ru": [
        "открой хранилище",
        "открой пароли"
      ]
    }
  },
  {
    "id": "center_history",
    "type": "center",
    "description": "Открыть историю запросов",
    "phrases": {
      "ru": [
        "открой историю запросов",
        "открой живые логи"
      ]
    }
  },
  {
    "id": "center_speech_stop",
    "type": "center",
    "description": "Остановить озвучку",
    "phrases": {
      "ru": [
        "останови озвучку"
      ]
    }
  }
  , {"id":"outlook_open","type":"center","description":"Открыть почту Outlook","phrases":{"ru":["открой почту","открой outlook","покажи почту"]}}
  , {"id":"outlook_refresh","type":"center","description":"Обновить последние 50 входящих","phrases":{"ru":["обнови почту","обнови письма","проверь почту"]}}
  , {"id":"outlook_read","type":"center","description":"Открыть письмо по номеру","phrases":{"ru":["открой второе письмо","прочитай первое письмо"]}}
  , {"id":"outlook_compose","type":"center","description":"Продиктовать получателя, тему и текст письма","phrases":{"ru":["напиши письмо","создай письмо","продолжить письмо"]}}
  , {"id":"outlook_send","type":"center","description":"Проверить письмо и запросить подтверждение отправки","phrases":{"ru":["отправь письмо"]}}
  , {"id":"outlook_draft","type":"center","description":"Сохранить письмо в черновиках Outlook","phrases":{"ru":["сохрани письмо в черновик","сохрани черновик"]}}
  , {"id":"outlook_reply","type":"center","description":"Новое письмо отправителю выбранного письма","phrases":{"ru":["ответь на письмо"]}}
]"#).expect("valid built-in Center command catalog");
    for command in &mut commands {
        if let Some(phrases) = command.phrases.get_mut("ru") {
            let originals = phrases.clone();
            for phrase in originals {
                phrases.push(format!("пожалуйста {phrase}"));
                phrases.push(format!("{phrase} пожалуйста"));
                for (verb, aliases) in [
                    ("открой ", vec!["открыть ", "можешь открыть ", "перейди в "]),
                    ("создай ", vec!["создать ", "можешь создать "]),
                    ("обнови ", vec!["обновить ", "можешь обновить "]),
                    ("переведи ", vec!["перевести ", "можешь перевести "]),
                    ("отметь ", vec!["отметить ", "можешь отметить "]),
                ] {
                    if let Some(rest) = phrase.strip_prefix(verb) {
                        phrases.extend(aliases.into_iter().map(|alias| format!("{alias}{rest}")));
                    }
                }
            }
            phrases.sort(); phrases.dedup();
        }
    }
    commands
}
pub fn normalize_phrase(text: &str) -> String {
    let mut text = text.to_lowercase().replace('ё', "е").replace("пожалуйста,", "пожалуйста").split_whitespace().collect::<Vec<_>>().join(" ");
    text = text.trim_matches(|ch: char| ch.is_ascii_punctuation() || ch == '«' || ch == '»').to_string();
    loop {
        let prefix = ["пожалуйста ", "ты можешь ", "можешь ты ", "можешь ", "не мог бы ты ", "давай "]
            .into_iter().find(|prefix| text.starts_with(prefix));
        let Some(prefix) = prefix else { break; };
        // "давай поговорим" is a dialogue action, not filler.
        if prefix == "давай " && ["давай поговорим", "давай пообщаемся", "давай поболтаем"].contains(&text.as_str()) { break; }
        text = text[prefix.len()..].trim().to_owned();
    }
    if let Some(without) = text.strip_suffix(" пожалуйста") { text = without.trim_end_matches(',').to_owned(); }
    for (alias, canonical) in [
        ("переключись на ", "открой "), ("перейди в ", "открой "), ("открыть ", "открой "),
        ("запустить ", "запусти "), ("поставить таймер", "запусти таймер"), ("поставь таймер", "запусти таймер"), ("включи таймер", "запусти таймер"),
        ("начни таймер", "запусти таймер"), ("обновить ", "обнови "), ("освежи ленту", "обнови ленту"),
        ("перевести ", "переведи "), ("добавить ", "добавь "), ("создать ", "создай "),
        ("сделай заметку", "создай заметку"), ("новая заметка", "создай заметку"),
        ("поставь напоминание", "создай напоминание"), ("отметить ", "отметь "),
        ("покажи календарь", "открой календарь"), ("покажи заметки", "открой заметки"),
        ("покажи привычки", "открой привычки"), ("покажи новости", "открой новости"),
    ] {
        if let Some(rest) = text.strip_prefix(alias) {
            if alias.ends_with(' ') || rest.is_empty() || rest.starts_with(' ') { return format!("{canonical}{rest}"); }
        }
    }
    text
}

pub fn matches_phrase(text: &str) -> bool {
    let normalized = normalize_phrase(text);
    available_commands().iter().any(|command|command.get_all_phrases().iter().any(|phrase|phrase.replace('ё',"е")==normalized))
}
#[cfg(test)]
mod tests {
    #[test]
    fn catalog_has_unique_ids_and_refresh_alias() {
        let commands=super::available_commands();
        let ids:std::collections::HashSet<_>=commands.iter().map(|c|&c.id).collect();
        assert_eq!(commands.len(),ids.len());
        assert!(super::matches_phrase("обнови ленту"));assert!(super::matches_phrase("создай карту мыслей"));assert!(super::matches_phrase("создать карту идей"));
        assert!(!super::matches_phrase("открой браузер"));
        assert!(super::matches_phrase("Пожалуйста, открой календарь!"));
    }
    #[test]
    fn natural_variants_preserve_parameters_and_negation() {
        assert_eq!(super::normalize_phrase("можешь поставить таймер"), "запусти таймер");
        assert_eq!(super::normalize_phrase("пожалуйста поставь таймер на десять минут"), "запусти таймер на десять минут");
        assert_eq!(super::normalize_phrase("открыть заметки пожалуйста"), "открой заметки");
        assert_eq!(super::normalize_phrase("не создавай заметку"), "не создавай заметку");
        assert!(super::matches_phrase("можешь открыть календарь"));
    }
}

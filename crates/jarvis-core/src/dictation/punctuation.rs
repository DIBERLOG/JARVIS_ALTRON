//! Voice punctuation: what a person says, turned into what they meant.
//!
//! A dictation arrives as prose with the punctuation words still in it — "привет
//! точка как дела вопросительный знак". This module replaces a spoken mark with
//! the mark itself, locally, before the text is delivered. It never calls a
//! model, and it never changes a word that is not one of the spoken marks.
//!
//! # The rules, and the one honest limitation
//!
//! * a mark is recognized only as a whole word, so "точка" inside "точками"
//!   survives, and a phrase like "в точку" is left alone where the grammar makes
//!   it clear that it is not punctuation;
//! * a mark attaches to the word before it without a space, and a mark that opens
//!   something attaches to the word after it;
//! * a sentence break capitalizes the first letter of the next word, which is
//!   what a person expects to see;
//! * **quoted dictation is not detected.** "Он сказал цитата точка цитата" cannot
//!   be told apart from a real full stop without understanding the sentence, and
//!   pretending to would corrupt text. The limitation is documented, and the
//!   setting that turns this pass off exists for exactly this reason.

/// What one pass changed.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PunctuationReport {
    /// Marks that were written as marks.
    pub marks: usize,
    /// Whether the pass changed anything at all.
    pub changed: bool,
}

/// One spoken mark: the words that mean it, and what it becomes.
struct Mark {
    /// Phrases, longest first, so "новый абзац" is not read as "новый".
    phrases: &'static [&'static str],
    /// The text it becomes. `\n\n` for a paragraph, `\n` for a line.
    text: &'static str,
    /// Whether a space belongs after it that the mark itself does not carry.
    space_after: bool,
    /// Whether it opens something, so it attaches to the word after it.
    opens: bool,
    /// Whether a sentence ends here, so the next word starts with a capital.
    sentence_end: bool,
}

/// Every mark this build understands, in three languages.
///
/// The Russian and Ukrainian words are spelled the way a person says them, and
/// the English ones are the words a dictation in English produces.
const MARKS: [Mark; 13] = [
    Mark {
        phrases: &["новый абзац", "новий абзац", "new paragraph"],
        text: "\n\n",
        space_after: false,
        opens: false,
        sentence_end: true,
    },
    Mark {
        phrases: &["новая строка", "нова строка", "new line"],
        text: "\n",
        space_after: false,
        opens: false,
        sentence_end: true,
    },
    Mark {
        phrases: &["вопросительный знак", "знак питання", "question mark"],
        text: "?",
        space_after: true,
        opens: false,
        sentence_end: true,
    },
    Mark {
        phrases: &["восклицательный знак", "знак оклику", "exclamation mark"],
        text: "!",
        space_after: true,
        opens: false,
        sentence_end: true,
    },
    Mark {
        phrases: &["двоеточие", "двокрапка", "colon"],
        text: ":",
        space_after: true,
        opens: false,
        sentence_end: false,
    },
    Mark {
        phrases: &["точка с запятой", "крапка з комою", "semicolon"],
        text: ";",
        space_after: true,
        opens: false,
        sentence_end: false,
    },
    Mark {
        phrases: &["запятая", "кома", "comma"],
        text: ",",
        space_after: true,
        opens: false,
        sentence_end: false,
    },
    // Quotes are said as a pair: "открой кавычки … закрой кавычки". A bare
    // "кавычки" is deliberately not interpreted, and that limit is documented.
    Mark {
        phrases: &[
            "открой кавычки",
            "відкрий лапки",
            "open quote",
            "open quotation marks",
        ],
        text: "«",
        space_after: false,
        opens: true,
        sentence_end: false,
    },
    Mark {
        phrases: &[
            "закрой кавычки",
            "закрий лапки",
            "close quote",
            "close quotation marks",
        ],
        text: "»",
        space_after: true,
        opens: false,
        sentence_end: false,
    },
    Mark {
        phrases: &[
            "открой скобку",
            "відкрий дужку",
            "open bracket",
            "open parenthesis",
        ],
        text: "(",
        space_after: false,
        opens: true,
        sentence_end: false,
    },
    Mark {
        phrases: &[
            "закрой скобку",
            "закрий дужку",
            "close bracket",
            "close parenthesis",
        ],
        text: ")",
        space_after: true,
        opens: false,
        sentence_end: false,
    },
    Mark {
        phrases: &["точка", "крапка", "full stop", "period"],
        text: ".",
        space_after: true,
        opens: false,
        sentence_end: true,
    },
    Mark {
        phrases: &["тире", "дефіс", "dash"],
        text: " — ",
        space_after: false,
        opens: false,
        sentence_end: false,
    },
];

/// The words that take a mark back: "точка" after them is a word, not a mark.
///
/// This is what keeps "в точку", "до точки", "на точку" and their like intact.
const KEEPERS: [&str; 12] = [
    "в", "во", "до", "на", "за", "про", "у", "к", "ко", "о", "об", "по",
];

/// Whether a spoken word starts a phrase, ignoring case and surrounding marks.
fn clean_word(word: &str) -> String {
    word.trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase()
}

/// Applies voice punctuation to a recognized phrase.
pub fn apply_voice_punctuation(text: &str) -> (String, PunctuationReport) {
    let mut report = PunctuationReport::default();
    // The tokens keep the whitespace structure of the input, so a dictation that
    // already has line breaks keeps them.
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let mut output = String::with_capacity(text.len());
    let mut index = 0usize;
    // Whether the next word has to start with a capital letter.
    let mut capitalize_next = true;

    while index < tokens.len() {
        // The phrase table is matched below; a single word is not read here.
        // A phrase is looked for before a single word: "новый абзац" must not be
        // read as "новый".
        let mut matched: Option<(&Mark, usize)> = None;
        for mark in MARKS.iter() {
            for phrase in mark.phrases {
                let parts: Vec<&str> = phrase.split(' ').collect();
                if index + parts.len() > tokens.len() {
                    continue;
                }
                let candidate: Vec<String> = tokens[index..index + parts.len()]
                    .iter()
                    .map(|token| clean_word(token))
                    .collect();
                if candidate == parts {
                    matched = Some((mark, parts.len()));
                    break;
                }
            }
            if matched.is_some() {
                break;
            }
        }

        match matched {
            Some((mark, length)) => {
                // A phrase that follows a preposition is a word, not a mark.
                let previous = output
                    .split_whitespace()
                    .last()
                    .map(clean_word)
                    .unwrap_or_default();
                if mark.sentence_end && KEEPERS.contains(&previous.as_str()) {
                    push_word(&mut output, tokens[index], &mut capitalize_next);
                } else if mark.opens {
                    // An opening bracket stands apart from the word before it and
                    // attaches to the word after it: "список (один)".
                    if !output.is_empty() && !output.ends_with(' ') && !output.ends_with('\n') {
                        output.push(' ');
                    }
                    output.push_str(mark.text);
                    capitalize_next = false;
                    report.marks += 1;
                    report.changed = true;
                } else {
                    // Every other mark attaches to what came before it: "привет,"
                    // and not "привет ,".
                    while output.ends_with(' ') {
                        output.pop();
                    }
                    output.push_str(mark.text);
                    if mark.space_after && index + length < tokens.len() {
                        output.push(' ');
                    }
                    capitalize_next = mark.sentence_end;
                    report.marks += 1;
                    report.changed = true;
                }
                index += length;
            }
            None => {
                push_word(&mut output, tokens[index], &mut capitalize_next);
                index += 1;
            }
        }
    }

    (output.trim_end().to_string(), report)
}

fn push_word(output: &mut String, word: &str, capitalize_next: &mut bool) {
    // A word after an opening bracket or an opening quote attaches to it, and a
    // word after anything else is separated by a space.
    let after_an_opening = matches!(output.chars().last(), Some('(') | Some('«'));
    if !output.is_empty() && !output.ends_with('\n') && !output.ends_with(' ') && !after_an_opening
    {
        output.push(' ');
    }
    if *capitalize_next {
        let mut characters = word.chars();
        if let Some(first) = characters.next() {
            output.extend(first.to_uppercase());
            output.push_str(characters.as_str());
        }
        *capitalize_next = false;
    } else {
        output.push_str(word);
    }
    // A word that already ends a sentence keeps the rule for the next one.
    if word.ends_with('.') || word.ends_with('!') || word.ends_with('?') {
        *capitalize_next = true;
    }
}

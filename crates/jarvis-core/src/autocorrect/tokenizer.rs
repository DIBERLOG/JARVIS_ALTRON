//! Tokenizer: which parts of a text are words, and which are not.
//!
//! The tokenizer decides what the checker is allowed to look at. It is deliberately
//! conservative: anything that is not clearly a word in one of the two supported
//! languages is **skipped**, because a false report on a path, a hash, or an
//! identifier is worse than a missed typo.
//!
//! Skipped by default:
//!
//! * URLs, e-mail addresses, and Windows paths;
//! * UUIDs, hex digests, and long random-looking tokens;
//! * numbers, version strings, times, and dates;
//! * fenced code blocks, inline code spans, and Markdown link targets;
//! * short all-caps acronyms such as `API` or `JSON`;
//! * tokens longer than [`MAX_TOKEN_CHARS`], and tokens that are mostly symbols.
//!
//! Checked:
//!
//! * words in the Cyrillic and Latin scripts, including a hyphen inside a word
//!   (`что-то`, `well-known`) and an apostrophe (`don't`), which are split and each
//!   part checked on its own;
//! * `snake_case` and `camelCase` identifiers, split into their sub-words;
//! * all ranges are in Unicode scalar values, and every one of them is produced from
//!   a single pass over `char_indices`, so an emoji or a combining mark cannot shift
//!   a range.

use super::model::{is_double_capital, Language, TextRange, MAX_TOKEN_CHARS};

/// Why a run of characters was not checked.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SkipReason {
    Url,
    Email,
    Path,
    Uuid,
    Hash,
    Number,
    Code,
    LinkTarget,
    Acronym,
    TooLong,
    MostlySymbols,
}

impl SkipReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Url => "url",
            Self::Email => "email",
            Self::Path => "path",
            Self::Uuid => "uuid",
            Self::Hash => "hash",
            Self::Number => "number",
            Self::Code => "code",
            Self::LinkTarget => "link_target",
            Self::Acronym => "acronym",
            Self::TooLong => "too_long",
            Self::MostlySymbols => "mostly_symbols",
        }
    }
}

/// One word the checker should look at.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WordToken {
    pub text: String,
    pub range: TextRange,
    /// Script-based language guess, `None` for a word with no letters.
    pub language: Option<Language>,
}

/// One run the checker ignores, kept for tests and diagnostics.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkippedToken {
    pub text: String,
    pub range: TextRange,
    pub reason: SkipReason,
}

/// Result of one tokenization pass.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ScanResult {
    pub words: Vec<WordToken>,
    pub skipped: Vec<SkippedToken>,
    /// Ranges that were masked before scanning (code blocks, inline code, link targets).
    pub masked: Vec<TextRange>,
}

impl ScanResult {
    pub fn skipped_count(&self) -> usize {
        self.skipped.len()
    }
}

/// Tokenizes a text with the default limits.
pub fn scan(text: &str) -> ScanResult {
    scan_with_limit(text, MAX_TOKEN_CHARS)
}

/// Tokenizes a text, refusing to look at tokens longer than `max_token_chars`.
pub fn scan_with_limit(text: &str, max_token_chars: usize) -> ScanResult {
    let masked = mask_ranges(text);
    let mut result = ScanResult {
        words: Vec::new(),
        skipped: Vec::new(),
        masked: masked.clone(),
    };

    for (text_run, run) in runs(text) {
        if masked.iter().any(|range| range.overlaps(&run)) {
            result.skipped.push(SkippedToken {
                text: text_run.to_string(),
                range: run,
                reason: SkipReason::Code,
            });
            continue;
        }
        // The whole run is classified first: a URL, an e-mail address, a path, a UUID,
        // a digest, or a number is one thing, and splitting it at its `-` or `.` would
        // turn it into pieces the checker would then report as words.
        if let Some(reason) = classify(text_run, max_token_chars) {
            result.skipped.push(SkippedToken {
                text: text_run.to_string(),
                range: run,
                reason,
            });
            continue;
        }
        // A word written with two capitals is a typo rather than an identifier, so it
        // is kept whole: splitting `HEllo` into `H` and `Ello` would hide the mistake.
        if is_double_capital(text_run) {
            result.words.push(WordToken {
                text: text_run.to_string(),
                range: run,
                language: Language::of_word(text_run),
            });
            continue;
        }
        // What is left is split at identifier boundaries, so `snake_case` and
        // `camelCase` names are checked one sub-word at a time.
        for piece in split_identifiers(text_run, run) {
            let word = piece.text(text);
            if word.is_empty() {
                continue;
            }
            match classify(word, max_token_chars) {
                Some(reason) => result.skipped.push(SkippedToken {
                    text: word.to_string(),
                    range: piece,
                    reason,
                }),
                None => result.words.push(WordToken {
                    text: word.to_string(),
                    range: piece,
                    language: Language::of_word(word),
                }),
            }
        }
    }
    result
}

/// A run of characters that can belong to one candidate token.
fn runs(text: &str) -> Vec<(&str, TextRange)> {
    let mut runs = Vec::new();
    // (first char index, first byte offset) of the run being collected.
    let mut start: Option<(usize, usize)> = None;
    let mut index = 0usize;
    for (offset, character) in text.char_indices() {
        let allowed = is_token_character(character);
        match (allowed, start) {
            (true, None) => start = Some((index, offset)),
            (false, Some((begin, begin_offset))) => {
                runs.push((&text[begin_offset..offset], TextRange::new(begin, index)));
                start = None;
            }
            _ => {}
        }
        index += 1;
    }
    if let Some((begin, begin_offset)) = start {
        runs.push((&text[begin_offset..], TextRange::new(begin, index)));
    }
    runs
}

/// Characters that may appear inside a candidate token.
///
/// The set is intentionally narrow: a token that contains `:` or `/` is a URL or a
/// path, and those are recognised by the classifier rather than by this function.
/// Combining marks are included so a letter written with one (`e` + U+0301) stays a
/// single token and its range cannot drift.
fn is_token_character(character: char) -> bool {
    character.is_alphanumeric()
        || is_combining_mark(character)
        || matches!(
            character,
            '_' | '-' | '\'' | '\u{2019}' | '.' | '@' | '/' | '\\' | ':' | '#' | '$' | '%' | '+'
        )
}

/// Whether a character is a combining mark.
///
/// The four Unicode blocks that carry them are listed explicitly: `char` has no stable
/// "is combining" predicate in the standard library, and a wrong guess here would split
/// a grapheme in two.
fn is_combining_mark(character: char) -> bool {
    matches!(character as u32,
        0x0300..=0x036F   // Combining Diacritical Marks
        | 0x1AB0..=0x1AFF // Combining Diacritical Marks Extended
        | 0x20D0..=0x20FF // Combining Diacritical Marks for Symbols
        | 0xFE20..=0xFE2F // Combining Half Marks
    )
}

/// Whether a candidate run is something the checker must leave alone.
fn classify(word: &str, max_token_chars: usize) -> Option<SkipReason> {
    let letters = word
        .chars()
        .filter(|character| character.is_alphabetic())
        .count();
    // Anything without a letter is a number, a time, a date, or a version.
    if letters == 0 {
        return Some(SkipReason::Number);
    }
    if word.contains("://") || word.starts_with("www.") {
        return Some(SkipReason::Url);
    }
    if looks_like_email(word) {
        return Some(SkipReason::Email);
    }
    if looks_like_path(word) {
        return Some(SkipReason::Path);
    }
    if looks_like_uuid(word) {
        return Some(SkipReason::Uuid);
    }
    if looks_like_hash(word) {
        return Some(SkipReason::Hash);
    }
    if looks_like_link_target(word) {
        return Some(SkipReason::LinkTarget);
    }
    if looks_like_code(word) {
        return Some(SkipReason::Code);
    }
    if word.chars().count() > max_token_chars {
        return Some(SkipReason::TooLong);
    }
    let total = word.chars().count();
    if letters * 2 < total {
        return Some(SkipReason::MostlySymbols);
    }
    if is_acronym(word) {
        return Some(SkipReason::Acronym);
    }
    None
}

fn looks_like_email(word: &str) -> bool {
    match word.split_once('@') {
        Some((local, domain)) => {
            !local.is_empty() && domain.contains('.') && !domain.starts_with('.')
        }
        None => false,
    }
}

fn looks_like_path(word: &str) -> bool {
    if word.contains('\\') {
        return true;
    }
    // An absolute POSIX path, or one written from the home directory.
    if word.starts_with('/') || word.starts_with("~/") {
        return true;
    }
    let mut characters = word.chars();
    let first = characters.next();
    let second = characters.next();
    if let (Some(drive), Some(':')) = (first, second) {
        if drive.is_ascii_alphabetic() {
            return true;
        }
    }
    // A slash with something on both sides is a path or a fraction, not a word.
    match word.split_once('/') {
        Some((left, right)) => !left.is_empty() && !right.is_empty(),
        None => false,
    }
}

fn looks_like_uuid(word: &str) -> bool {
    let groups: Vec<&str> = word.split('-').collect();
    if groups.len() != 5 {
        return false;
    }
    let lengths = [8usize, 4, 4, 4, 12];
    groups.iter().zip(lengths).all(|(group, length)| {
        group.len() == length && group.chars().all(|character| character.is_ascii_hexdigit())
    })
}

fn looks_like_hash(word: &str) -> bool {
    if word.len() >= 32 && word.chars().all(|character| character.is_ascii_hexdigit()) {
        return true;
    }
    // A long mixed-case, digit-bearing run without separators is a token, not a word.
    word.len() >= 40
        && word
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
        && word.chars().any(|character| character.is_ascii_digit())
        && word.chars().any(|character| character.is_ascii_uppercase())
        && word.chars().any(|character| character.is_ascii_lowercase())
}

fn looks_like_code(word: &str) -> bool {
    word.contains(['$', '%', '#', '+', '='])
        || word.starts_with('#')
        || (word.contains('(') || word.contains(')'))
        || word.chars().filter(|character| *character == '.').count() > 1
        || (word.contains(':') && !word.contains("://"))
}

fn looks_like_link_target(word: &str) -> bool {
    // Markdown targets are handled by `mask_ranges`; this covers what is left, such as
    // a bare `something.md` file name.
    const EXTENSIONS: [&str; 12] = [
        ".md", ".rs", ".toml", ".json", ".txt", ".exe", ".dll", ".gguf", ".png", ".jpg", ".svg",
        ".html",
    ];
    EXTENSIONS.iter().any(|extension| word.ends_with(extension))
}

/// Whether a token is an abbreviation rather than a word.
///
/// Any run of two or more letters written entirely in capitals is left alone:
/// `API`, `JSON`, `JARVIS`, and `ALTRON` are all names, and a dictionary that does not
/// contain them would otherwise turn every one of them into a reported typo. A
/// single capital letter is still checked, and a mixed word such as `HEllo` is not an
/// abbreviation — it is the mistake the checker is looking for.
fn is_acronym(word: &str) -> bool {
    let letters: Vec<char> = word
        .chars()
        .filter(|character| character.is_alphabetic())
        .collect();
    letters.len() >= 2 && letters.iter().all(|character| character.is_uppercase())
}

/// Splits a run into identifier sub-words, keeping each sub-word's range.
///
/// `snake_case` splits at underscores and digits; `camelCase` and `PascalCase` split
/// at the case boundary, so `getUserName` becomes `get`, `User`, `Name`, and each part
/// is checked against the dictionary on its own.
fn split_identifiers(word: &str, range: TextRange) -> Vec<TextRange> {
    let mut parts = Vec::new();
    let mut local_start = 0usize;
    let characters: Vec<char> = word.chars().collect();
    let mut index = 0usize;
    while index < characters.len() {
        let current = characters[index];
        let boundary = if index == 0 {
            false
        } else {
            let previous = characters[index - 1];
            matches!(current, '_' | '-' | '\'' | '\u{2019}')
                || matches!(previous, '_' | '-' | '\'' | '\u{2019}')
                || (current.is_uppercase() && previous.is_lowercase())
                || (current.is_uppercase()
                    && previous.is_uppercase()
                    && characters
                        .get(index + 1)
                        .map(|next| next.is_lowercase())
                        .unwrap_or(false))
        };
        if boundary {
            push_part(&mut parts, range, local_start, index);
            local_start = index;
        }
        index += 1;
    }
    push_part(&mut parts, range, local_start, characters.len());
    parts
}

fn push_part(parts: &mut Vec<TextRange>, range: TextRange, local_start: usize, local_end: usize) {
    let start = range.start + local_start;
    let end = range.start + local_end;
    if end > start {
        parts.push(TextRange::new(start, end));
    }
}

impl TextRange {
    /// The text at this range, which the tokenizer knows is valid.
    fn text<'a>(&self, text: &'a str) -> &'a str {
        self.slice(text).unwrap_or("")
    }
}

/// Ranges that must not be checked: fenced code blocks, inline code, and link targets.
fn mask_ranges(text: &str) -> Vec<TextRange> {
    let mut ranges: Vec<TextRange> = Vec::new();
    let characters: Vec<char> = text.chars().collect();
    let mut index = 0usize;
    let mut fence_start: Option<usize> = None;

    while index < characters.len() {
        // A fence of three backticks or three tildes starts or ends a code block.
        let triple = index + 2 < characters.len()
            && (characters[index] == '`'
                && characters[index + 1] == '`'
                && characters[index + 2] == '`'
                || characters[index] == '~'
                    && characters[index + 1] == '~'
                    && characters[index + 2] == '~');
        if triple {
            match fence_start {
                Some(start) => {
                    ranges.push(TextRange::new(start, index + 3));
                    fence_start = None;
                }
                None => fence_start = Some(index),
            }
            index += 3;
            continue;
        }
        if fence_start.is_none() {
            if characters[index] == '`' {
                // Inline code: up to the next backtick on any line.
                if let Some(end) = characters[index + 1..]
                    .iter()
                    .position(|character| *character == '`')
                {
                    ranges.push(TextRange::new(index, index + 1 + end + 1));
                    index = index + 1 + end + 1;
                    continue;
                }
            }
            // Markdown link target: `](target)` and image targets.
            if characters[index] == ']'
                && index + 1 < characters.len()
                && characters[index + 1] == '('
            {
                if let Some(end) = characters[index + 1..]
                    .iter()
                    .position(|character| *character == ')')
                {
                    ranges.push(TextRange::new(index + 1, index + 1 + end + 1));
                    index = index + 1 + end + 1;
                    continue;
                }
            }
        }
        index += 1;
    }
    // An unterminated fence masks the rest of the text.
    if let Some(start) = fence_start {
        ranges.push(TextRange::new(start, characters.len()));
    }
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &str) -> Vec<String> {
        scan(text)
            .words
            .into_iter()
            .map(|token| token.text)
            .collect()
    }

    fn skip_reason(text: &str, needle: &str) -> Option<SkipReason> {
        scan(text)
            .skipped
            .into_iter()
            .find(|token| token.text == needle)
            .map(|token| token.reason)
    }

    #[test]
    fn russian_and_english_words_are_collected() {
        assert_eq!(words("Привет, мир!"), vec!["Привет", "мир"]);
        assert_eq!(words("Hello, world!"), vec!["Hello", "world"]);
        // An all-caps name is an abbreviation, not a word to check.
        assert_eq!(
            words("Привет, JARVIS! Как дела?"),
            vec!["Привет", "Как", "дела"]
        );
    }

    #[test]
    fn the_script_of_each_word_decides_its_language() {
        let scan = scan("Привет hello ёж");
        let languages: Vec<Option<Language>> =
            scan.words.iter().map(|token| token.language).collect();
        assert_eq!(
            languages,
            vec![
                Some(Language::Russian),
                Some(Language::English),
                Some(Language::Russian)
            ]
        );
    }

    #[test]
    fn yo_and_ye_are_both_kept_as_words() {
        assert_eq!(words("ёж и еж"), vec!["ёж", "и", "еж"]);
        assert_eq!(words("Ёж и Еж"), vec!["Ёж", "и", "Еж"]);
        // Written in full capitals it is treated as an abbreviation and left alone.
        assert_eq!(words("ЁЖ"), Vec::<String>::new());
    }

    #[test]
    fn hyphens_and_apostrophes_split_words_into_parts() {
        let tokens = scan("что-то well-known don't").words;
        let collected: Vec<&str> = tokens.iter().map(|token| token.text.as_str()).collect();
        assert_eq!(collected, vec!["что", "то", "well", "known", "don", "t"]);
        // The parts keep real ranges, so a suggestion can replace one half.
        let first = &tokens[0];
        assert_eq!(first.range, TextRange::new(0, 3));
        assert_eq!(first.range.slice("что-то well-known don't").unwrap(), "что");
    }

    #[test]
    fn identifiers_are_split_at_case_and_separator_boundaries() {
        let tokens = scan("getUserName snake_case_value HTTPServer").words;
        let collected: Vec<&str> = tokens.iter().map(|token| token.text.as_str()).collect();
        // `HTTP` is dropped as an abbreviation; `Server` is still checked.
        assert_eq!(
            collected,
            vec!["get", "User", "Name", "snake", "case", "value", "Server"]
        );
    }

    #[test]
    fn urls_emails_and_paths_are_skipped() {
        assert_eq!(
            skip_reason(
                "see https://example.com/page today",
                "https://example.com/page"
            ),
            Some(SkipReason::Url)
        );
        assert_eq!(
            skip_reason("write to user@example.com now", "user@example.com"),
            Some(SkipReason::Email)
        );
        assert_eq!(
            skip_reason(
                "open C:\\Users\\angel\\file.txt please",
                "C:\\Users\\angel\\file.txt"
            ),
            Some(SkipReason::Path)
        );
        assert_eq!(
            skip_reason("read /home/user/notes.md here", "/home/user/notes.md"),
            Some(SkipReason::Path)
        );
    }

    #[test]
    fn uuids_hashes_numbers_and_times_are_skipped() {
        let uuid = "550e8400-e29b-41d4-a716-446655440000";
        assert_eq!(
            skip_reason(&format!("id {uuid} here"), uuid),
            Some(SkipReason::Uuid)
        );

        let digest = "ae3f1c9d5b7e2a4f6c8d0b1a3e5f7c9dae3f1c9d5b7e2a4f6c8d0b1a3e5f7c9d";
        assert_eq!(
            skip_reason(&format!("hash {digest} here"), digest),
            Some(SkipReason::Hash)
        );

        assert_eq!(
            skip_reason("at 12:30 on 2026-09-19", "12:30"),
            Some(SkipReason::Number)
        );
        assert_eq!(skip_reason("value 42 here", "42"), Some(SkipReason::Number));
        assert_eq!(
            skip_reason("version 1.2.3 here", "1.2.3"),
            Some(SkipReason::Number)
        );
        // A comma is not a character a word can contain, so a grouped number is two
        // tokens and both of them are skipped as numbers.
        assert_eq!(
            skip_reason("price 1,000.50 here", "1"),
            Some(SkipReason::Number)
        );
        assert_eq!(
            skip_reason("price 1,000.50 here", "000.50"),
            Some(SkipReason::Number)
        );
        assert_eq!(
            skip_reason("date 2026-09-19 here", "2026-09-19"),
            Some(SkipReason::Number)
        );
    }

    #[test]
    fn acronyms_and_long_tokens_are_skipped() {
        assert_eq!(
            skip_reason("use the API now", "API"),
            Some(SkipReason::Acronym)
        );
        assert_eq!(
            skip_reason("in the JSON file", "JSON"),
            Some(SkipReason::Acronym)
        );
        // Longer abbreviations are skipped as well, so product names survive.
        assert_eq!(
            skip_reason("ask JARVIS now", "JARVIS"),
            Some(SkipReason::Acronym)
        );
        assert_eq!(
            skip_reason("ask ALTRON now", "ALTRON"),
            Some(SkipReason::Acronym)
        );
        // A mixed-capitalisation word is the mistake the checker looks for.
        assert_eq!(skip_reason("write HEllo now", "HEllo"), None);
        // A long word is dropped, and a long all-hex run counts as a digest first.
        let long = "ъ".repeat(MAX_TOKEN_CHARS + 1);
        assert_eq!(
            skip_reason(&format!("x {long} y"), &long),
            Some(SkipReason::TooLong)
        );
        let hex_run = "a".repeat(MAX_TOKEN_CHARS + 1);
        assert_eq!(
            skip_reason(&format!("x {hex_run} y"), &hex_run),
            Some(SkipReason::Hash)
        );
    }

    #[test]
    fn code_blocks_inline_code_and_link_targets_are_masked() {
        let text = "Text here.\n```rust\nlet speling = 1;\n```\nMore `inline_code` text and [a link](https://example.com/x).";
        let scan = scan(text);
        let checked: Vec<&str> = scan.words.iter().map(|token| token.text.as_str()).collect();
        assert!(checked.contains(&"Text"));
        assert!(checked.contains(&"More"));
        assert!(checked.contains(&"text"));
        assert!(checked.contains(&"a"));
        assert!(checked.contains(&"link"));
        // Nothing from the code block, the inline span, or the link target.
        assert!(!checked.contains(&"rust"));
        assert!(!checked.contains(&"speling"));
        assert!(!checked.contains(&"inline_code"));
        assert!(!checked.contains(&"example"));
        assert!(!scan.masked.is_empty());
    }

    #[test]
    fn an_unterminated_code_block_masks_the_rest() {
        let scan = scan("Visible text\n```\nhidden words here");
        let checked: Vec<&str> = scan.words.iter().map(|token| token.text.as_str()).collect();
        assert!(checked.contains(&"Visible"));
        assert!(checked.contains(&"text"));
        assert!(!checked.contains(&"hidden"));
    }

    #[test]
    fn ranges_are_exact_with_emoji_and_combining_marks() {
        let text = "😀 Привт 👋 мир";
        let scan_result = scan(text);
        let first = &scan_result.words[0];
        assert_eq!(first.text, "Привт");
        assert_eq!(first.range, TextRange::new(2, 7));
        assert_eq!(first.range.slice(text).unwrap(), "Привт");
        // The interface gets UTF-16 offsets: the emoji counts as two units.
        let utf16 = first.range.to_utf16(text).unwrap();
        assert_eq!(utf16.start, 3);
        assert_eq!(utf16.end, 8);

        let combined = "e\u{0301} Привет";
        let scan_result = scan(combined);
        // The combining mark stays with its letter, so neither token is misaligned.
        assert_eq!(scan_result.words.len(), 2);
        assert_eq!(scan_result.words[0].text, "e\u{0301}");
        assert_eq!(scan_result.words[0].range, TextRange::new(0, 2));
        assert_eq!(scan_result.words[1].text, "Привет");
        assert_eq!(
            scan_result.words[1].range.slice(combined).unwrap(),
            "Привет"
        );
        assert_eq!(scan_result.words[1].range, TextRange::new(3, 9));
    }

    #[test]
    fn punctuation_and_empty_text_produce_nothing() {
        assert!(scan("").words.is_empty());
        assert!(scan("   ").words.is_empty());
        assert!(scan("!!! ... ???").words.is_empty());
        assert!(scan("— – «»").words.is_empty());
    }

    #[test]
    fn a_mixed_text_keeps_every_word_in_order() {
        let text = "Открой файл report.md и напиши в API chat, please";
        let collected = words(text);
        // A file name and an abbreviation are skipped; the words around them stay.
        assert_eq!(
            collected,
            vec!["Открой", "файл", "и", "напиши", "в", "chat", "please"]
        );
    }

    #[test]
    fn a_whole_run_is_classified_before_it_is_split() {
        // A UUID, a POSIX path, and a version string each contain separators that the
        // identifier splitter would otherwise turn into words of their own.
        let uuid = "550e8400-e29b-41d4-a716-446655440000";
        let scan_result = scan(&format!("id {uuid} at /home/user/notes.txt v1.2.3"));
        let checked: Vec<&str> = scan_result
            .words
            .iter()
            .map(|token| token.text.as_str())
            .collect();
        // The version string is skipped as code, because a run with two dots in it is a
        // version rather than a word.
        assert_eq!(checked, vec!["id", "at"]);
        assert_eq!(
            scan_result
                .skipped
                .iter()
                .find(|token| token.text == uuid)
                .map(|token| token.reason),
            Some(SkipReason::Uuid)
        );
        assert_eq!(
            scan_result
                .skipped
                .iter()
                .find(|token| token.text == "/home/user/notes.txt")
                .map(|token| token.reason),
            Some(SkipReason::Path)
        );
    }

    #[test]
    fn a_word_with_two_capitals_is_kept_whole() {
        let scan_result = scan("HEllo ПРивет HTTPServer");
        let checked: Vec<&str> = scan_result
            .words
            .iter()
            .map(|token| token.text.as_str())
            .collect();
        assert_eq!(checked, vec!["HEllo", "ПРивет", "Server"]);
        // `HTTP` is still an abbreviation, not a word.
        assert_eq!(
            scan_result
                .skipped
                .iter()
                .find(|token| token.text == "HTTP")
                .map(|token| token.reason),
            Some(SkipReason::Acronym)
        );
    }

    #[test]
    fn every_range_can_be_sliced_back_out_of_the_text() {
        let text = "Привет, world! snake_case and camelCase, 42 items";
        let scan = scan(text);
        assert!(!scan.words.is_empty());
        for token in &scan.words {
            assert_eq!(
                token.range.slice(text).unwrap(),
                token.text,
                "range must point at the token text"
            );
            let utf16 = token.range.to_utf16(text).unwrap();
            assert!(utf16.end > utf16.start);
        }
        for token in &scan.skipped {
            assert_eq!(token.range.slice(text).unwrap(), token.text);
        }
        // Ranges never overlap each other.
        let mut ranges: Vec<TextRange> = scan.words.iter().map(|token| token.range).collect();
        ranges.sort_by_key(|range| range.start);
        for pair in ranges.windows(2) {
            assert!(!pair[0].overlaps(&pair[1]), "{pair:?}");
        }
    }
}

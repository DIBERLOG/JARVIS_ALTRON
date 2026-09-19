//! The secret filter: a heuristic that keeps credentials out of long-term memory.
//!
//! It runs on text that is about to be stored as a fact, on model suggestions, on
//! generated summaries, and on imported data. When it recognizes something, the
//! automatic path is blocked and the user is asked; a manual save can still be
//! confirmed, because the user owns their data.
//!
//! Three properties are deliberate:
//!
//! * **the matched text never leaves this module.** A [`SecretFinding`] carries the
//!   kind, the byte span, and the line, and nothing else, so no secret can reach a
//!   log, a dialog, an error message, or the model;
//! * **the filter is never asked to classify a finding.** The model is not used to
//!   decide whether something is a secret; there is no path from here to a
//!   provider;
//! * **it is honest about being a heuristic.** A hash, a long identifier, or a
//!   base64 blob of ordinary text can be a false positive, and a secret written in
//!   an unusual way is a false negative. See `docs/AI_MEMORY.md`.

use std::fmt;
use std::ops::Range;

use super::error::SecretKind;

/// Shortest token considered "long and random-looking".
const HIGH_ENTROPY_MIN_LEN: usize = 32;
/// Entropy floor in bits per character.
const HIGH_ENTROPY_MIN_BITS: f64 = 3.5;
/// Entropy floor for hex-only tokens, whose alphabet is smaller by design.
const HIGH_ENTROPY_HEX_BITS: f64 = 3.0;
/// Shortest right-hand side that counts as an assigned secret.
const ASSIGNMENT_MIN_VALUE: usize = 4;

/// One place where something secret-shaped was recognized.
///
/// It never contains the matched text: only where it is and what it looks like.
#[derive(Clone, Eq, PartialEq)]
pub struct SecretFinding {
    pub kind: SecretKind,
    /// Byte range in the scanned text. Byte offsets never leak content.
    pub span: Range<usize>,
    /// One-based line number, so the interface can point at it.
    pub line: usize,
    /// Which rule fired, for diagnostics that stay content-free.
    pub rule: &'static str,
}

impl fmt::Debug for SecretFinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecretFinding")
            .field("kind", &self.kind)
            .field("line", &self.line)
            .field("len", &(self.span.end - self.span.start))
            .field("rule", &self.rule)
            .finish()
    }
}

/// Everything one scan found.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SecretScan {
    findings: Vec<SecretFinding>,
}

impl SecretScan {
    pub fn is_clean(&self) -> bool {
        self.findings.is_empty()
    }

    pub fn findings(&self) -> &[SecretFinding] {
        &self.findings
    }

    pub fn len(&self) -> usize {
        self.findings.len()
    }

    pub fn is_empty(&self) -> bool {
        self.findings.is_empty()
    }

    /// Distinct kinds, in stable order, for a content-free message.
    pub fn kinds(&self) -> Vec<SecretKind> {
        let mut kinds: Vec<SecretKind> = self.findings.iter().map(|found| found.kind).collect();
        kinds.sort_unstable();
        kinds.dedup();
        kinds
    }

    /// Distinct rule names, for diagnostics.
    pub fn rules(&self) -> Vec<&'static str> {
        let mut rules: Vec<&'static str> = self.findings.iter().map(|found| found.rule).collect();
        rules.sort_unstable();
        rules.dedup();
        rules
    }

    fn push(&mut self, kind: SecretKind, span: Range<usize>, text: &str, rule: &'static str) {
        if span.start >= span.end {
            return;
        }
        self.findings.push(SecretFinding {
            kind,
            line: line_of(text, span.start),
            span,
            rule,
        });
    }
}

/// Scans one text.
pub fn scan(text: &str) -> SecretScan {
    let mut found = SecretScan::default();
    scan_private_keys(text, &mut found);
    scan_assigned_secrets(text, &mut found);
    scan_tokens(text, &mut found);
    scan_recovery_codes(text, &mut found);
    scan_payment_cards(text, &mut found);
    found.findings.sort_by(|left, right| {
        left.span
            .start
            .cmp(&right.span.start)
            .then_with(|| left.kind.cmp(&right.kind))
    });
    found
}

/// Scans several texts as one, for example the fields of an import.
pub fn scan_all<'a>(texts: impl IntoIterator<Item = &'a str>) -> SecretScan {
    let mut combined = SecretScan::default();
    for text in texts {
        let one = scan(text);
        combined.findings.extend(one.findings);
    }
    combined.findings.sort_by(|left, right| {
        left.span
            .start
            .cmp(&right.span.start)
            .then_with(|| left.kind.cmp(&right.kind))
    });
    combined
}

/// Whether the text is clean enough to store without asking the user.
pub fn is_clean(text: &str) -> bool {
    scan(text).is_clean()
}

// ------------------------------------------------------------------- detectors

/// PEM and OpenSSH private-key blocks.
fn scan_private_keys(text: &str, found: &mut SecretScan) {
    const MARKERS: [&str; 3] = [
        "PRIVATE KEY-----",
        "PRIVATE KEY BLOCK",
        "OPENSSH PRIVATE KEY",
    ];
    let mut offset = 0usize;
    for line in text.split_inclusive('\n') {
        if line.contains("-----BEGIN") && MARKERS.iter().any(|marker| line.contains(marker)) {
            found.push(
                SecretKind::PrivateKey,
                offset..offset + line.trim_end().len(),
                text,
                "pem_header",
            );
        }
        offset += line.len();
    }
    // A single-line blob that carries the header without a line break.
    if let Some(position) = text.find("-----BEGIN") {
        if text[position..].contains("PRIVATE KEY-----")
            && !found.findings.iter().any(|finding| {
                finding.rule == "pem_header"
                    && finding.span.start <= position
                    && position < finding.span.end
            })
        {
            let end = (position + 64).min(text.len());
            found.push(SecretKind::PrivateKey, position..end, text, "pem_inline");
        }
    }
}

/// `password: value` and friends, in three languages.
fn scan_assigned_secrets(text: &str, found: &mut SecretScan) {
    const NAMES: [&str; 14] = [
        "password",
        "passwd",
        "passphrase",
        "pwd",
        "secret",
        "api key",
        "api_key",
        "apikey",
        "access token",
        "access_token",
        "token",
        "пароль",
        "токен",
        "парол",
    ];
    let mut offset = 0usize;
    for line in text.split_inclusive('\n') {
        if let Some((name_end, value_start)) = assignment_split(line) {
            let name = line[..name_end].to_lowercase();
            if NAMES.iter().any(|candidate| name.contains(candidate)) {
                let value = line[value_start..].trim_end();
                if is_secret_like_value(value) {
                    let start = offset + value_start;
                    found.push(
                        SecretKind::PasswordAssignment,
                        start..start + value.len(),
                        text,
                        "assigned_secret",
                    );
                }
            }
        }
        offset += line.len();
    }
}

/// Finds the `:` or `=` that separates a name from a value.
fn assignment_split(line: &str) -> Option<(usize, usize)> {
    let bytes = line.as_bytes();
    // Only the first separator counts, so a value may contain more of them.
    let position = bytes
        .iter()
        .position(|byte| *byte == b':' || *byte == b'=')?;
    // A URL scheme is not an assignment. The test compares a prefix instead of slicing
    // three bytes, because the character after the separator can be multi-byte and a
    // byte slice of a fixed length would land inside it.
    if line[position..].starts_with("://") {
        return None;
    }
    let name = line[..position].trim();
    // A sentence like "the password is stored safely, always:" has a long,
    // multi-word left side; a setting name is short.
    if name.is_empty() || name.len() > 64 || name.split_whitespace().count() > 4 {
        return None;
    }
    let rest_start = position + 1;
    let trimmed = line[rest_start..].len() - line[rest_start..].trim_start().len();
    Some((position, rest_start + trimmed))
}

/// Whether a value looks like something the user should not store unattended.
fn is_secret_like_value(value: &str) -> bool {
    let trimmed = value.trim_matches(['"', '\'', '`', ' ', ',', ';']);
    if trimmed.len() < ASSIGNMENT_MIN_VALUE {
        return false;
    }
    // Placeholders are the usual way to document a field, not a real secret.
    let lowered = trimmed.to_lowercase();
    const PLACEHOLDERS: [&str; 8] = [
        "<",
        ">",
        "...",
        "xxx",
        "***",
        "your_",
        "example",
        "placeholder",
    ];
    if PLACEHOLDERS
        .iter()
        .any(|placeholder| lowered.contains(placeholder))
    {
        return false;
    }
    true
}

/// Recognizable provider tokens, JWTs, and long random-looking strings.
fn scan_tokens(text: &str, found: &mut SecretScan) {
    const PREFIXES: [(&str, &str); 16] = [
        ("sk-", "openai_key"),
        ("sk-ant-", "anthropic_key"),
        ("ghp_", "github_token"),
        ("gho_", "github_token"),
        ("ghu_", "github_token"),
        ("ghs_", "github_token"),
        ("github_pat_", "github_token"),
        ("xoxb-", "slack_token"),
        ("xoxp-", "slack_token"),
        ("xoxa-", "slack_token"),
        ("glpat-", "gitlab_token"),
        ("npm_", "npm_token"),
        ("pypi-", "pypi_token"),
        ("hf_", "huggingface_token"),
        ("dckr_pat_", "docker_token"),
        ("r8_", "provider_token"),
    ];

    for (start, token) in tokens(text) {
        let end = start + token.len();

        // A JSON Web Token: three base64url segments, the first of which is a
        // JSON header. The structural check keeps ordinary dotted words out.
        if let Some(consumed) = jwt_length(token) {
            found.push(
                SecretKind::Jwt,
                start..start + consumed,
                text,
                "jwt_segments",
            );
            continue;
        }

        // A recognizable provider prefix is enough on its own.
        if PREFIXES
            .iter()
            .find(|(prefix, _)| token.starts_with(prefix))
            .is_some_and(|(prefix, _)| token.len() >= prefix.len() + 12)
        {
            found.push(SecretKind::ApiToken, start..end, text, "provider_prefix");
            continue;
        }

        if token.starts_with("AKIA")
            && token.len() >= 20
            && token[4..]
                .chars()
                .all(|character| character.is_ascii_uppercase() || character.is_ascii_digit())
        {
            found.push(SecretKind::ApiToken, start..end, text, "aws_key_id");
            continue;
        }
        if token.starts_with("AIza") && token.len() >= 39 {
            found.push(SecretKind::ApiToken, start..end, text, "google_key");
            continue;
        }
        if token.starts_with("ya29.") && token.len() >= 24 {
            found.push(SecretKind::ApiToken, start..end, text, "google_oauth");
            continue;
        }

        if looks_high_entropy(token) {
            found.push(
                SecretKind::HighEntropyToken,
                start..end,
                text,
                "high_entropy",
            );
        }
    }

    // A bearer header carries the token after the scheme.
    for (start, token) in tokens(text) {
        if token.eq_ignore_ascii_case("bearer") {
            let after = start + token.len();
            let rest = text[after..].trim_start_matches([' ', ':']);
            let value: String = rest
                .chars()
                .take_while(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | '=')
                })
                .collect();
            if value.len() >= 20 {
                let value_start = after + (text[after..].len() - rest.len());
                found.push(
                    SecretKind::ApiToken,
                    value_start..value_start + value.len(),
                    text,
                    "bearer_header",
                );
            }
        }
    }
}

/// Recovery-code blocks such as `A1B2-C3D4-E5F6`.
fn scan_recovery_codes(text: &str, found: &mut SecretScan) {
    const LABELS: [&str; 6] = [
        "recovery code",
        "recovery codes",
        "backup code",
        "backup codes",
        "код восстановления",
        "коды восстановления",
    ];
    let mut offset = 0usize;
    for line in text.split_inclusive('\n') {
        let lowered = line.to_lowercase();
        let labelled = LABELS.iter().any(|label| lowered.contains(label));
        for (start, token) in tokens(line) {
            let groups: Vec<&str> = token.split('-').collect();
            let blocked = groups.len() >= 3
                && groups.iter().all(|group| {
                    (3..=10).contains(&group.len())
                        && group
                            .chars()
                            .all(|character| character.is_ascii_alphanumeric())
                });
            // A labelled one-time code is often written without separators, and it
            // is almost always alphanumeric with digits and capitals.
            let code_like = (8..=64).contains(&token.len())
                && token
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric())
                && token.chars().any(|character| character.is_ascii_digit())
                && token
                    .chars()
                    .any(|character| character.is_ascii_uppercase());
            let labelled_code = labelled && code_like;
            let labelled_block = labelled
                && groups.len() >= 2
                && groups.iter().all(|group| {
                    (3..=10).contains(&group.len())
                        && group
                            .chars()
                            .all(|character| character.is_ascii_alphanumeric())
                });
            if blocked || labelled_code || labelled_block {
                let absolute = offset + start;
                found.push(
                    SecretKind::RecoveryCode,
                    absolute..absolute + token.len(),
                    text,
                    if blocked {
                        "recovery_block"
                    } else {
                        "recovery_label"
                    },
                );
                break;
            }
        }
        offset += line.len();
    }
}

/// Digit runs that pass the Luhn check.
fn scan_payment_cards(text: &str, found: &mut SecretScan) {
    let bytes = text.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        if !bytes[index].is_ascii_digit() {
            index += 1;
            continue;
        }
        let start = index;
        let mut digits = String::new();
        while index < bytes.len() {
            let byte = bytes[index];
            if byte.is_ascii_digit() {
                digits.push(byte as char);
                index += 1;
            } else if matches!(byte, b' ' | b'-')
                && index + 1 < bytes.len()
                && bytes[index + 1].is_ascii_digit()
            {
                index += 1;
            } else {
                break;
            }
        }
        let end = index;
        if (13..=19).contains(&digits.len()) && luhn_valid(&digits) {
            found.push(SecretKind::PaymentCard, start..end, text, "luhn_valid");
        }
    }
}

// -------------------------------------------------------------------- helpers

/// Every token of the text with its byte offset.
///
/// A token is a run of characters that can appear inside a credential. Whitespace,
/// quotes, brackets, and commas end a token.
fn tokens(text: &str) -> Vec<(usize, &str)> {
    let mut tokens = Vec::new();
    let mut start: Option<usize> = None;
    for (offset, character) in text.char_indices() {
        let allowed = character.is_ascii_alphanumeric()
            || matches!(
                character,
                '-' | '_' | '.' | '+' | '/' | '=' | ':' | '@' | '~' | '!' | '$' | '%' | '&' | '*'
            );
        match (allowed, start) {
            (true, None) => start = Some(offset),
            (false, Some(begin)) => {
                tokens.push((begin, &text[begin..offset]));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(begin) = start {
        tokens.push((begin, &text[begin..]));
    }
    tokens
}

/// One-based line number of a byte offset.
fn line_of(text: &str, offset: usize) -> usize {
    text[..offset.min(text.len())]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1
}

/// Length of the JWT prefix in `token`, when the token is a JWT.
///
/// The first segment must decode to a JSON object that mentions `alg`, which is
/// what keeps `a.b.c` file names and version strings out.
fn jwt_length(token: &str) -> Option<usize> {
    let mut segments = token.split('.');
    let header = segments.next()?;
    let payload = segments.next()?;
    let signature = segments.next()?;
    if segments.next().is_some() {
        return None;
    }
    if !header.starts_with("eyJ") || payload.len() < 8 || signature.len() < 8 {
        return None;
    }
    let decoded = base64url_decode(header)?;
    let text = String::from_utf8(decoded).ok()?;
    if !text.starts_with('{') || !text.contains("alg") {
        return None;
    }
    Some(header.len() + 1 + payload.len() + 1 + signature.len())
}

/// Small base64url decoder, so no dependency is added for one check.
fn base64url_decode(input: &str) -> Option<Vec<u8>> {
    fn value(character: u8) -> Option<u8> {
        match character {
            b'A'..=b'Z' => Some(character - b'A'),
            b'a'..=b'z' => Some(character - b'a' + 26),
            b'0'..=b'9' => Some(character - b'0' + 52),
            b'-' => Some(62),
            b'_' => Some(63),
            _ => None,
        }
    }
    let cleaned: Vec<u8> = input.bytes().filter(|byte| *byte != b'=').collect();
    let mut output = Vec::with_capacity(cleaned.len() * 3 / 4);
    for chunk in cleaned.chunks(4) {
        let mut buffer = 0u32;
        let mut bits = 0u32;
        for byte in chunk {
            buffer = (buffer << 6) | u32::from(value(*byte)?);
            bits += 6;
        }
        while bits >= 8 {
            bits -= 8;
            output.push(((buffer >> bits) & 0xff) as u8);
        }
    }
    Some(output)
}

/// Whether a token looks like a random credential rather than a word or an id.
fn looks_high_entropy(token: &str) -> bool {
    if token.len() < HIGH_ENTROPY_MIN_LEN {
        return false;
    }
    if !token.chars().all(|character| {
        character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '+' | '/' | '=')
    }) {
        return false;
    }
    let has_lower = token
        .chars()
        .any(|character| character.is_ascii_lowercase());
    let has_upper = token
        .chars()
        .any(|character| character.is_ascii_uppercase());
    let has_digit = token.chars().any(|character| character.is_ascii_digit());
    let classes = [has_lower, has_upper, has_digit]
        .iter()
        .filter(|present| **present)
        .count();
    if classes < 2 {
        return false;
    }
    let hex_only = token.chars().all(|character| character.is_ascii_hexdigit());
    let threshold = if hex_only {
        HIGH_ENTROPY_HEX_BITS
    } else {
        HIGH_ENTROPY_MIN_BITS
    };
    entropy_bits_per_char(token) >= threshold
}

/// Shannon entropy of a string, in bits per character.
pub fn entropy_bits_per_char(text: &str) -> f64 {
    if text.is_empty() {
        return 0.0;
    }
    let total = text.chars().count() as f64;
    let mut counts: std::collections::HashMap<char, usize> = std::collections::HashMap::new();
    for character in text.chars() {
        *counts.entry(character).or_insert(0) += 1;
    }
    counts
        .values()
        .map(|count| {
            let probability = *count as f64 / total;
            -probability * probability.log2()
        })
        .sum()
}

/// Luhn checksum used by payment cards.
pub fn luhn_valid(digits: &str) -> bool {
    if digits.len() < 13 || !digits.chars().all(|character| character.is_ascii_digit()) {
        return false;
    }
    let mut sum = 0u32;
    for (index, character) in digits.chars().rev().enumerate() {
        let mut value = character.to_digit(10).unwrap_or(0);
        if index % 2 == 1 {
            value *= 2;
            if value > 9 {
                value -= 9;
            }
        }
        sum += value;
    }
    // An all-zero run is structurally valid but is never a real card number.
    sum.is_multiple_of(10) && digits.chars().any(|character| character != '0')
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fabricated PEM header. No real key material is used anywhere in tests.
    const FICTIONAL_PEM: &str =
        "-----BEGIN PRIVATE KEY-----\nFICTIONAL_KEY_BODY_NOT_A_REAL_KEY\n-----END PRIVATE KEY-----";
    const FICTIONAL_JWT: &str =
        "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiJGSU3USU9OQUwifQ.FICTIONAL_SIGNATURE_VALUE";
    const FICTIONAL_OPENAI: &str = "sk-FICTIONAL0000000000000000000000000000";
    const FICTIONAL_GITHUB: &str = "ghp_FICTIONAL0000000000000000000000000000";
    const FICTIONAL_CARD: &str = "4111 1111 1111 1111";

    #[test]
    fn a_private_key_block_is_recognized() {
        let found = scan(FICTIONAL_PEM);
        assert!(!found.is_clean());
        assert_eq!(found.kinds(), vec![SecretKind::PrivateKey]);
        assert_eq!(found.rules(), vec!["pem_header"]);
    }

    #[test]
    fn a_json_web_token_is_recognized_by_structure() {
        let found = scan(&format!("token = {FICTIONAL_JWT}"));
        assert!(found.kinds().contains(&SecretKind::Jwt));
        // Ordinary dotted words are not JWTs.
        assert!(scan("version 1.2.3 and a.b.c are fine").is_clean());
    }

    #[test]
    fn provider_token_prefixes_are_recognized() {
        for token in [
            FICTIONAL_OPENAI,
            FICTIONAL_GITHUB,
            "xoxb-FICTIONAL-000000000000",
            "AIzaFICTIONAL000000000000000000000000000",
            "AKIAFICTIONALKEY1234",
        ] {
            let found = scan(token);
            assert!(
                !found.is_clean(),
                "expected {token} to be recognized as a token"
            );
            assert!(found.kinds().contains(&SecretKind::ApiToken));
        }
        // Too short to be a key.
        assert!(scan("sk-short").is_clean());
    }

    #[test]
    fn assigned_passwords_are_recognized_in_three_languages() {
        for line in [
            "password: hunter2FICTIONAL",
            "password = \"hunter2FICTIONAL\"",
            "API_KEY: FICTIONAL_VALUE_1234567890",
            "пароль: FICTIONAL_ПАРОЛЬ_123",
            "token = FICTIONAL_TOKEN_VALUE_1",
        ] {
            let found = scan(line);
            assert!(!found.is_clean(), "expected {line} to be recognized");
            assert!(
                found.kinds().contains(&SecretKind::PasswordAssignment)
                    || found.kinds().contains(&SecretKind::HighEntropyToken),
                "unexpected kinds for {line}: {:?}",
                found.kinds()
            );
        }
        // A placeholder is documentation, not a secret.
        assert!(scan("password: <your password>").is_clean());
        assert!(scan("password: ...").is_clean());
        assert!(scan("password: your_password_here").is_clean());
        // A URL is not an assignment.
        assert!(scan("endpoint = https://example.com/path").is_clean());
    }

    #[test]
    fn a_multi_byte_value_after_a_label_does_not_panic() {
        // Regression: the separator check used to slice three bytes from the colon, which
        // lands inside the first letter when the value is Cyrillic and the whole scan
        // panicked instead of reporting a finding.
        let found = scan("Улучшенный: привет мир");
        assert!(found.is_clean(), "{:?}", found.findings());
        let found = scan("пароль: секретное_значение");
        assert!(!found.is_clean());
        assert!(found.kinds().contains(&SecretKind::PasswordAssignment));
        // A value that is a URL scheme is still not an assignment.
        assert!(scan("endpoint = https://пример.рф/путь").is_clean());
        // A short value is still an assignment.
        assert!(!scan("pwd: abcd").is_clean());
    }

    #[test]
    fn recovery_codes_are_recognized() {
        let found = scan("Recovery codes:\nA1B2-C3D4-E5F6\nG7H8-I9J0-K1L2");
        assert!(found.kinds().contains(&SecretKind::RecoveryCode));
        assert!(found.findings().len() >= 2, "{:?}", found.findings());

        // Ukrainian and Russian labels work as well.
        assert!(!scan("Код восстановления: A1B2C3D4E5F6").is_clean());
        assert!(!scan("коды восстановления: ABCD-EFGH").is_clean());

        // Prose about recovery codes is not a recovery code.
        assert!(scan("Recovery codes are not enabled for this account").is_clean());
        assert!(scan("The user asked how backup codes work").is_clean());

        // A date-like or version-like token is not a recovery code.
        assert!(scan("released 2026-09-19 at 12:00").is_clean());
    }

    #[test]
    fn payment_cards_are_checked_with_luhn() {
        let found = scan(FICTIONAL_CARD);
        assert!(found.kinds().contains(&SecretKind::PaymentCard));

        // The same digits with one changed fail the checksum.
        assert!(scan("4111 1111 1111 1112").is_clean());
        // A long number that is not a card is not reported, because the checksum
        // does not hold.
        assert!(!luhn_valid("1234567890123456789"));
        assert!(scan("order 1234567890123456789").is_clean());
        assert!(luhn_valid("4111111111111111"));
        assert!(!luhn_valid("4111111111111112"));
        assert!(!luhn_valid("0000000000000000"));
        assert!(!luhn_valid("4111"));
    }

    #[test]
    fn long_random_tokens_are_recognized_and_words_are_not() {
        let random = "kQ3vT9xR2mB7nD4sL6pW8yZ1aC5eF0gH";
        let found = scan(random);
        assert!(found.kinds().contains(&SecretKind::HighEntropyToken));

        // Ordinary prose, paths, and identifiers are left alone. False positives
        // are still possible and are documented as such.
        assert!(scan("The user prefers dark themes in the evening").is_clean());
        assert!(scan("C:\\Users\\angel\\Documents\\project").is_clean());
        assert!(scan("internationalization-considerations").is_clean());
        // A hex digest is a deliberate false positive: a hash is not a password,
        // but the filter cannot tell the difference and asks the user instead.
        assert!(scan("ae3f1c9d5b7e2a4f6c8d0b1a3e5f7c9d")
            .kinds()
            .contains(&SecretKind::HighEntropyToken));
        // A plain sentence of the same length is not a token.
        assert!(scan("this sentence has no token inside it at all").is_clean());
    }

    #[test]
    fn findings_never_contain_the_matched_text() {
        let text = format!(
            "{FICTIONAL_PEM}\npassword: FICTIONAL_SECRET_VALUE\n{FICTIONAL_OPENAI}\n{FICTIONAL_CARD}"
        );
        let found = scan(&text);
        assert!(found.len() >= 4);
        for finding in found.findings() {
            let debug = format!("{finding:?}");
            assert!(!debug.contains("FICTIONAL_KEY_BODY"));
            assert!(!debug.contains("FICTIONAL_SECRET_VALUE"));
            assert!(!debug.contains("sk-FICTIONAL"));
            assert!(!debug.contains("4111"));
            // Only the kind, position, and rule are kept.
            assert!(debug.contains("kind"));
            assert!(debug.contains("line"));
        }
        // The scan itself renders without content too.
        let rendered = format!("{found:?}");
        assert!(!rendered.contains("FICTIONAL_SECRET_VALUE"));
    }

    #[test]
    fn findings_point_at_the_right_line() {
        let text = "first line\nsecond line\npassword: FICTIONAL_VALUE\n".to_string();
        let found = scan(&text);
        let finding = found
            .findings()
            .iter()
            .find(|finding| finding.kind == SecretKind::PasswordAssignment)
            .expect("the assigned secret must be reported");
        assert_eq!(finding.line, 3);
    }

    #[test]
    fn a_clean_conversation_stays_clean() {
        let found = scan_all([
            "Привет! Как дела?",
            "Всё хорошо, спасибо. Обсуждаем проект JARVIS.",
            "The user prefers short answers.",
        ]);
        assert!(found.is_clean(), "{:?}", found.findings());
    }

    #[test]
    fn entropy_and_luhn_helpers_are_sane() {
        assert!(entropy_bits_per_char("aaaaaaaa") < 0.1);
        assert!(entropy_bits_per_char("abcdefghijklmnop") > 3.5);
        assert_eq!(entropy_bits_per_char(""), 0.0);
    }

    #[test]
    fn the_base64_helper_decodes_what_the_jwt_check_needs() {
        let decoded = base64url_decode("eyJhbGciOiJIUzI1NiJ9").unwrap();
        let text = String::from_utf8(decoded).unwrap();
        assert!(text.starts_with('{'));
        assert!(text.contains("alg"));
        assert!(base64url_decode("!!!!").is_none());
    }
}

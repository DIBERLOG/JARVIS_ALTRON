//! The small amount of text handling shared by the parts of the application
//! that write about themselves: lifecycle reports, diagnostics, and error
//! details.
//!
//! One rule matters here: **a path is never written down.** A report about the
//! application is read by a person and often pasted into a message, and a path
//! carries a user name, a disk layout, and sometimes the name of a file that
//! says what the user was doing. The functions below replace paths with a
//! marker and keep everything else.

/// How long a single line may be.
pub const MAX_LINE_CHARS: usize = 200;

/// Replaces anything that looks like a path with a marker.
///
/// Windows drive paths, UNC paths, and `/`-separated paths are all covered. The
/// replacement is deliberate: a report that says `could not read <path>` is as
/// useful as one that says `could not read C:/Users/someone/...`, and it is safe
/// to send.
pub fn redact(text: &str) -> String {
    let mut cleaned = String::with_capacity(text.len());
    let characters: Vec<char> = text.chars().collect();
    let mut index = 0usize;
    while index < characters.len() {
        let character = characters[index];
        // A drive path: a letter, a colon, and a separator.
        let drive = character.is_ascii_alphabetic()
            && characters.get(index + 1) == Some(&':')
            && matches!(characters.get(index + 2), Some('\\') | Some('/'));
        // A UNC path, or a `/`-rooted path that starts a word.
        let after_boundary = index == 0
            || characters[index - 1].is_whitespace()
            || matches!(characters[index - 1], '"' | '\'' | '(' | '[' | '=' | ':');
        let unc = character == '\\' && characters.get(index + 1) == Some(&'\\');
        let rooted = character == '/'
            && after_boundary
            && characters
                .get(index + 1)
                .map(|next| next.is_alphanumeric())
                .unwrap_or(false);
        if drive || unc || rooted {
            cleaned.push_str("<path>");
            index += if drive {
                3
            } else if unc {
                2
            } else {
                1
            };
            // The first segment: everything up to a space or a terminator.
            index = skip_path_segment(&characters, index);
            // A path with spaces in it keeps going while the next word still
            // looks like part of a path: it carries a separator, or it ends with
            // an extension.
            while let Some(&space) = characters.get(index) {
                if !space.is_whitespace() {
                    break;
                }
                let start = index + 1;
                let end = skip_path_segment(&characters, start);
                let word: String = characters[start..end].iter().collect();
                if word.contains(['\\', '/']) || has_extension(&word) {
                    index = end;
                } else {
                    break;
                }
            }
            continue;
        }
        cleaned.push(character);
        index += 1;
    }
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Advances past one path segment: up to a space or a terminator.
fn skip_path_segment(characters: &[char], mut index: usize) -> usize {
    while let Some(&next) = characters.get(index) {
        if next.is_whitespace() || matches!(next, ',' | ';' | ')' | ']' | '"' | '\'') {
            break;
        }
        index += 1;
    }
    index
}

/// Whether a word ends with something that looks like a file extension.
fn has_extension(word: &str) -> bool {
    let Some((_, extension)) = word.rsplit_once('.') else {
        return false;
    };
    (2..=5).contains(&extension.len())
        && extension
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
}
/// The last component of a path, for a report that may name a file but never
/// the folder it is in.
pub fn file_label(path: &str) -> String {
    let trimmed = path.trim().trim_end_matches(['\\', '/']);
    let name = trimmed
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(trimmed)
        .to_string();
    if name.is_empty() {
        "<unnamed>".to_string()
    } else {
        shorten(&name, MAX_LINE_CHARS)
    }
}

/// Shortens a line to `limit` characters, with an ellipsis.
pub fn shorten(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let mut shortened: String = text.chars().take(limit.saturating_sub(1)).collect();
    shortened.push('…');
    shortened
}

/// Whether a piece of text still carries something it should not.
///
/// This is a second line of defence for the diagnostics screen: the report is
/// built from fields that cannot hold user content, and this check fails if one
/// of them ever learns how.
pub fn looks_like_it_carries_a_path_or_a_secret(text: &str) -> Option<&'static str> {
    let lowered = text.to_lowercase();
    if lowered.contains(":\\") || lowered.contains("\\\\") || lowered.contains("/users/") {
        return Some("path");
    }
    if lowered.contains("key.dpapi") {
        return Some("key file");
    }
    for marker in [
        "password=",
        "password:",
        "passwd",
        "token=",
        "api_key",
        "apikey",
        "secret=",
        "authorization:",
        "bearer ",
        "-----begin",
    ] {
        if lowered.contains(marker) {
            return Some("secret");
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_windows_path_becomes_a_marker() {
        assert_eq!(
            redact("could not read C:/Users/someone/AppData/key.dpapi"),
            "could not read <path>"
        );
        assert_eq!(
            redact(r"failed on C:\Program Files\Jarvis\app.exe while starting"),
            "failed on <path> while starting"
        );
        assert_eq!(
            redact(r"failed on \\server\share\file.bin"),
            "failed on <path>"
        );
    }

    #[test]
    fn a_unix_style_path_becomes_a_marker() {
        assert_eq!(
            redact("failed on /home/someone/models/x.bin"),
            "failed on <path>"
        );
        assert_eq!(
            redact("no such file: /var/log/jarvis.log, again"),
            "no such file: <path>, again"
        );
    }

    #[test]
    fn ordinary_text_is_left_alone() {
        assert_eq!(
            redact("the model server refused to stop"),
            "the model server refused to stop"
        );
        assert_eq!(redact("a/b"), "a/b");
        assert_eq!(redact("either/or"), "either/or");
        assert_eq!(redact(""), "");
        // Whitespace is collapsed, so a report line is one line.
        assert_eq!(
            redact("  two   words\nand a line  "),
            "two words and a line"
        );
    }

    #[test]
    fn a_file_is_named_without_its_folder() {
        assert_eq!(file_label("C:/Users/someone/AppData/notes.db"), "notes.db");
        assert_eq!(file_label(r"C:\data\timers.json"), "timers.json");
        assert_eq!(file_label("/home/someone/ggml-small.bin"), "ggml-small.bin");
        assert_eq!(file_label("plain.db"), "plain.db");
        assert_eq!(file_label("C:/Users/someone/"), "someone");
        assert_eq!(file_label(""), "<unnamed>");
    }

    #[test]
    fn a_long_line_is_shortened() {
        let long = "я".repeat(MAX_LINE_CHARS + 50);
        assert_eq!(
            shorten(&long, MAX_LINE_CHARS).chars().count(),
            MAX_LINE_CHARS
        );
        assert_eq!(shorten("short", MAX_LINE_CHARS), "short");
    }

    #[test]
    fn the_guard_finds_what_must_not_be_in_a_report() {
        assert_eq!(
            looks_like_it_carries_a_path_or_a_secret("model at C:/Users/me/m.bin"),
            Some("path")
        );
        assert_eq!(
            looks_like_it_carries_a_path_or_a_secret("restored from \\\\server\\share"),
            Some("path")
        );
        assert_eq!(
            looks_like_it_carries_a_path_or_a_secret("could not open key.dpapi"),
            Some("key file")
        );
        assert_eq!(
            looks_like_it_carries_a_path_or_a_secret("api_key=abc"),
            Some("secret")
        );
        assert_eq!(
            looks_like_it_carries_a_path_or_a_secret("Authorization: Bearer x"),
            Some("secret")
        );
        assert_eq!(
            looks_like_it_carries_a_path_or_a_secret("everything is fine"),
            None
        );
        assert_eq!(
            looks_like_it_carries_a_path_or_a_secret("python.exe is ready"),
            None
        );
    }
}

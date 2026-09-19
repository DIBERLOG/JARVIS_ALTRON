//! Structural checks of the safe Windows actions.
//!
//! These are not behaviour tests: they read the feature's own sources and assert that the
//! shapes the stage forbids are absent from the code that can run. They exist because a
//! review argument ("nobody would write that") is weaker than a test that fails when someone
//! does.
//!
//! What is checked, and why each one matters:
//!
//! * no command line, shell, or PowerShell anywhere in the feature or in the command module,
//!   and `std::process::Command` appears in exactly one place — the native backend, where it is
//!   handed a validated absolute path with the arguments the user fixed;
//! * no process termination, no input synthesis, and no window-station tricks: closing a window
//!   posts `WM_CLOSE`, and nothing else in the feature can end a process;
//! * no path to the password vault, the notes storage, the AI memory, a master key, or a key
//!   derivation routine: the feature shares the application data directory and nothing else;
//! * no file deletion outside the audit log's own rotation;
//! * Wake-on-LAN is absent and stays absent: it is excluded from the project, so this test also
//!   fails if the words appear anywhere in the workspace's own sources;
//! * the log's shape cannot carry a window title, a path, or a reminder text.

use std::path::{Path, PathBuf};

/// The crate root, so the test works from any working directory.
fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate lives in a workspace")
        .to_path_buf()
}

/// Every Rust source of the feature, with its test module and comments removed.
fn feature_sources() -> Vec<(String, String)> {
    let root = repository_root().join("crates/jarvis-core/src/windows_actions");
    let mut sources = Vec::new();
    visit(&root, &mut sources);
    sources.sort_by(|left, right| left.0.cmp(&right.0));
    assert!(
        sources.len() >= 12,
        "expected the whole feature, got {}",
        sources.len()
    );
    sources
}

fn visit(directory: &Path, into: &mut Vec<(String, String)>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            visit(&path, into);
            continue;
        }
        if path.extension().and_then(|value| value.to_str()) != Some("rs") {
            continue;
        }
        if path.file_name().and_then(|value| value.to_str()) == Some("mod.rs") {
            // `mod.rs` is a list of modules and the design notes; it has nothing to run.
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("a source file must be readable");
        into.push((path.to_string_lossy().into_owned(), shipped_part(&text)));
    }
}

/// The part of a file that actually compiles: comments and the test module are dropped, so a
/// test that names a forbidden word does not match itself and a doc comment cannot fail a scan.
fn shipped_part(source: &str) -> String {
    let shipped = match source.find("#[cfg(test)]") {
        Some(index) => &source[..index],
        None => source,
    };
    shipped
        .lines()
        .filter(|line| {
            let trimmed = line.trim_start();
            !trimmed.starts_with("//") && !trimmed.starts_with("/*") && !trimmed.starts_with('*')
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Removes comments from a source file, so a sentence about what is *not* done cannot fail a
/// scan that is about what *is* done.
fn strip_code_comments(source: &str, extension: &str) -> String {
    let without_blocks = source
        .replace("<!--", "<!--\u{1}")
        .replace("-->", "\u{1}-->");
    let mut kept = String::with_capacity(source.len());
    let mut in_block = false;
    for line in without_blocks.lines() {
        let trimmed = line.trim_start();
        if in_block {
            if let Some(end) = line.find("*/") {
                in_block = false;
                kept.push_str(&line[end + 2..]);
                kept.push('\n');
            }
            continue;
        }
        if trimmed.starts_with("/*") {
            in_block = true;
            continue;
        }
        if trimmed.starts_with("//") {
            continue;
        }
        if extension == "ftl" && trimmed.starts_with('#') {
            continue;
        }
        // `<!--` comments are removed inline; Svelte markup keeps the rest of the line.
        let mut rest = line;
        let mut cleaned = String::new();
        while let Some(start) = rest.find("<!--") {
            cleaned.push_str(&rest[..start]);
            match rest[start..].find("\u{1}-->") {
                Some(end) => rest = &rest[start + end + 4..],
                None => {
                    rest = "";
                    break;
                }
            }
        }
        cleaned.push_str(rest);
        kept.push_str(&cleaned);
        kept.push('\n');
    }
    kept
}

/// Whether a shell name appears only as an entry of a refusal list.
///
/// The two lists are `FORBIDDEN_EXECUTABLE_NAMES` in the policy and `FORBIDDEN_ARGUMENT_NAMES`
/// in the tool decoder; both are lists of quoted names, one per line, so an occurrence on any
/// other kind of line is a real problem.
fn only_in_refusal_lists(path: &str, text: &str, needles: &[&str]) -> Result<(), String> {
    let is_refusal_list_file = path.contains("policy.rs") || path.contains("tools.rs");
    for needle in needles {
        for (number, line) in text.lines().enumerate() {
            if !line.to_lowercase().contains(&needle.to_lowercase()) {
                continue;
            }
            if !is_refusal_list_file {
                return Err(format!(
                    "line {} names {needle:?} in an execution module",
                    number + 1
                ));
            }
            let trimmed = line.trim();
            // A list entry is a quoted name, optionally with an extension, on its own line.
            let is_entry = trimmed.starts_with(&format!("\"{needle}"))
                || trimmed.starts_with(&format!("(\"{needle}"));
            if !is_entry {
                return Err(format!(
                    "line {} uses {needle:?} outside the refusal list: {trimmed}",
                    number + 1
                ));
            }
        }
    }
    Ok(())
}

fn assert_absent(sources: &[(String, String)], needles: &[&str], why: &str) {
    for (path, text) in sources {
        for needle in needles {
            assert!(
                !text.contains(needle),
                "{path} must not contain {needle:?}: {why}"
            );
        }
    }
}

#[test]
fn no_source_of_the_feature_can_run_a_command_line() {
    // `policy.rs` is the one module allowed to name a shell, because it names shells in order
    // to refuse them; the check at the end of this test proves it still does.
    let sources = feature_sources();
    // Names of shells may appear in exactly two places, both of them refusal lists: the policy,
    // which refuses to start them, and the tool decoder, which refuses the argument names that
    // would ask for one. Everywhere else — and anywhere in those two files outside such a list
    // — a shell name is a failure.
    let refusal_names = [
        concat!("power", "shell"),
        concat!("pw", "sh"),
        concat!("cmd", ".exe"),
        concat!("wscript", ".exe"),
        concat!("mshta", ".exe"),
        concat!("rundll32", ".exe"),
    ];
    for (path, text) in &sources {
        assert!(
            only_in_refusal_lists(path, text, &refusal_names).is_ok(),
            "{path} names a shell outside a refusal list: {}",
            only_in_refusal_lists(path, text, &refusal_names).unwrap_err()
        );
    }

    // The rest of the execution shapes are refused everywhere, without exception.
    assert_absent(
        &sources,
        &[
            concat!("cmd", " /C"),
            concat!("/C ", "start"),
            concat!("Shell", "Execute"),
            concat!("Win", "Exec"),
            concat!("Create", "Process"),
            concat!("invoke-express", "ion"),
            concat!("Start-", "Process"),
            concat!("std::process::Command::new(&\"", ""),
        ],
        "a typed action is the only way to ask for anything",
    );

    // The refusal lists are still there: a policy that forgot them would pass the scan above.
    let policy = sources
        .iter()
        .find(|(path, _)| path.contains("policy.rs"))
        .map(|(_, text)| text.clone())
        .expect("the policy module exists");
    for refused in refusal_names {
        assert!(
            policy.contains(refused),
            "the policy must refuse {refused:?}: a launcher can run anything"
        );
    }
    let tools = sources
        .iter()
        .find(|(path, _)| path.contains("tools.rs"))
        .map(|(_, text)| text.clone())
        .expect("the tool catalogue exists");
    for refused in ["command", "shell", "exec", "path", "arguments"] {
        assert!(
            tools.contains(refused),
            "the decoder must refuse an argument named {refused:?}"
        );
    }

    // `std::process::Command` is allowed in the native backend alone, and only through a
    // `LaunchSpec` the allowlist produced.
    for (path, text) in &sources {
        if text.contains("Command::new(") {
            assert!(
                path.ends_with("backend\\native.rs") || path.ends_with("backend/native.rs"),
                "{path} may not start a process: only the native backend does that"
            );
            assert!(
                text.contains("Command::new(&spec.program)"),
                "{path} must start exactly the executable from the stored allowlist entry, and \
                 never a name it computed itself"
            );
            // The arguments come from the same stored entry; nothing on this path is built from
            // a request, a message, or a tool call.
            assert!(
                text.contains("command.args(&spec.arguments)"),
                "{path} must pass the fixed arguments that were stored with the entry"
            );
            assert!(
                !text.contains("shell") || text.contains("never a shell"),
                "{path} must not ask for a shell"
            );
        }
    }
}

#[test]
fn no_source_of_the_feature_can_end_a_process_or_type_for_the_user() {
    let sources = feature_sources();
    assert_absent(
        &sources,
        &[
            concat!("Terminate", "Process"),
            concat!("task", "kill"),
            concat!("NtTerminate", "Process"),
            concat!("PROCESS_TERMINATE", ""),
            concat!("PROCESS_VM_", "WRITE"),
            concat!("PROCESS_CREATE_", "THREAD"),
            concat!("Zi", "p"),
            concat!("keybd", "_event"),
            concat!("mouse", "_event"),
            concat!("Send", "Input"),
            concat!("SetWindowsHook", "Ex"),
            concat!("BlockInput", ""),
            concat!("CreateRemote", "Thread"),
            concat!("WriteProcess", "Memory"),
        ],
        "closing a window posts WM_CLOSE, and nothing here may end a process or type",
    );

    // Reading a process name needs a handle, so `OpenProcess` may appear — but only with the
    // right to *ask*, never with the right to end or to write.
    for (path, text) in &sources {
        if text.contains("OpenProcess(") {
            assert!(
                text.contains("PROCESS_QUERY_LIMITED_INFORMATION"),
                "{path} may only open a process to read its name"
            );
        }
    }
}

#[test]
fn no_source_of_the_feature_reaches_the_encrypted_storages_or_a_key() {
    let sources = feature_sources();
    assert_absent(
        &sources,
        &[
            concat!("jarvis.system", ".exec"),
            concat!("Vault", "Paths"),
            concat!("vault", "::"),
            concat!("Notes", "Store"),
            concat!("notes", "::"),
            concat!("memory", "::store"),
            concat!("derive", "_key"),
            concat!("master", "_key"),
            concat!("MasterKey", ""),
            concat!("keyring", ""),
            concat!("Secret", "String"),
            concat!("Dpapi", "MasterKey"),
            concat!("lua", "::"),
            concat!("mlua", ""),
            concat!("Lua", "Sandbox"),
        ],
        "the feature may not read a vault, a note, the AI memory, or a key",
    );

    // DPAPI is used for one thing only: sealing a reminder's own text. The key derivation and
    // the storage session must stay out of reach.
    for (path, text) in &sources {
        if text.contains("dpapi_seal_bytes") || text.contains("dpapi_open_bytes") {
            assert!(
                path.contains("timers.rs") || path.contains("session.rs"),
                "{path} may only use DPAPI for the reminder text it owns"
            );
        }
    }
}

#[test]
fn only_the_audit_log_removes_a_file() {
    let sources = feature_sources();
    for (path, text) in &sources {
        let deletes = text.contains("remove_file")
            || text.contains("remove_dir")
            || text.contains("remove_dir_all");
        if deletes {
            assert!(
                path.contains("audit.rs"),
                "{path} must not delete anything: this stage has no file deletion"
            );
        }
    }
    // The one deletion is the rotation of the log itself, and it is bounded to one file.
    let audit = sources
        .iter()
        .find(|(path, _)| path.contains("audit.rs"))
        .map(|(_, text)| text.clone())
        .expect("the audit module exists");
    assert!(audit.contains("remove_file(&rotated)"));
    assert!(audit.contains("AUDIT_ROTATED_FILE"));
}

#[test]
fn wake_on_lan_is_absent_from_the_whole_workspace() {
    // The stage excludes it completely, and this is the test that keeps it excluded: it scans
    // the repository's own sources rather than one module, so it cannot be added quietly.
    let root = repository_root();
    let mut checked = 0usize;
    let mut offenders = Vec::new();
    let needles = [
        concat!("wake", "onlan"),
        concat!("wake", "_on_lan"),
        concat!("Wake", "-on-LAN"),
        concat!("magic", "_packet"),
        concat!("Magic", "Packet"),
        concat!("wake-on-", "lan"),
    ];
    for directory in ["crates", "frontend/src"] {
        scan_workspace(
            &root.join(directory),
            &needles,
            &mut checked,
            &mut offenders,
        );
    }
    assert!(
        checked > 100,
        "expected to scan the workspace, saw {checked} files"
    );
    assert!(
        offenders.is_empty(),
        "Wake-on-LAN is excluded from this project, but it appears in: {offenders:?}"
    );
}

fn scan_workspace(
    directory: &Path,
    needles: &[&str],
    checked: &mut usize,
    offenders: &mut Vec<String>,
) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("");
            if matches!(name, "node_modules" | ".routify" | "dist" | "target") {
                continue;
            }
            scan_workspace(&path, needles, checked, offenders);
            continue;
        }
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        // Documentation is skipped: it is the place where the absence of a feature is
        // *stated*, so a sentence about Wake-on-LAN not being implemented belongs there.
        if !matches!(extension, "rs" | "ts" | "svelte" | "ftl") {
            continue;
        }
        // This test names the words; it is the one file allowed to.
        if path
            .to_string_lossy()
            .ends_with("windows_actions_isolation.rs")
        {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        *checked += 1;
        // A comment may name the feature in order to say it is excluded; code may not use it.
        let lowered = strip_code_comments(&text, extension).to_lowercase();
        for needle in needles {
            if lowered.contains(&needle.to_lowercase()) {
                offenders.push(path.to_string_lossy().into_owned());
                break;
            }
        }
    }
}

#[test]
fn the_log_cannot_carry_a_title_a_path_or_a_text() {
    let root = repository_root();
    let audit =
        std::fs::read_to_string(root.join("crates/jarvis-core/src/windows_actions/audit.rs"))
            .expect("the audit module exists");
    let shipped = shipped_part(&audit);
    let start = shipped
        .find("pub struct AuditEntry")
        .expect("the entry type exists");
    let end = shipped[start..]
        .find('}')
        .map(|offset| start + offset)
        .expect("the struct ends");
    let entry = &shipped[start..end];
    for forbidden in ["title", "path", "message", "command", "arguments"] {
        assert!(
            !entry.contains(forbidden),
            "an audit entry must not have a {forbidden:?} field: the log holds no content"
        );
    }
    assert!(entry.contains("action_type"));
    assert!(entry.contains("duration_ms"));
    assert!(entry.contains("target"));
}

#[test]
fn the_command_surface_never_takes_a_path_or_a_risk_level_from_the_interface() {
    let root = repository_root();
    let module = std::fs::read_to_string(
        root.join("crates/jarvis-gui/src/tauri_commands/windows_actions.rs"),
    )
    .expect("the command module exists");
    let shipped = shipped_part(&module);
    // The interface asks for actions and confirms them by token; it never sends the request
    // back, so a modified window cannot change what was agreed to.
    assert!(shipped.contains("windows_actions_request"));
    assert!(shipped.contains("windows_actions_confirm"));
    assert!(shipped.contains("token: String"));
    for forbidden in [
        concat!("Command", "::new"),
        concat!("power", "shell"),
        concat!("cmd", ".exe"),
        concat!("std::process", "::Command"),
        concat!("RiskLevel", "::Forbidden"),
        concat!("force_confirm", ""),
    ] {
        assert!(
            !shipped.contains(forbidden),
            "the command module must not contain {forbidden:?}"
        );
    }
    // The only path that enters the allowlist is the one the native dialog returned.
    assert!(shipped.contains("blocking_pick_file"));
    assert!(shipped.contains("AllowedApplicationDraft"));
    // The model path is offered the catalogue and nothing else; prose is never parsed.
    assert!(shipped.contains("ai_tools"));
}

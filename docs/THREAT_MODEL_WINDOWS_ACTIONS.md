# Threat model: safe Windows actions

This is the threat model for the "Команды Windows" feature: what it protects
against, what it does not, and which assumptions would have to hold for each
claim.

## Assets

| Asset | Why it matters |
|---|---|
| The user's session and files | An action runs with the user's own rights. A wrong launch, a wrong window action, or a screenshot is a real effect on real data. |
| The screen's contents | A screenshot of the wrong moment can contain a password, a bank page, or a private message. |
| The password vault, notes, and AI memory | They are the application's own secrets, and none of them may be readable by this feature. |
| The integrity of what the user approved | If a pending approval could be changed after it was shown, the dialog would be theatre. |
| The audit log | It is the only record of what was done, and it must not itself become a leak. |

## Adversaries considered

1. **A prompt injection in the local model's input.** Text the model reads (a
   document, a page the user pasted, a note) tries to make the model ask for an
   action that the user did not intend.
2. **A modified webview.** The interface is a web page. A script running in it
   tries to request an action, to confirm one, or to change what a confirmation
   will do.
3. **A hostile file on disk in the allowlist.** A program the user allowed is
   replaced by another one.
4. **A hostile window.** A window whose title or identity is chosen to make an
   action land somewhere else, or to look like a different program.
5. **A person at the keyboard who is not the owner.** Someone approves a
   confirmation while the owner is away.
6. **A local program reading the feature's files.** The audit log, `timers.json`,
   or the allowlist is read or edited by another process running as the user.

## What is protected, and how

### The model cannot ask for a command line

The catalogue contains only typed tools. Every schema sets
`additionalProperties: false`, the decoder rejects unknown fields, an argument
named `command`, `cmd`, `shell`, `powershell`, `exec`, `execute`, `path`,
`executable`, `args`, `arguments`, `script`, or `environment` is refused with
`forbidden_action`, and an unknown tool name is an error. A tool call becomes a
`WindowsAction` or nothing.

*Assumption:* the decoding is the only path from a tool call to an action, and
`windows_actions_isolation.rs` keeps prose parsing out of the module.

*Residual risk:* a prompt injection can still make the model ask for a
**legitimate** action — for example, starting an allowed browser or closing a
window. That is why every such action needs a confirmation, and why the dialog
shows the exact fields.

### A modified webview cannot change what was approved

The request is stored in the core. The interface receives a preview and a
128-bit single-use token; `windows_actions_confirm` accepts the token and
nothing else, and the core runs the stored request. The token expires, is
compared in constant time, and cannot be replayed.

*Residual risk:* a script in the webview can press "confirm" on a dialog the
user was looking at. It cannot change *what* the dialog says.

### A replaced program is refused

The allowlist stores size, mtime, and SHA-256. A launch re-validates the file,
and a difference is reported as `executable_changed` until the user accepts the
new version. Launchers and script hosts are refused by name, network paths and
environment variables are refused, and the path is never supplied by a caller.

*Residual risk:* a file that is replaced *and* accepted by the user is
trusted again; the feature cannot tell a legitimate update from a malicious one.

### A hostile window cannot be reached by a stale identifier

Identifiers are opaque, minted per listing, valid for 90 seconds, and replaced
by every new listing; an identifier from an earlier listing is refused with
`window_expired`. Before acting, the native backend re-checks that the window
still exists, is visible, still belongs to the same process, is not this
application's own window, and is not a system shell window.

*Residual risk:* a window that is still alive and still matches is acted on. A
program can also choose a title that looks innocuous to the sensitive-window
guard.

### The encrypted storages are out of reach

The feature takes the application data directory and nothing else. It does not
import the vault, the notes store, the memory store, a key derivation routine,
or the Lua sandbox; the isolation test scans for each of them and fails on a
match. Reminder text is the only payload the feature seals, with DPAPI for the
current user, in its own file.

### The log does not leak and cannot be trusted

`AuditEntry` has no title, path, or message field — the struct itself is
checked by a test — and the target is a bounded, sanitized label. Clearing and
exporting need an explicit confirmation.

*Residual risk:* the log lives in the user's own profile. A program running as
that user can read, edit, or delete it. **The log is not evidence and not
tamper-proof**, and nothing in the application treats it as either.

### Confirmation is not authentication

The dialog is a decision point, not a proof of identity. A person at the
keyboard can approve an action; the feature cannot and does not claim to know
who they are. This is why destructive capabilities (deletion, shutdown,
uninstall, credential changes) are simply not implemented: no confirmation
dialog would make them safe enough for this stage.

## What is explicitly not protected

* **The screen is not protected.** The sensitive-window guard stops *this
  feature's* capture while a window whose title looks like a credential prompt
  has the focus. It is a courtesy offered to a person, not a boundary: another
  program running as the user can capture the screen at any time, and the guard
  is skipped when the user turns it off.
* **The machine is not sandboxed.** An allowed program runs with the user's
  full rights. Allowing a browser allows everything a browser can do, including
  downloading and running a file.
* **A decided action is not revocable.** Once an action runs, undoing it is the
  program's business, not the feature's. Closing a window with unsaved work
  loses that work; the dialog says so.
* **Other users' sessions are not touched**, and neither is anything requiring
  elevation: the feature never requests it, and a refused elevation is reported
  as a failure rather than retried.
* **Wake-on-LAN is not implemented and never will be.** It is excluded from the
  project. A test scans the workspace's own sources for it, so it cannot be
  added quietly — including as a "deferred" item.

## Assumptions

1. The operating system enforces user separation. The feature stores a
   reminder's text with DPAPI, so another user cannot read it, but nothing else
   here defends against a program that already runs as this user.
2. The application's own process is not compromised. A debugger attached to it,
   or a modified binary, defeats every rule in this document.
3. The local model server is the managed one on loopback. A model server the
   user points elsewhere is still only *offered* the catalogue, so the blast
   radius stays the same — but its answers are no longer something this project
   can reason about, which is why the capability is probed rather than assumed.
4. The interface is not trusted with secrets. It never receives one: no key, no
   path from a request, no reminder text it did not type.

## Test evidence

| Claim | Test |
|---|---|
| No command line, shell, or PowerShell in the feature | `windows_actions_isolation::no_source_of_the_feature_can_run_a_command_line` |
| No process termination and no input synthesis | `windows_actions_isolation::no_source_of_the_feature_can_end_a_process_or_type_for_the_user` |
| No vault, notes, memory, or key access | `windows_actions_isolation::no_source_of_the_feature_reaches_the_encrypted_storages_or_a_key` |
| No file deletion outside the log's own rotation | `windows_actions_isolation::only_the_audit_log_removes_a_file` |
| Wake-on-LAN absent everywhere | `windows_actions_isolation::wake_on_lan_is_absent_from_the_whole_workspace` |
| The log cannot carry content | `windows_actions_isolation::the_log_cannot_carry_a_title_a_path_or_a_text` |
| The interface cannot supply a path or a risk | `windows_actions_isolation::the_command_surface_never_takes_a_path_or_a_risk_level_from_the_interface` |
| The core and the interface agree on every field | `windows_actions_contract` |
| Every message exists in all three locales | `windows_actions_contract::every_locale_translates_every_message_the_dialog_and_the_panel_use` |
| A token is single-use, expiring, and constant-time compared | `safety::tests` |
| A stale window identifier is refused | `executor::tests::a_window_identifier_is_short_lived`, `session::tests::a_stale_window_identifier_cannot_be_confirmed_later` |
| A sensitive window blocks a capture | `executor::tests`, `session::tests::a_window_capture_of_a_sensitive_window_is_refused_through_the_session` |
| A changed executable is refused until re-accepted | `allowlist::tests` |
| A reminder fires once and is logged before it is announced | `timers::tests`, `session::tests::a_fired_timer_reaches_the_host_hook_and_the_audit` |
| The model is offered only strict schemas | `tools::tests` |
| A tool call is refused for an extra or a forbidden field | `tools::tests::a_command_like_field_is_refused_as_forbidden`, `an_extra_field_is_refused` |

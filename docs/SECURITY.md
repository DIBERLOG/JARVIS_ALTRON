# Security baseline

## Local command execution

TOML CLI commands are launched as an executable plus separate arguments. The
executor no longer invokes `cmd /C` or `sh -c`, so command arguments are not
reparsed as shell syntax. Existing Lua and AutoHotkey command packs remain a
trusted local extension surface and require a separate hardening audit before
third-party packs are accepted.

Commands use `safe`, `confirmation_required`, or `forbidden` risk levels. A
confirmation expires after 15 seconds and releases only the stored command ID.
Voice confirmation is not authentication, so destructive and elevated actions
need a future GUI confirmation flow. `browser_close` is currently `forbidden`:
its AutoHotkey implementation elevates, closes applications, and restarts
JARVIS.

## Planned GUI confirmation contract

The confirmation dialog will show the action name, target program, normalized
arguments, consequence, and an expiry countdown, with **Cancel** and
**Confirm** controls. Confirming must release an immutable prepared-action ID;
the UI must not accept a replacement executable or argument list. Voice is not
an acceptable confirmation channel for application termination, elevation,
reboot/shutdown, file deletion, system-settings changes, passwords, or data
transmission.

## AI boundary

AI providers are chat-only. They must not receive a shell, arbitrary code
runner, clipboard contents, files, notes, Vault data, passwords, API keys, or
system diagnostics without an explicit feature-specific user request. A future
tool call must be schema-validated, allow-listed, and passed through SafetyGate.

## Known risks

`Cargo.toml` declares GPL-3.0-only while `LICENSE.txt` and README describe
CC-BY-NC-SA-4.0. This licensing conflict must be resolved by the copyright
holder before distribution. It was recorded, not changed.

# Lua `jarvis.system.exec` audit

## Current implementation

`crates/jarvis-core/src/lua/api/system.rs` registers `jarvis.system.exec` only
when a command uses the `full` Lua sandbox. It constructs `cmd /C <string>` on
Windows and `sh -c <string>` elsewhere. The supplied command string is shell
syntax, and optional Lua arguments are appended afterwards.

## Reachability today

The only route to a Lua script is a locally parsed TOML command selected by the
existing Vosk/intent router. The command context exposes the recognized phrase
and extracted slots to Lua. Consequently, a locally installed `full` command
could pass user-controlled phrase or slots to `jarvis.system.exec` unless the
script validates them.

No current command pack declares `sandbox = "full"`, and no bundled Lua script
calls `jarvis.system.exec`. Existing bundles use `standard`; standard does not
register this API. The current `ChatProvider` has no tool-call API and no path
to Lua, so it cannot invoke this function directly or indirectly.

## Logging

The executor logs command IDs, sandbox level, and errors. The Lua system API
does not currently log the `exec` command itself, but logs may still include a
script error containing user-controlled text. Secrets must therefore never be
passed to Lua contexts or shell errors.

## Required boundary

```
AIProvider / future tool calling
  -> typed tool schema
  -> argument validation and allow-list
  -> SafetyGate
  -> approved executable plus separate arguments
```

`jarvis.system.exec` is legacy and must remain unavailable to AI tools. A
compatible hardening change needs a command-pack trust policy plus explicit
user opt-in for `full` sandbox; neither currently exists. Until that policy is
designed, do not add `full` sandbox command packs and do not route AI output to
Lua.

# ADR: local AI runtime

## Status

Accepted for the first local-AI implementation.

## Decision

JARVIS_ALTRON keeps three independently replaceable paths:

| Concern | Initial implementation | Boundary |
| --- | --- | --- |
| Short fixed commands | Existing Vosk + intent registry | `CommandRecognizer` (future) |
| Long dictation and audio files | `whisper.cpp` | `DictationService` (future) |
| Chat and text analysis | `llama-server` / Qwen GGUF | `jarvis_core::ai::ChatProvider` |

The desktop provider will launch `llama-server` as a child process and call its
OpenAI-compatible HTTP API through `http://127.0.0.1:<configured-port>/v1`.
It must bind only to `127.0.0.1` by default. No model endpoint is published to
the LAN or the Internet automatically.

Windows defaults are Qwen3-8B Q4_K_M and an 8192-token context. Android
defaults are Qwen3-4B Q4_K_M (or compatible Q4) and a 2048–4096-token
context. GGUF and Whisper model files are user-managed local data: they are
excluded from Git, Git LFS, and releases.

## Command safety

The language model cannot execute a process. If it later proposes a tool call,
the application will parse a typed request, validate its schema and arguments,
look up a local allow-listed command ID, then use `SafetyGate`. It must never
pass model text to CMD, PowerShell, a shell, Lua, or AutoHotkey.

Existing command packs now carry an explicit `risk_level`. `safe` executes
immediately, `confirmation_required` creates one pending command ID for 15
seconds, and `forbidden` is rejected. Confirmation therefore cannot be used to
smuggle a second, arbitrary command into the executor.

## Profiles and data boundaries

JARVIS and ALTRON use the same local model but different system prompts,
generation settings, conversations, voice settings, and memory namespaces.
The profile changes style only; it does not change SafetyGate or available
system actions. Thinking is off by default and is a configured generation mode,
not a permission.

Passwords, tokens, keys, and Vault contents are never added to chat history,
AI memory, prompts, diagnostics, or the local model API. Notes are separate
from AI memory and can be supplied only by an explicit user action.

## Audio ownership

Vosk and Whisper must not concurrently read the microphone. A future
`AudioSessionManager` owns the microphone lease and exposes mutually exclusive
`Command`, `Dictation`, and `AudioTranscription` modes. Audio-file transcription
does not need a microphone lease.

## Android remote mode

The Android provider can be local llama.cpp or a private connection to the
user's PC. A remote mode requires a private network such as Tailscale or
WireGuard plus client authentication; plaintext public-port access is not an
option. Sync remains a separate encrypted-data protocol and is not an AI API.

## Consequences

The next implementation step adds a configuration-backed llama.cpp process
manager, model-file validation (including SHA-256), loopback health checks,
stream cancellation, and shutdown cleanup. It must report missing models and
insufficient RAM before starting the process. whisper.cpp, model downloads,
and Android packaging are not implemented by this ADR.

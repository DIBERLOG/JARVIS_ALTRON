# The «Команды» page

The page answers one question honestly: **what can I say, and why did that
phrase do nothing?** It replaced a `[404] раздел в разработке` notice and an
animated picture, which answered neither.

## What the page is built from

`crates/jarvis-core/src/commands/catalog.rs` reads the installed packs with the
loader's own parser — `parse_command_document`, the same function
`commands::parse_commands` uses — and turns each command into a card:

| Field | Where it comes from |
| --- | --- |
| `id` | the command's identifier, the one the matcher answers with |
| `pack` | the **logical name** of the pack: the name of its directory |
| `category` | a table over the pack name (`category_of`) |
| `phrases` | the pack's phrases for the language of the window, with the same fallback the pack uses |
| `slots` | the names and entities the command declares |
| `risk_level` | `jarvis_core::safety::RiskLevel`, read from the pack |
| `requires_confirmation` | `RiskLevel::ConfirmationRequired` |
| `enabled`, `unavailable_reason` | whether the command can run as it is, and why not |

Categories: `applications`, `sound`, `windows`, `screenshots`, `timers`,
`system`, `weather`, `global_voice_input`. The filter always offers all eight: a
category with no card yet is an empty state, not an absent category. The global
voice input is listed from the settings, where its phrase really lives, and is
marked `source: settings` — it is not a pack and it is not executed by the
command list.

## What never crosses the boundary

No path, no executable, no script, no argument and no secret. A pack is named by
its logical name; the executable's presence is checked inside the core and only
the answer crosses. The page stores nothing: no browser storage, no cookie, no
URL, no console. The only thing it can send is a phrase to check, and the voice
host compares it with the same `check_phrase` the microphone goes through and
forgets it.

## Packs the loader does not read

The loader reads `command.toml` only. Every other directory is listed under
**«Наборы, которые загрузчик не прочитал»** with its logical name and a reason
code — `unsupported_format` (`command.yaml`, which the loader does not open),
`parse_failed`, `missing_document`, `unreadable` — instead of being silently
absent. This is the first thing to look at when a phrase a pack promises does
nothing: if the pack is on that list, its commands are not loaded and no phrase
in them can match.

On this checkout, `resources/commands` holds 11 packs: `browser`, `counter`,
`test_slots` and `weather` ship `command.toml` and therefore appear as cards;
`calculator`, `jarvis`, `steam`, `stop`, `terminate`, `volume` and `windows`
ship `command.yaml` and appear on the unreadable list. Whether to teach the
loader the older format is a separate decision, and it is not made here — the
page reports the state instead of hiding it.

A command that cannot run says so: `no_phrases` (voice cannot reach it),
`executable_missing`, `script_missing`, `unsupported_type`, and
`disabled_in_settings` for the global voice input.

## What is not verified

Nothing on this page has been run on Windows by hand: no desktop session was
available. The catalogue, the DTO and the filters are covered by tests in
`commands::catalog::tests` and `frontend/tests/commands-page.test.mjs`, and the
window is type-checked, but a person still has to open the page and read it once.

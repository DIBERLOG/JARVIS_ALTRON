# Command packs, executors and readiness

A command pack is a document the user installs. It names phrases a person says and
one executor that answers them. It cannot name a program of its own, a path, or a
command line — the schema has no field for any of those.

## The four questions, kept apart

For a command to work, four separate things have to be true, and this project
refuses to blur them together:

| Question | Where it is answered |
| --- | --- |
| **MATCHED** — a phrase reaches this command | `commands::check_phrase` / `fetch_command`, the single matcher the listener and the settings button both use |
| **EXECUTABLE** — the executor exists in this build | the schema: `native` and `internal` are closed sets; `ahk` and `lua` need their file next to the pack |
| **ALLOWED** — the safety gate permits it | `JCommand.risk_level` plus, for typed actions, the Windows-action policy table and the user's allowlist |
| **EXECUTED** — the executor returned a confirmed result | the executor itself, reported as a stage in the diagnostics |

A card in the catalogue is not proof of any of them. The page shows which one is
unanswered.

## Executors

| Type | What it is | Can it start a program? |
| --- | --- | --- |
| `native` | one of `NativeAction` — get/set/change/mute volume, launch an allowed application by **role**, screenshot, list windows, lock the workstation | only through the user's allowlist, by role, never by path |
| `internal` | one of `InternalEvent` — end the chain, pause the listener | no |
| `voice` | plays one of its sounds | no |
| `terminate` | ends the assistant process | no |
| `stop_chaining` | ends the current chain (legacy spelling of the same event) | no |
| `lua` | a script next to the pack, in the sandbox | the script decides, inside its sandbox |
| `ahk` | a compiled helper next to the pack | yes, that helper, and only that file |
| `cli` | an executable and its fixed arguments | yes — which is why every `cli` entry in this repository is either refused by policy or reviewed as a safe, non-interpreter program |

There is no shell anywhere: `no_pack_can_become_a_shell` refuses an interpreter
(`cmd`, `powershell`, `pwsh`, `sh`, `bash`, `wscript`, `cscript`), a `-c`/`/c`
switch, and an argument that looks like a command line.

## The migration

Seven packs used to ship `command.yaml`, which the loader has never read. Their
commands did not exist at run time: the page listed the pack as
`unsupported_format` and no phrase in it could match. All seven are now
`command.toml`, parsed by the production parser, and the old YAML files are gone.
Intention was preserved; safety was raised where the old entry relied on a
compiled helper that force-closed or forced-killed something.

| pack | command_id | format | executor | risk | matched | executable | allowed | tested | migration_decision |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| browser | browser_open | toml | native `launch_application(role=browser)` | safe | yes | needs an allowed browser | policy table | yes | ahk helper → typed action by role |
| browser | browser_close | toml | ahk (helper not in the repository) | forbidden | yes | no — `executor_missing` | refused by policy | yes | kept, and forbidden: force-closing browsers is not something a phrase should do |
| browser | open_google | toml | ahk `Run website.exe` | safe | yes | no — `executor_missing` | — | yes | kept: this build has no typed "open a URL" action, and inventing one is a separate decision |
| calculator | calculator_open | toml | native `launch_application(role=calculator)` | safe | yes | needs an allowed calculator | policy table | yes | `cli calc` → typed action by role |
| calculator | calculator_close | toml | cli `taskkill /f /im CalculatorApp.exe` | forbidden | yes | yes | refused by policy | yes | kept forbidden, as the old pack intended |
| counter | counter | toml | lua `script.lua` | safe | yes | yes | yes | yes | untouched |
| jarvis | jarvis_thanks | toml | voice (`thanks`) | safe | yes | yes | yes | yes | `voice` unchanged |
| jarvis | jarvis_joke | toml | voice (`joke1..5`) | safe | yes | yes | yes | yes | `voice` unchanged |
| jarvis | jarvis_insult | toml | voice (`stupid`) | safe | yes | yes | yes | yes | `voice` unchanged |
| jarvis | jarvis_reboot | toml | cli `shutdown /r /t 0` | forbidden | yes | yes | refused by policy | yes | ahk `reboot.exe` → the command it really is, forbidden |
| steam | steam_open | toml | native `launch_application(role=steam)` | safe | yes | needs an allowed Steam | policy table | yes | ahk helper → typed action by role |
| steam | steam_close | toml | ahk (helper not in the repository) | safe | yes | no — `executor_missing` | — | yes | kept: no typed "close that window" action from a pack |
| stop | stop_listening | toml | internal `stop_chaining` | safe | yes | yes | yes | yes | `stop_chaining` action → typed event; `отмена` removed (it answers a confirmation elsewhere) |
| terminate | terminate | toml | terminate | confirmation_required | yes | yes | asks for a spoken confirmation | yes | risk raised from unset to `confirmation_required` |
| test_slots | test_greet_name | toml | lua `greet.lua` | safe | yes | yes | yes | yes | untouched |
| volume | volume_get | toml | native `get_volume` | safe | yes | yes | yes | yes | new entry for an action this build already had |
| volume | volume_mute | toml | native `mute_volume(true)` | safe | yes | yes | yes | yes | ahk `Mute volume.exe` → typed action |
| volume | volume_unmute | toml | native `mute_volume(false)` | safe | yes | yes | yes | yes | ahk `Mute volume.exe` → typed action |
| volume | volume_min | toml | native `set_volume(25)` | safe | yes | yes | yes | yes | ahk `Set sound.exe 25` → typed action |
| volume | volume_mid | toml | native `set_volume(50)` | safe | yes | yes | yes | yes | ahk `Set sound.exe 50` → typed action |
| volume | volume_max | toml | native `set_volume(100)` | safe | yes | yes | yes | yes | ahk `Set sound.exe 100` → typed action |
| weather | weather | toml | lua `script.lua` | safe | yes | yes | yes | yes | untouched |
| weather | set_city | toml | lua `set_city.lua` | safe | yes | yes | yes | yes | untouched |
| windows | windows_screenshot | toml | native `take_screenshot` | safe | yes | yes | policy table (and the sensitive-window refusal) | yes | ahk `screenshot.exe` → typed action |
| windows | windows_lock | toml | native `lock_workstation` | safe | yes | yes | policy table | yes | ahk `blocking.exe` → typed action |
| windows | windows_list | toml | native `list_windows` | safe | yes | yes | yes | yes | new, same action the voice router already used |
| windows | windows_task_manager | toml | native `launch_application(role=task_manager)` | confirmation_required | yes | needs an allowed task manager | policy table | yes | ahk `Task manager open.exe` → typed action by role |
| windows | windows_minimize_all | toml | ahk (helper not in the repository) | safe | yes | no — `executor_missing` | — | yes | kept: no typed "minimize everything" action |
| windows | windows_empty_trash | toml | ahk (helper not in the repository) | confirmation_required | yes | no — `executor_missing` | — | yes | kept, and raised to a confirmation: it destroys files |
| windows | windows_sleep | toml | ahk (helper not in the repository) | confirmation_required | yes | no — `executor_missing` | — | yes | kept, raised to a confirmation |
| windows | windows_clipboard | toml | ahk (helper not in the repository) | safe | yes | no — `executor_missing` | — | yes | kept |
| windows | windows_keyboard_layout | toml | ahk (helper not in the repository) | safe | yes | no — `executor_missing` | — | yes | kept |

`executor_missing` is not a failure of this build: it is the honest answer that the
compiled helper is not next to the pack. Compiling those `.ahk` sources, or giving
one of them a typed action, is a separate decision, and until then the page says
which commands cannot run and why.

## Statuses on a card

`ready`, `configuration_required` (a role launch waiting for the user's allowlist),
`disabled` (no phrases), `forbidden` (refused by policy), `executor_missing`,
`dependency_missing` (a script or helper the command needs is not there), and
`unsupported_format` for a pack the loader did not read — which is now empty.

## What is not verified

Nothing in this document has been run on a Windows desktop from here: `native`
actions, the allowlist resolution and the screenshot path are covered by tests
against the real policy and a stand-in pipeline, but no microphone, no window and
no Core Audio device was touched in this environment.

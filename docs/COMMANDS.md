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
| browser | browser_close | toml | native `close_application_windows(role=browser)` | confirmation_required | yes | yes | yes, after a spoken confirmation | yes | ahk force-close → typed graceful close, on the owner's decision |
| browser | open_google | toml | ahk `Run website.exe` | safe | yes | no — `executor_missing` | — | yes | kept: this build has no typed "open a URL" action, and inventing one is a separate decision |
| calculator | calculator_open | toml | native `launch_application(role=calculator)` | safe | yes | needs an allowed calculator | policy table | yes | `cli calc` → typed action by role |
| calculator | calculator_close | toml | native `close_application_windows(role=calculator)` | confirmation_required | yes | yes | yes, after a spoken confirmation | yes | `taskkill /f` → typed graceful close; no process is killed |
| counter | counter | toml | lua `script.lua` | safe | yes | yes | yes | yes | untouched |
| jarvis | jarvis_thanks | toml | voice (`thanks`) | safe | yes | yes | yes | yes | `voice` unchanged |
| jarvis | jarvis_joke | toml | voice (`joke1..5`) | safe | yes | yes | yes | yes | `voice` unchanged |
| jarvis | jarvis_insult | toml | voice (`stupid`) | safe | yes | yes | yes | yes | `voice` unchanged |
| jarvis | jarvis_reboot | toml | cli `shutdown /r /t 0` | confirmation_required | yes | yes | yes, after a spoken confirmation | yes | ahk `reboot.exe` → the command it really is, and it asks first |
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

Exactly one per command, mutually exclusive, adding up to the total:
`ready`, `confirmation_required`, `allowlist_required`, `forbidden`,
`executor_missing`, `disabled`. The precedence is documented in
`commands::catalog::status_of` and the counts are pinned by
`the_statuses_are_mutually_exclusive_and_add_up_to_the_total`.

On this checkout, for the 32 installed commands:

| Status | Count | Which |
| --- | --- | --- |
| `ready` | 17 | the six volume commands, `windows_screenshot`, `windows_lock`, `windows_list`, `stop_listening`, the three `voice` replies of the jarvis pack, `counter`, `weather`, `set_city`, `test_greet_name` |
| `confirmation_required` | 4 | `terminate`, `jarvis_reboot`, `calculator_close`, `browser_close` |
| `allowlist_required` | 4 | `browser_open`, `calculator_open`, `steam_open`, `windows_task_manager` |
| `forbidden` | 0 | nothing in the installed packs |
| `executor_missing` | 7 | `open_google`, `steam_close`, `windows_minimize_all`, `windows_empty_trash`, `windows_sleep`, `windows_clipboard`, `windows_keyboard_layout` |
| `disabled` | 0 | — |
| **total** | **32** | |

The 4 in `allowlist_required` need one thing from the user: the application, added
through Настройки → «Windows Actions» → «Приложения», where the native file dialog
picks the program (the interface never names a path). The role decides which file
counts: `browser` → `chrome.exe`, `firefox.exe`, `msedge.exe`, `brave.exe`,
`opera.exe`, `vivaldi.exe`, `browser.exe`; `calculator` → `calc.exe`,
`calculator.exe`, `calculatorapp.exe`; `steam` → `steam.exe`; `task_manager` →
`taskmgr.exe`. Two matching entries make the role ambiguous and the command refuses
rather than guessing; a file that changed since it was allowed must be re-accepted.

The 3 that used to be `forbidden` are now `confirmation_required`, on the owner's
explicit decision, and the way they run is the safe one: `jarvis_reboot` is
`shutdown /r /t 0` with fixed arguments and no shell, gated by a spoken
«подтверждаю»; `calculator_close` and `browser_close` are the typed
`close_application_windows` action, which posts the same graceful close the
interface's window list posts — the window is asked to close, no process is killed,
and only a window that exists right now and whose process matches the role can be
chosen. `nothing_in_a_pack_kills_a_process` refuses `taskkill`, `tskill`, `pskill`,
`wmic` and any `/f` argument in any pack.

## What is not verified

Nothing in this document has been run on a Windows desktop from here: `native`
actions, the allowlist resolution and the screenshot path are covered by tests
against the real policy and a stand-in pipeline, but no microphone, no window and
no Core Audio device was touched in this environment.

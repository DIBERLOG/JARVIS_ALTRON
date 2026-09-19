# The «Команды» page

The page answers one question honestly: **what can I say, and why did that
phrase do nothing?** It replaced a `[404] раздел в разработке` notice and an
animated picture, which answered neither.

It has three tabs: **Команды**, **Проверка фразы** and **Диагностика**.

## Layout

The cards are a grid, not one long row: three to four on a wide window, two in the
middle, one when the window is narrow (`repeat(auto-fill, minmax(17rem, 1fr))` with
breakpoints at 1100 px and 700 px). A card never stretches its text across the whole
window, a long phrase wraps instead of breaking the grid, and the font sizes are the
page's own — a card is readable, not a label.

Each card shows the **name in the language of the page** (the first phrase a person
would say), the `command_id`, the pack, the category (a coloured dot with a title),
the status, the risk level, whether a confirmation is needed and how many phrases
there are — and, separately, the four questions:

| Indicator | Meaning |
| --- | --- |
| **Фраза распознаётся** | a phrase reaches this command (`recognized`) |
| **Исполнитель готов** | the executor exists in this build (`executor_ready`) — a fact about the build and the files next to the pack, blind to the policy and to the configuration |
| **Разрешено политикой** | not forbidden by the policy (`allowed`) |
| **Проверено** | the command would really run: recognised, executable, allowed and nothing waiting for configuration (`verified`) |

The four are deliberately independent, and the page is the place where that shows: a
forbidden command whose program is present reports the executor as *ready* and the
policy as *not allowed*, while a launch that waits for the allowlist reports the
executor as *ready* and its status as *not configured*. Collapsing those into one
"not ready" is what makes such a page useless for finding out *why*.

A card expands to show its description, the phrases in the current language, its
slots, its source, and the reason it cannot run. The block of packs the loader did
not read is behind its own toggle and starts collapsed — after the migration it is
empty, and a test fails the suite if a `command.yaml` ever comes back.

The filters are a search (a phrase, an identifier, a pack name or a slot name),
the categories, the statuses and the risk levels, with a refresh button. The empty
states are the page's own words, not a blank area.

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
| `enabled`, `status`, `recognized`, `executor_ready`, `allowed`, `verified`, `unavailable_reason` | whether the phrase reaches it, whether its executor exists, and why not |

Categories: `applications`, `sound`, `windows`, `screenshots`, `timers`,
`system`, `weather`, `global_voice_input`. The filter always offers all eight: a
category with no card yet is an empty state, not an absent category. The global
voice input is listed from the settings, where its phrase really lives, and is
marked `source: settings` — it is not a pack and it is not executed by the
command list.

Statuses — exactly one per command, mutually exclusive, adding up to the total:
`ready`, `confirmation_required`, `allowlist_required`, `forbidden`,
`executor_missing`, `disabled`. The `unavailable_reason` on the card carries the
technical detail (`no_phrases`, `executable_missing`, `script_missing`,
`unsupported_type`, `allowlist_required`, `disabled_in_settings`,
`forbidden_by_policy`), and `docs/COMMANDS.md` has the full inventory of packs,
executors and decisions. The precedence, and the one documented overlap, are
described in `commands::catalog::status_of`: a command that is both forbidden and
missing its helper is reported as **forbidden**, and the card still shows the
executor as missing in its own indicator, because there are two true answers there
and hiding one would be worse.

## What never crosses the boundary

No path, no executable, no script, no argument and no secret. A pack is named by
its logical name; the executable's presence is checked inside the core and only
the answer crosses. A test serializes the whole catalogue and refuses the answer if
it contains the runtime directory or the word `resources`. The page stores nothing:
no browser storage, no cookie, no URL, no console. The only thing it can send is a
phrase to check, and the voice host compares it with the same `check_phrase` the
microphone goes through and forgets it.

## Runtime resources

`APP_DIR` is the directory of the running executable, and packs are read from
`resources/commands` next to it — the same place in a debug build (`cargo run`,
`target/debug/jarvis-gui.exe`, `target/debug/jarvis-app.exe`), in a direct run of
the built binary, and in a release bundle, where the Tauri configuration copies
`resources/commands` into the bundle. No pack path is absolute and none depends on
a developer's checkout; `the_runtime_layout_is_what_is_read` lays out a temporary
runtime directory and reads it to prove that.

One caveat on a developer machine: `target/debug/resources` is a copy, and a file
deleted from the repository stays in the copy until the directory is removed. A
stale `command.yaml` next to a new `command.toml` changes nothing — the loader
reads `command.toml`, and the page only reports a pack as unreadable when the
document it reads is absent.

## Window chrome and the way the window appears

The white top bar is the native title bar drawn in the light theme. It is answered
by the supported mechanism rather than by a custom title bar: the Tauri window
configuration asks for `"theme": "Dark"`, so Windows draws the native bar dark, and
the document itself is dark so the first paint does not flash white. A custom title
bar would have to reimplement dragging, DPI scaling and the system menu, and none of
that can be verified from here, so it was not done.

The default window is 1180×820 and resizable (minimum 420×560): the grid needs a
window wide enough for three or four cards, and a person may size it.

The shell fades in over 200 ms with a small upward movement, in CSS, with no
library, and it is switched off entirely under `prefers-reduced-motion: reduce`. It
is decoration on a window that already works: it adds no delay and hides no error.

Closing the window and leaving the application stay two different actions, and both
already went through one place (`desktop::on_close_requested`): hide to the tray,
ask, or a full exit — and the full exit stops the managed voice host while hiding
does not.

## What is not verified

Nothing on this page has been run on Windows by hand: no desktop session was
available. The catalogue, the DTO, the filters, the grid, the animation and the
window configuration are covered by tests in `commands::catalog::tests` and
`frontend/tests/commands-page.test.mjs`, and the window is type-checked, but a
person still has to open it once and look at the title bar, the grid and the fade.

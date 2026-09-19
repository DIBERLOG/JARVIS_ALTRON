import { test } from "node:test"
import assert from "node:assert/strict"
import { existsSync, readFileSync } from "node:fs"
import { fileURLToPath } from "node:url"

import {
    RISKS,
    STATUSES,
    categoryKey,
    filterEntries,
    foldForSearch,
    matchesQuery,
    packReasonKey,
    riskKey,
    statusKey,
    titleOf,
    unavailableKey
} from "../src/lib/command-catalog.ts"

/**
 * The «Команды» page, checked where a static check can reach it:
 *
 * * the page exists and the placeholder it replaced is gone — the `[404]` notice,
 *   the animated picture and their messages;
 * * the catalogue is built by the loader's own parser in the core, from the packs
 *   the voice host loads, and its answer carries no path, no executable and no
 *   argument;
 * * a pack the loader does not read is listed with a reason instead of being left
 *   out;
 * * the page can execute nothing and stores nothing;
 * * the filter helpers behave, and every key the page and the codes need exist in
 *   all three locales.
 */

const PAGE = fileURLToPath(new URL("../src/routes/commands/index.svelte", import.meta.url))
const SHELL = fileURLToPath(new URL("../src/routes/_module.svelte", import.meta.url))
const TAURI_CONFIG = fileURLToPath(new URL("../../crates/jarvis-gui/tauri.conf.json", import.meta.url))
const DESKTOP = fileURLToPath(new URL("../../crates/jarvis-gui/src/desktop.rs", import.meta.url))
const API = fileURLToPath(new URL("../src/lib/command-catalog.ts", import.meta.url))
const CORE_CATALOG = fileURLToPath(new URL("../../crates/jarvis-core/src/commands/catalog.rs", import.meta.url))
const CORE_COMMANDS = fileURLToPath(new URL("../../crates/jarvis-core/src/commands.rs", import.meta.url))
const RUST_CHECKER = fileURLToPath(
    new URL("../../crates/jarvis-gui/src/tauri_commands/commands.rs", import.meta.url)
)
const RUST_MAIN = fileURLToPath(new URL("../../crates/jarvis-gui/src/main.rs", import.meta.url))
const LOCALES = ["en", "ru", "ua"]

function localeKeys(language) {
    const path = fileURLToPath(
        new URL(`../../crates/jarvis-core/src/i18n/locales/${language}.ftl`, import.meta.url)
    )
    const keys = new Set()
    for (const line of readFileSync(path, "utf8").split("\n")) {
        const trimmed = line.trim()
        if (!trimmed || trimmed.startsWith("#") || trimmed.startsWith("-")) continue
        const match = /^([A-Za-z0-9_-]+)\s*=/.exec(trimmed)
        if (match) keys.add(match[1])
    }
    return keys
}

test("the placeholder page is gone and the real one is there", () => {
    assert.ok(existsSync(PAGE), "the commands page must exist")
    const page = readFileSync(PAGE, "utf8")
    for (const removed of ["commands-wip", "tenor.gif", "placeholder-image", "[404]"]) {
        assert.equal(page.includes(removed), false, `the page must not keep ${removed}`)
    }
    // The animated picture is not referenced by a message any more either.
    for (const language of LOCALES) {
        const available = localeKeys(language)
        for (const removed of [
            "commands-wip-title",
            "commands-wip-desc",
            "commands-wip-follow",
            "commands-wip-channel"
        ]) {
            assert.equal(available.has(removed), false, `${language}.ftl still has ${removed}`)
        }
    }
})

test("the catalogue comes from the loader's own parser and carries no path", () => {
    const catalog = readFileSync(CORE_CATALOG, "utf8")
    const commands = readFileSync(CORE_COMMANDS, "utf8")
    assert.ok(commands.includes("pub use catalog::*;"), "the catalogue must be reachable")
    assert.ok(catalog.includes("parse_command_document(&text)"), "it must use the loader's parser")
    assert.ok(catalog.includes('path.join("command.toml")'), "it must read what the loader reads")
    // The answer has no path, no executable and no argument as a field of its own:
    // the codes may mention that a script is missing, but the pack's script is not
    // carried.
    const start = catalog.indexOf("pub struct CatalogEntry")
    const end = catalog.indexOf("\n}", start)
    const fields = [...catalog.slice(start, end).matchAll(/pub ([a-z_]+):/g)].map(
        (match) => match[1]
    )
    assert.ok(fields.length >= 10, `expected the card fields, got ${fields.length}`)
    for (const forbidden of [
        "path",
        "pack_path",
        "exe_path",
        "exe_args",
        "cli_cmd",
        "cli_args",
        "script",
        "args",
        "sandbox"
    ]) {
        assert.equal(fields.includes(forbidden), false, `a card must not carry ${forbidden}`)
    }
    // The risk level and the confirmation flag are the core's own.
    assert.ok(catalog.slice(start, end).includes("risk_level: String"));
    assert.ok(catalog.slice(start, end).includes("requires_confirmation: bool"));
    assert.ok(catalog.includes("use crate::safety::RiskLevel"))
    assert.ok(catalog.includes("RiskLevel::ConfirmationRequired"))
    // The eight categories are the filter's vocabulary.
    for (const category of [
        "applications",
        "sound",
        "windows",
        "screenshots",
        "timers",
        "system",
        "weather",
        "global_voice_input"
    ]) {
        assert.ok(catalog.includes(`"${category}"`), `the filter must offer ${category}`)
    }
})

test("an unreadable pack is listed with a reason instead of being hidden", () => {
    const catalog = readFileSync(CORE_CATALOG, "utf8")
    assert.ok(catalog.includes('"unsupported_format"'), "a pack in another format is named")
    assert.ok(catalog.includes('"parse_failed"'), "a document that does not parse is named")
    assert.ok(catalog.includes('"missing_document"'), "a pack without a document is named")
    assert.ok(catalog.includes("pub struct UnreadablePack"))
    const start = catalog.indexOf("pub struct UnreadablePack")
    const end = catalog.indexOf("\n}", start)
    assert.equal(
        catalog.slice(start, end).includes("Path"),
        false,
        "an unreadable pack is named by its logical name"
    )
})

test("the page executes nothing and stores nothing", () => {
    const page = readFileSync(PAGE, "utf8")
    const api = readFileSync(API, "utf8")
    for (const forbidden of [
        "localStorage",
        "sessionStorage",
        "indexedDB",
        "document.cookie",
        "location.hash",
        "window.history",
        "console.log",
        "eval("
    ]) {
        assert.equal(page.includes(forbidden), false, `the page must not use ${forbidden}`)
    }
    for (const forbidden of ["localStorage", "sessionStorage", "console.log"]) {
        assert.equal(api.includes(forbidden), false, `the catalogue api must not use ${forbidden}`)
    }
    // The only thing the page sends is the catalogue read and the phrase check.
    const invoked = [...api.matchAll(/invoke<[^>]*>\("([a-z_]+)"/g)].map((match) => match[1])
    assert.deepEqual(invoked, ["command_catalog"])
    for (const key of ["commands-search", "commands-refresh", "commands-phrases", "commands-slots"]) {
        assert.ok(page.includes(key), `the page needs ${key}`)
    }
    assert.ok(page.includes("<PhraseCheck />"), "the page can check a phrase")
})

test("the catalogue command is registered and needs no second matcher", () => {
    const rust = readFileSync(RUST_CHECKER, "utf8")
    const main = readFileSync(RUST_MAIN, "utf8")
    assert.ok(rust.includes("pub fn command_catalog("), "the catalogue command must exist")
    assert.ok(main.includes("tauri_commands::command_catalog,"), "it must be registered")
    assert.ok(rust.includes("commands::load_catalog("), "it must use the core's loader")
    assert.ok(
        rust.includes("commands::global_voice_input_entry("),
        "the global voice input comes from the settings"
    )
})

test("the filter helpers behave", () => {
    const base = {
        id: "browser_open",
        pack: "browser",
        category: "applications",
        source: "pack",
        description: "",
        phrases: ["открой браузер"],
        slots: [],
        risk_level: "safe",
        requires_confirmation: false,
        enabled: true,
        status: "ready",
        recognized: true,
        executor_ready: true,
        allowed: true,
        verified: true,
        unavailable_reason: null
    }
    const entries = [
        base,
        {
            ...base,
            id: "weather",
            pack: "weather",
            category: "weather",
            phrases: ["какая погода в {city}"],
            slots: [{ name: "city", entity: "city" }]
        },
        {
            ...base,
            id: "open_google",
            pack: "browser",
            phrases: ["открой гугл"],
            risk_level: "forbidden",
            status: "forbidden",
            enabled: false,
            allowed: false,
            verified: false,
            unavailable_reason: "forbidden_by_policy"
        },
        {
            ...base,
            id: "windows_sleep",
            pack: "windows",
            phrases: ["спящий режим"],
            status: "executor_missing",
            enabled: false,
            executor_ready: false,
            verified: false,
            unavailable_reason: "executable_missing"
        }
    ]
    assert.equal(filterEntries(entries, "", "").length, 4)
    assert.equal(filterEntries(entries, "weather", "").length, 1)
    assert.deepEqual(
        filterEntries(entries, "", "открой").map((entry) => entry.id),
        ["browser_open", "open_google"]
    )
    // The three filters compose.
    assert.deepEqual(
        filterEntries(entries, "", "", "ready").map((entry) => entry.id),
        ["browser_open", "weather"]
    )
    assert.deepEqual(
        filterEntries(entries, "", "", "", "forbidden").map((entry) => entry.id),
        ["open_google"]
    )
    assert.deepEqual(
        filterEntries(entries, "browser", "", "executor_missing").map((entry) => entry.id),
        []
    )
    assert.deepEqual(
        filterEntries(entries, "", "спящий", "executor_missing", "safe").map((entry) => entry.id),
        ["windows_sleep"]
    )
    // A phrase, an identifier, a pack and a slot name are all searched.
    assert.ok(matchesQuery(entries[1], "погода"))
    assert.ok(matchesQuery(entries[1], "WEATHER"))
    assert.ok(matchesQuery(entries[1], "city"))
    assert.equal(matchesQuery(entries[0], "погода"), false)
    // `ё` and `е` are the same letter here, as they are for the matcher.
    assert.equal(foldForSearch("Королёве"), "королеве")
    assert.ok(matchesQuery({ ...entries[1], phrases: ["погода в Королёве"] }, "королеве"))

    // The card's name is the phrase a person would say in that language.
    assert.equal(titleOf(entries[0]), "открой браузер")
    assert.equal(titleOf({ ...entries[0], phrases: [] }), "browser_open")

    assert.equal(categoryKey("weather"), "command-category-weather")
    assert.equal(riskKey("confirm"), "command-risk-confirm")
    assert.equal(statusKey("executor_missing"), "command-status-executor_missing")
    assert.equal(unavailableKey("executable_missing"), "command-unavailable-executable_missing")
    assert.equal(unavailableKey(null), "command-unavailable-unknown")
    assert.equal(packReasonKey("unsupported_format"), "command-pack-reason-unsupported_format")
    assert.equal(STATUSES.length, 6)
    assert.deepEqual(RISKS, ["safe", "confirm", "forbidden"])
})

test("the page is a grid that stays readable, with tabs and filters", () => {
    const page = readFileSync(PAGE, "utf8")
    // Three to four on a wide window, two in the middle, one when narrow: a grid
    // with breakpoints, not a single row of cards.
    assert.ok(page.includes("grid-template-columns"), "the cards need a grid")
    assert.ok(page.includes("repeat(auto-fill, minmax"), "the grid must fill the width")
    assert.ok(page.includes("@media (max-width: 1100px)"), "two columns in the middle")
    assert.ok(page.includes("@media (max-width: 700px)"), "one column when narrow")
    assert.ok(page.includes("min-width: 0"), "a long phrase must not break the grid")
    assert.ok(page.includes("overflow-wrap: anywhere"), "a long word must wrap")
    // The three tabs.
    for (const tab of ["commands", "phrase", "diagnostics"]) {
        assert.ok(page.includes(`commands-tab-${tab}`), `the page needs the ${tab} tab`)
    }
    // Filters: category, status, risk, search, refresh.
    for (const key of [
        "commands-filter-category",
        "commands-filter-status",
        "commands-filter-risk",
        "commands-search",
        "commands-refresh"
    ]) {
        assert.ok(page.includes(key), `the page needs ${key}`)
    }
    // A card shows the four indicators separately.
    for (const indicator of [
        "command-indicator-recognized",
        "command-indicator-executable",
        "command-indicator-allowed",
        "command-indicator-verified"
    ]) {
        assert.ok(page.includes(indicator), `a card needs ${indicator}`)
    }
    // Cards expand, and the unsupported block is behind its own toggle.
    assert.ok(page.includes("commands-expand") && page.includes("commands-collapse"))
    assert.ok(page.includes("unreadableOpen"), "unsupported packs are collapsed by default")
    assert.ok(page.includes("catalog.unreadable.length > 0"), "the block is hidden when empty")
    // The four indicators come from the backend, not from a second guess.
    assert.ok(page.includes("entry.recognized"))
    assert.ok(page.includes("entry.executor_ready"))
    assert.ok(page.includes("entry.allowed"))
    assert.ok(page.includes("entry.verified"))
})

test("the shell appears softly without touching the backend", () => {
    const shell = readFileSync(SHELL, "utf8")
    assert.ok(shell.includes("@keyframes jarvis-appear"), "the shell must animate in")
    assert.ok(shell.includes("opacity: 0"), "it fades")
    assert.ok(shell.includes("translateY"), "it lifts")
    assert.ok(shell.includes("200ms"), "the animation is short")
    assert.ok(
        shell.includes("@media (prefers-reduced-motion: reduce)"),
        "reduced motion must switch the animation off"
    )
    // No library, no timer, no splash delay.
    for (const forbidden of ["setTimeout", "import(", "gsap", "anime", "svelte/transition"]) {
        assert.equal(shell.includes(forbidden), false, `the shell must not use ${forbidden}`)
    }
})

test("the window chrome is dark and the close behaviour is one place", () => {
    const config = JSON.parse(readFileSync(TAURI_CONFIG, "utf8"))
    const window = config.app.windows.find((entry) => entry.label === "main") ?? config.app.windows[0]
    // The white title bar is answered by the supported Windows mechanism: the window
    // asks for the dark theme, so Windows draws the native bar dark instead of a
    // custom title bar that would have to reimplement dragging and DPI.
    assert.equal(window.theme, "Dark", "the window must ask for the dark theme")
    assert.ok(window.width >= 1024, "the commands page needs a window wide enough for a grid")
    assert.equal(window.resizable, true, "a person may size the window")
    // And closing is one decision, not two: the tray, the dialog and the setting all
    // go through `on_close_requested`.
    const desktop = readFileSync(DESKTOP, "utf8")
    assert.ok(desktop.includes("enum CloseBehavior") || desktop.includes("CloseBehavior::Tray"))
    assert.ok(desktop.includes("CloseBehavior::Exit"), "a full exit exists and is separate")
    assert.ok(desktop.includes("fn request_exit"), "exit stops the managed voice host")
})

test("every commands page key and code exists in all three locales", () => {
    const page = readFileSync(PAGE, "utf8")
    const api = readFileSync(API, "utf8")
    const required = new Set()
    for (const source of [page, api]) {
        for (const match of source.matchAll(/["'`](commands?-[a-z0-9_-]+)["'`]/g)) {
            required.add(match[1])
        }
    }
    for (const category of [
        "applications",
        "sound",
        "windows",
        "screenshots",
        "timers",
        "system",
        "weather",
        "global_voice_input"
    ]) {
        required.add(categoryKey(category))
    }
    for (const risk of ["safe", "confirm", "forbidden"]) {
        required.add(riskKey(risk))
    }
    for (const reason of [
        "no_phrases",
        "executable_missing",
        "script_missing",
        "unsupported_type",
        "disabled_in_settings"
    ]) {
        required.add(unavailableKey(reason))
    }
    for (const reason of ["parse_failed", "unsupported_format", "missing_document", "unreadable"]) {
        required.add(packReasonKey(reason))
    }
    assert.ok(required.size > 30, `expected a substantial key set, got ${required.size}`)
    for (const language of LOCALES) {
        const available = localeKeys(language)
        const missing = [...required].filter((key) => !available.has(key)).sort()
        assert.deepEqual(missing, [], `${language}.ftl is missing a commands page message`)
    }
})

import { test } from "node:test"
import assert from "node:assert/strict"
import { readFileSync } from "node:fs"
import { fileURLToPath } from "node:url"

import {
    COMPONENT_STATES,
    COMPONENTS,
    ERROR_CODES,
    STAGES,
    STEPS,
    UNKNOWN_LABEL_KEY,
    WARNING_CODES,
    componentLabelKey,
    errorLabelKey,
    originLabelKey,
    stageLabelKey,
    stateLabelKey,
    stepLabelKey,
    warningLabelKey
} from "../src/lib/local-ai-setup-model.ts"

const LOCALES = ["en", "ru", "ua"]
const WIZARD_FILE = fileURLToPath(
    new URL("../src/components/ai/LocalAiSetupWizard.svelte", import.meta.url)
)
const MODEL_FILE = fileURLToPath(new URL("../src/lib/local-ai-setup-model.ts", import.meta.url))
const API_FILE = fileURLToPath(new URL("../src/lib/local-ai-setup.ts", import.meta.url))

function localePath(language) {
    return fileURLToPath(
        new URL(`../../crates/jarvis-core/src/i18n/locales/${language}.ftl`, import.meta.url)
    )
}

/**
 * The messages a locale defines.
 *
 * Fluent keys here are all of the form `key = value`, and a key is allowed to
 * contain an underscore — the stage and error codes are full of them — so this
 * parser follows the file format rather than the narrower shape one of the
 * older suites happened to use.
 */
function messages(language) {
    const source = readFileSync(localePath(language), "utf8")
    const entries = new Map()
    for (const line of source.split("\n")) {
        const trimmed = line.trim()
        if (!trimmed || trimmed.startsWith("#") || trimmed.startsWith("-")) continue
        const match = /^([A-Za-z0-9_-]+)\s*=(.*)$/.exec(trimmed)
        if (!match) continue
        entries.set(match[1], match[2].trim())
    }
    return entries
}

/**
 * Removes comments so documented prohibitions are not mistaken for usage.
 *
 * Markup comments are removed first: in Svelte a comment is text, and the
 * `--` of its closing marker is not a command-line flag.
 */
function stripComments(source) {
    const withoutMarkup = source.replace(/<!--[\s\S]*?-->/g, "")
    let out = ""
    let index = 0
    while (index < withoutMarkup.length) {
        const pair = withoutMarkup.slice(index, index + 2)
        if (pair === "//") {
            const end = withoutMarkup.indexOf("\n", index)
            index = end === -1 ? withoutMarkup.length : end
            continue
        }
        if (pair === "/*") {
            const end = withoutMarkup.indexOf("*/", index + 2)
            index = end === -1 ? withoutMarkup.length : end + 2
            continue
        }
        out += withoutMarkup[index]
        index += 1
    }
    return out
}

function wizardSource() {
    return readFileSync(WIZARD_FILE, "utf8")
}

/** Literal `t('key')` usages in the wizard, the way the other suites collect them. */
function literalWizardKeys() {
    const keys = new Set()
    const code = stripComments(wizardSource())
    for (const match of code.matchAll(/\bt\(\s*['"]([a-z0-9_-]+)['"]/g)) {
        keys.add(match[1])
    }
    return keys
}

/** String literals the two TypeScript modules can hand to the panel. */
function moduleLiteralKeys() {
    const keys = new Set()
    for (const path of [MODEL_FILE, API_FILE]) {
        const code = stripComments(readFileSync(path, "utf8"))
        for (const match of code.matchAll(/['"](ai-setup-[a-z0-9_-]+)['"]/g)) {
            keys.add(match[1])
        }
    }
    return keys
}

// ---------------------------------------------------------------- key coverage

test("every key the wizard and its modules can build exists in all locales", () => {
    const required = new Set([...literalWizardKeys(), ...moduleLiteralKeys()])
    assert.ok(required.size > 90, `expected a substantial key set, got ${required.size}`)
    for (const language of LOCALES) {
        const available = messages(language)
        const missing = [...required].filter((key) => !available.has(key)).sort()
        assert.deepEqual(missing, [], `${language}.ftl is missing local AI setup messages`)
    }
})

test("every code the panel renders has a message in all locales", () => {
    const required = new Map()
    for (const code of STAGES) required.set(`stage ${code}`, stageLabelKey(code))
    for (const code of STEPS) required.set(`step ${code}`, stepLabelKey(code))
    for (const code of COMPONENTS) required.set(`component ${code}`, componentLabelKey(code))
    for (const code of COMPONENT_STATES) required.set(`state ${code}`, stateLabelKey(code))
    for (const code of ERROR_CODES) required.set(`error ${code}`, errorLabelKey(code))
    for (const code of WARNING_CODES) required.set(`warning ${code}`, warningLabelKey(code))
    for (const origin of ["managed", "user_provided", "mixed", "unset"]) {
        required.set(`origin ${origin}`, originLabelKey(origin))
    }
    required.set("unknown", UNKNOWN_LABEL_KEY)

    // One message per code, which is what makes the fallback unnecessary here.
    const keys = [...required.values()]
    assert.equal(new Set(keys).size, keys.length, "two codes share one message")

    for (const language of LOCALES) {
        const available = messages(language)
        const missing = [...required.entries()]
            .filter(([, key]) => !available.has(key))
            .map(([label, key]) => `${label} -> ${key}`)
            .sort()
        assert.deepEqual(missing, [], `${language}.ftl is missing code messages`)
    }
})

test("the three locales define the same local AI setup key set", () => {
    const reference = [...messages("en").keys()].filter((key) => key.startsWith("ai-setup-")).sort()
    // The full code set plus the interface strings: a short list would mean a
    // locale lost messages, not that the interface uses fewer of them.
    assert.equal(reference.length, 149, `expected the full setup key set, got ${reference.length}`)
    for (const language of LOCALES.slice(1)) {
        const available = [...messages(language).keys()].filter((key) => key.startsWith("ai-setup-")).sort()
        assert.equal(
            available.length,
            reference.length,
            `${language}.ftl defines ${available.length} setup messages, en.ftl defines ${reference.length}`
        )
        const onlyInEnglish = reference.filter((key) => !available.includes(key))
        const onlyInLocale = available.filter((key) => !reference.includes(key))
        assert.deepEqual(onlyInEnglish, [], `${language}.ftl is missing setup messages`)
        assert.deepEqual(onlyInLocale, [], `${language}.ftl has setup messages English does not`)
    }
})

test("no local AI setup message is left empty", () => {
    for (const language of LOCALES) {
        const empty = [...messages(language).entries()]
            .filter(([key, value]) => key.startsWith("ai-setup-") && value.length === 0)
            .map(([key]) => key)
            .sort()
        assert.deepEqual(empty, [], `${language}.ftl has empty setup messages`)
    }
})

test("Russian and Ukrainian are translated, not copied from English", () => {
    const english = messages("en")
    // A human-readable message, not a proper noun or a bare code.
    const sample = [...english.entries()]
        .filter(([key, value]) => key.startsWith("ai-setup-") && value.includes(" ") && value.length > 12)
        .map(([key]) => key)
    assert.ok(sample.length >= 20, `expected a readable sample, got ${sample.length}`)
    for (const language of LOCALES.slice(1)) {
        const translated = messages(language)
        const copied = sample.filter((key) => translated.get(key) === english.get(key))
        assert.deepEqual(copied, [], `${language}.ftl repeats the English text`)
        const missing = sample.filter((key) => !translated.has(key))
        assert.deepEqual(missing, [], `${language}.ftl lost these messages`)
    }
})

// ------------------------------------------------------------------ source rules

/**
 * The three files this suite owns, with their comments removed.
 *
 * Concatenated, because a prohibition applies to the whole panel: a URL is as
 * forbidden in a comment-free helper as it is in the markup.
 */
function scannedSource() {
    return [wizardSource(), readFileSync(MODEL_FILE, "utf8"), readFileSync(API_FILE, "utf8")]
        .map(stripComments)
        .join("\n")
}

test("the panel never names an address, a checksum, or a file", () => {
    const code = scannedSource()
    const forbidden = [
        "http://",
        "https://",
        "sha256",
        "SHA256",
        ".exe",
        ".gguf",
        "://"
    ]
    for (const needle of forbidden) {
        assert.equal(
            code.includes(needle),
            false,
            `the setup panel must not contain ${needle}: the core owns every value it does not display`
        )
    }
    // A bare scheme cannot appear either, wherever the letters came from.
    assert.equal(/https?/i.test(code), false, "a URL scheme is the core's business")
    // No command-line flag: the panel never builds the command it launches.
    assert.equal(
        /(^|[\s'"(=])--[a-z][a-z0-9-]*/m.test(code),
        false,
        "the panel must not build a launch command"
    )
})

test("the wizard renders the steps and stages through the shared code lists", () => {
    const code = wizardSource()
    // The lists themselves, not a copy of them, decide what is rendered: the
    // step chips and the run list are both driven by these two arrays, and the
    // step list itself comes from the offer the core sent.
    for (const needle of ["STEPS", "STAGES"]) {
        assert.ok(
            new RegExp(`\\b${needle}\\b`).test(code),
            `the wizard must use the shared ${needle} list`
        )
    }
    assert.ok(
        code.includes("view.offer.steps"),
        "the wizard must render the step list the core sent"
    )
    assert.ok(
        code.includes("view.offer.stage_codes"),
        "the wizard must render the stage list the core sent"
    )

    // A message for each of the six steps and all sixteen stages, named in the
    // wizard's own label tables: a renamed code must fail this suite rather than
    // reach a person as a blank label. The tables are what the panel renders, so
    // this is the list that has to stay complete.
    const missing = [
        ...STEPS.map((step) => `step ${step} -> ${stepLabelKey(step)}`),
        ...STAGES.map((stage) => `stage ${stage} -> ${stageLabelKey(stage)}`)
    ].filter((entry) => !code.includes(entry.slice(entry.indexOf(">") + 2)))
    assert.deepEqual(missing, [], "the wizard does not name these step or stage messages")

    // And the generic line is reachable, so an unknown code still reads as words.
    assert.ok(
        code.includes(UNKNOWN_LABEL_KEY),
        "the wizard has no fallback message for a code it does not know"
    )
})

test("the wizard keeps its own motion out of the way when asked to", () => {
    assert.ok(
        wizardSource().includes("prefers-reduced-motion"),
        "the wizard must stop animating when the system asks it to"
    )
})

test("the panel paints its own controls dark, including the native ones", () => {
    const style = /<style[\s\S]*?<\/style>/.exec(wizardSource())
    assert.ok(style, "the wizard has no style block")
    const css = style[0]
    for (const selector of [".setup input", ".setup select", ".setup progress"]) {
        const rule = new RegExp(`${selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\s*\\{([^}]*)\\}`, "s")
        const match = rule.exec(css)
        assert.ok(match, `the style block has no rule for ${selector}`)
        const background = /background\s*:\s*([^;]+);/.exec(match[1])
        assert.ok(background, `${selector} has no background declaration`)
        const value = background[1].trim().toLowerCase()
        assert.equal(
            /^(#fff|#ffffff|white|rgb\(255,\s*255,\s*255\)|rgba\(255,\s*255,\s*255,\s*1\))$/.test(value),
            false,
            `${selector} must not render white in a dark theme`
        )
    }
})

test("the wizard calls every setup command the core exposes", () => {
    const code = stripComments(readFileSync(API_FILE, "utf8")) + "\n" + stripComments(wizardSource())
    const commands = [
        "local_ai_setup_status",
        "local_ai_setup_preflight",
        "local_ai_setup_start",
        "local_ai_setup_retry",
        "local_ai_setup_cancel",
        "local_ai_setup_cleanup_temp",
        "local_ai_setup_use_managed",
        "local_ai_setup_remove_runtime",
        "local_ai_setup_remove_model",
        "local_ai_setup_validate_existing",
        "local_ai_setup_run_test"
    ]
    for (const command of commands) {
        assert.ok(code.includes(`"${command}"`), `the setup panel never calls ${command}`)
    }
    // The native pickers and the config write belong to the existing settings
    // API, so the wizard reuses the wrappers rather than repeating the command
    // names: the picker commands are called through `localAiApi`, not here.
    for (const method of ["selectServer", "selectModel", "getConfig", "saveConfig"]) {
        assert.ok(code.includes(`localAiApi.${method}(`), `the setup panel never reuses localAiApi.${method}`)
    }
})

import { test } from "node:test"
import assert from "node:assert/strict"

import {
    CLIPBOARD_TIMEOUT_OPTIONS,
    DEFAULT_CLIPBOARD_TIMEOUT_SECONDS,
    DEFAULT_IDLE_TIMEOUT_SECONDS,
    GENERATOR_LENGTH_DEFAULT,
    GENERATOR_LENGTH_MAX,
    GENERATOR_LENGTH_MIN,
    IDLE_TIMEOUT_OPTIONS,
    MASK_CHARACTER,
    MIN_MASTER_PASSWORD_LENGTH,
    checkPasswordChange,
    clampGeneratorLength,
    clearedView,
    clipboardIndicatorKey,
    clipboardOptionKey,
    clipboardRemaining,
    defaultPolicy,
    defaultQuery,
    estimateEntropyBits,
    hiddenSecretValue,
    hiddenSecrets,
    idleOptionKey,
    isClipboardArmed,
    isIdleExpired,
    isMetadataDirty,
    isSecretDirty,
    maskedSecret,
    metadataFromDetails,
    nextSaveState,
    normalizeClipboardTimeout,
    normalizeIdleTimeout,
    parseUrlInput,
    selectedCategoryCount,
    sortOptionKey,
    trashOptionKey,
    validatePolicy,
    conflictOptionKey
} from "../src/lib/vault-model.ts"
import { createDebouncer } from "../src/lib/notes-model.ts"

function fakeTimers() {
    let nextHandle = 1
    let pending = null
    return {
        timers: {
            setTimeout(handler, delayMs) {
                const handle = nextHandle++
                pending = { handler, delayMs, handle }
                return handle
            },
            clearTimeout(handle) {
                if (pending && pending.handle === handle) pending = null
            }
        },
        pending: () => (pending ? 1 : 0),
        run() {
            const entry = pending
            pending = null
            if (entry) entry.handler()
        }
    }
}

const item = {
    id: "item-1",
    revision: 4,
    name: "FICTIONAL_ITEM_NAME",
    username: "FICTIONAL_USERNAME",
    urls: ["https://fictional.example.com/login"],
    tags: ["work"],
    favorite: false,
    created_at: "2026-01-01T00:00:00+00:00",
    updated_at: "2026-01-01T00:00:00+00:00",
    deleted_at: null
}

// ------------------------------------------------------------------ masking

test("a hidden password is masked and never rendered", () => {
    const secret = "FICTIONAL_PASSWORD"
    const hidden = maskedSecret(secret, false)
    assert.notEqual(hidden, secret)
    assert.equal(hidden, MASK_CHARACTER.repeat(12))
    assert.ok(!hidden.includes("FICTIONAL"))
    // The same mask for any length: the mask itself leaks nothing.
    assert.equal(maskedSecret("short", false), maskedSecret(secret, false))
    // Only an explicit reveal produces the value.
    assert.equal(maskedSecret(secret, true), secret)
})

test("hiding a secret clears it from component state", () => {
    const cleared = hiddenSecrets()
    assert.deepEqual(cleared, { password: "", notes: "" })
    assert.equal(hiddenSecretValue(), "")
    // Nothing in the cleared state can be mistaken for a secret.
    assert.ok(!JSON.stringify(cleared).includes("FICTIONAL"))
})

test("the lock action clears every decrypted view", () => {
    const view = clearedView()
    assert.deepEqual(view.items, [])
    assert.equal(view.selected, null)
    assert.equal(view.metadata, null)
    assert.equal(view.revealed, false)
    assert.deepEqual(view.secrets, { password: "", notes: "" })
    assert.deepEqual(view.conflicts, [])
    assert.deepEqual(view.tags, [])
})

// -------------------------------------------------------------- dirty state

test("metadata edits are detected without touching the secret", () => {
    const metadata = metadataFromDetails(item)
    assert.equal(isMetadataDirty(item, metadata), false)
    assert.equal(isMetadataDirty(item, { ...metadata, name: "other" }), true)
    assert.equal(isMetadataDirty(item, { ...metadata, username: "other" }), true)
    assert.equal(isMetadataDirty(item, { ...metadata, favorite: true }), true)
    assert.equal(isMetadataDirty(item, { ...metadata, urls: [] }), true)
    assert.equal(isMetadataDirty(item, { ...metadata, tags: ["work", "home"] }), true)
    assert.equal(isMetadataDirty(null, metadata), false)
})

test("a secret counts as changed only after it was revealed", () => {
    const current = { password: "FICTIONAL_PASSWORD", notes: "FICTIONAL_NOTES" }
    // Not revealed: the interface does not hold the secret, so it is not dirty.
    assert.equal(isSecretDirty(false, null, current), false)
    // Revealed and identical: not dirty.
    assert.equal(isSecretDirty(true, current, current), false)
    // Revealed and edited: dirty.
    assert.equal(isSecretDirty(true, current, { ...current, password: "new" }), true)
    assert.equal(isSecretDirty(true, current, { ...current, notes: "new" }), true)
})

// ----------------------------------------------------------------- autosave

test("vault autosave debounces and flushes like the notes editor", () => {
    const clock = fakeTimers()
    const saved = []
    const debouncer = createDebouncer(700, (value) => saved.push(value), clock.timers)

    debouncer.schedule("first")
    debouncer.schedule("second")
    assert.equal(clock.pending(), 1)
    clock.run()
    assert.deepEqual(saved, ["second"])

    debouncer.schedule("pending")
    debouncer.flush()
    assert.deepEqual(saved, ["second", "pending"])

    debouncer.schedule("dropped")
    debouncer.cancel()
    clock.run()
    assert.deepEqual(saved, ["second", "pending"])
})

test("the save indicator reuses the shared state machine", () => {
    let state = nextSaveState("idle", "edit")
    assert.equal(state, "dirty")
    state = nextSaveState(state, "save_started")
    assert.equal(state, "saving")
    state = nextSaveState(state, "save_ok")
    assert.equal(state, "saved")
    state = nextSaveState(state, "save_failed")
    assert.equal(state, "error")
    assert.equal(nextSaveState(state, "reset"), "idle")
})

// -------------------------------------------------------------------- idle

test("idle timeout options and expiry match the backend policy", () => {
    assert.deepEqual([...IDLE_TIMEOUT_OPTIONS], [60, 300, 900, 1800, 0])
    assert.equal(DEFAULT_IDLE_TIMEOUT_SECONDS, 300)
    for (const seconds of IDLE_TIMEOUT_OPTIONS) {
        assert.equal(normalizeIdleTimeout(seconds), seconds)
        assert.match(idleOptionKey(seconds), /^vault-idle-(1|5|15|30|never)$/)
    }
    // Unsupported values fall back to the recommended default.
    assert.equal(normalizeIdleTimeout(7), DEFAULT_IDLE_TIMEOUT_SECONDS)
    assert.equal(normalizeIdleTimeout(12345), DEFAULT_IDLE_TIMEOUT_SECONDS)
    assert.equal(idleOptionKey(7), "vault-idle-5")

    const automatic = { timeout_seconds: 300, automatic: true, remaining_seconds: 300 }
    assert.equal(isIdleExpired(automatic, 299), false)
    assert.equal(isIdleExpired(automatic, 300), true)
    assert.equal(isIdleExpired(automatic, 9999), true)
    // "never" must never expire, and an unknown state must not lock by accident.
    assert.equal(isIdleExpired({ timeout_seconds: 0, automatic: false, remaining_seconds: null }, 99999), false)
    assert.equal(isIdleExpired(null, 10), false)
    assert.equal(isIdleExpired({ timeout_seconds: 60, automatic: true, remaining_seconds: null }, 999), false)
})

// --------------------------------------------------------------- clipboard

test("clipboard indicator and countdown reflect the backend status", () => {
    assert.equal(clipboardIndicatorKey(null), "vault-clipboard-idle")
    assert.equal(clipboardIndicatorKey({ armed: false, remaining_seconds: 0, timeout_seconds: 30 }), "vault-clipboard-idle")
    assert.equal(clipboardIndicatorKey({ armed: true, remaining_seconds: 12, timeout_seconds: 30 }), "vault-clipboard-armed")

    assert.equal(isClipboardArmed(null), false)
    assert.equal(isClipboardArmed({ armed: true, remaining_seconds: 12, timeout_seconds: 30 }), true)
    assert.equal(clipboardRemaining({ armed: true, remaining_seconds: 12.7, timeout_seconds: 30 }), 12)
    assert.equal(clipboardRemaining({ armed: false, remaining_seconds: 30, timeout_seconds: 30 }), 0)
    assert.equal(clipboardRemaining(null), 0)

    assert.deepEqual([...CLIPBOARD_TIMEOUT_OPTIONS], [15, 30, 45, 60])
    assert.equal(DEFAULT_CLIPBOARD_TIMEOUT_SECONDS, 30)
    for (const seconds of CLIPBOARD_TIMEOUT_OPTIONS) {
        assert.equal(normalizeClipboardTimeout(seconds), seconds)
        assert.match(clipboardOptionKey(seconds), /^vault-clipboard-(15|30|45|60)$/)
    }
    // Out-of-range values are clamped to the supported band by the backend and
    // fall back to the default here.
    assert.equal(normalizeClipboardTimeout(5), DEFAULT_CLIPBOARD_TIMEOUT_SECONDS)
    assert.equal(normalizeClipboardTimeout(600), DEFAULT_CLIPBOARD_TIMEOUT_SECONDS)
})

// --------------------------------------------------------------- generator

test("generator policy validation matches the core rules", () => {
    const policy = defaultPolicy()
    assert.equal(policy.length, GENERATOR_LENGTH_DEFAULT)
    assert.equal(GENERATOR_LENGTH_DEFAULT, 20)
    assert.equal(validatePolicy(policy).ok, true)

    const none = { ...policy, lowercase: false, uppercase: false, digits: false, symbols: false }
    assert.equal(validatePolicy(none).ok, false)
    assert.equal(validatePolicy(none).errorKey, "vault-generator-needs-category")

    const tooShort = { ...policy, length: 3 }
    assert.equal(validatePolicy(tooShort).ok, false)
    assert.equal(validatePolicy(tooShort).errorKey, "vault-generator-length-invalid")

    // The minimum length already covers the four categories, so the
    // "one of each cannot fit" guard is only a safety net for future changes.
    assert.equal(validatePolicy({ ...policy, length: GENERATOR_LENGTH_MIN, require_each_category: true }).ok, true)
    assert.equal(validatePolicy({ ...policy, length: 4, require_each_category: true }).ok, false)
    assert.equal(
        validatePolicy({ ...policy, length: 4, require_each_category: true }).errorKey,
        "vault-generator-length-invalid"
    )
    assert.equal(selectedCategoryCount(policy), 4)
    assert.equal(selectedCategoryCount({ ...policy, symbols: false }), 3)
})

test("generator length is clamped to the supported band", () => {
    assert.equal(clampGeneratorLength(1), GENERATOR_LENGTH_MIN)
    assert.equal(clampGeneratorLength(20), 20)
    assert.equal(clampGeneratorLength(20.4), 20)
    assert.equal(clampGeneratorLength(999), GENERATOR_LENGTH_MAX)
    assert.equal(clampGeneratorLength(Number.NaN), GENERATOR_LENGTH_DEFAULT)
    assert.equal(GENERATOR_LENGTH_MIN, 8)
    assert.equal(GENERATOR_LENGTH_MAX, 128)
})

test("the local entropy estimate follows the policy", () => {
    const policy = defaultPolicy()
    const bits = estimateEntropyBits(policy)
    assert.ok(bits > 100, `expected a strong estimate, got ${bits}`)
    // Longer is stronger, and fewer categories is weaker.
    assert.ok(estimateEntropyBits({ ...policy, length: 40 }) > bits)
    assert.ok(estimateEntropyBits({ ...policy, symbols: false }) < bits)
    // Excluding similar characters removes thirteen glyphs from the alphabet.
    assert.ok(estimateEntropyBits({ ...policy, exclude_similar: true }) < bits)
    assert.equal(
        estimateEntropyBits({ ...policy, lowercase: false, uppercase: false, digits: false, symbols: false }),
        0
    )
})

// -------------------------------------------------------------------- urls

test("url input parsing trims, limits, and de-duplicates", () => {
    assert.deepEqual(parseUrlInput("https://a.example\nhttps://b.example"), [
        "https://a.example",
        "https://b.example"
    ])
    assert.deepEqual(parseUrlInput(" https://a.example , https://a.example "), ["https://a.example"])
    assert.deepEqual(parseUrlInput("\n\n"), [])
    const many = Array.from({ length: 20 }, (_, index) => `https://example.com/${index}`).join("\n")
    assert.equal(parseUrlInput(many).length, 16)
})

// --------------------------------------------------------- master password

test("master password change validation is complete", () => {
    assert.equal(checkPasswordChange("", "new-password", "new-password").errorKey, "vault-change-needs-current")
    assert.equal(checkPasswordChange("old-password", "short", "short").errorKey, "vault-password-short")
    assert.equal(
        checkPasswordChange("old-password", "new-password", "other-password").errorKey,
        "vault-password-mismatch"
    )
    assert.equal(
        checkPasswordChange("old-password", "old-password", "old-password").errorKey,
        "vault-change-same-password"
    )
    assert.equal(checkPasswordChange("old-password", "new-password", "new-password").ok, true)
    assert.equal(MIN_MASTER_PASSWORD_LENGTH, 8)
    // The boundary is accepted.
    assert.equal(checkPasswordChange("old-password", "12345678", "12345678").ok, true)
})

// ------------------------------------------------------------------- query

test("query defaults and option keys stay stable", () => {
    const query = defaultQuery()
    assert.equal(query.trash, "active")
    assert.equal(query.sort, "name_asc")
    assert.equal(query.favorites_only, false)
    assert.equal(query.tag, null)
    assert.equal(query.search, "")

    assert.equal(sortOptionKey("name_asc"), "vault-sort-name-asc")
    assert.equal(trashOptionKey("trashed"), "vault-filter-trashed")
    assert.equal(conflictOptionKey("accept_incoming"), "vault-conflict-accept-incoming")
})

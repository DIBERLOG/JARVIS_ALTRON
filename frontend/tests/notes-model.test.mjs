import { test } from "node:test"
import assert from "node:assert/strict"

import {
    AUTOSAVE_DELAY_MS,
    CONFLICT_RESOLUTIONS,
    MIN_MASTER_PASSWORD_LENGTH,
    NOTE_SORTS,
    TRASH_FILTERS,
    conflictOptionKey,
    createDebouncer,
    defaultQuery,
    displayExcerpt,
    displayTitle,
    draftFromNote,
    folderOptions,
    formatAbsoluteDate,
    formatTagInput,
    isDirty,
    nextSaveState,
    parseTagInput,
    relativeTime,
    saveIndicatorKey,
    shortenPath,
    sortOptionKey,
    timeUnitKey,
    trashOptionKey
} from "../src/lib/notes-model.ts"

/** Deterministic timer stand-in so debounce behaviour is not timing dependent. */
function fakeTimers() {
    let nextHandle = 1
    const scheduled = new Map()
    return {
        timers: {
            setTimeout(handler, delayMs) {
                const handle = nextHandle++
                scheduled.set(handle, { handler, delayMs })
                return handle
            },
            clearTimeout(handle) {
                scheduled.delete(handle)
            }
        },
        pending: () => scheduled.size,
        runAll() {
            const entries = [...scheduled.entries()]
            scheduled.clear()
            for (const [, entry] of entries) entry.handler()
        }
    }
}

test("autosave debounce coalesces keystrokes and flushes on demand", () => {
    const clock = fakeTimers()
    const saved = []
    const debouncer = createDebouncer(AUTOSAVE_DELAY_MS, (value) => saved.push(value), clock.timers)

    debouncer.schedule("a")
    debouncer.schedule("ab")
    debouncer.schedule("abc")
    assert.equal(clock.pending(), 1, "only the last keystroke may stay scheduled")
    assert.equal(debouncer.isPending(), true)
    assert.equal(saved.length, 0, "nothing is saved before the delay elapses")

    clock.runAll()
    assert.deepEqual(saved, ["abc"])
    assert.equal(debouncer.isPending(), false)

    // flush() must persist the latest value immediately, as when leaving a note.
    debouncer.schedule("abcd")
    debouncer.flush()
    assert.deepEqual(saved, ["abc", "abcd"])
    assert.equal(clock.pending(), 0)
    assert.equal(debouncer.isPending(), false)

    // cancel() must drop a pending save without persisting it.
    debouncer.schedule("dropped")
    debouncer.cancel()
    assert.equal(debouncer.isPending(), false)
    clock.runAll()
    assert.deepEqual(saved, ["abc", "abcd"])
})

test("save indicator never reports saved for unsaved work", () => {
    let state = nextSaveState("idle", "edit")
    assert.equal(state, "dirty")
    assert.equal(saveIndicatorKey(state, true), "notes-dirty")

    state = nextSaveState(state, "save_started")
    assert.equal(saveIndicatorKey(state, true), "notes-saving")

    // A keystroke during a save keeps the indicator in the saving state.
    assert.equal(nextSaveState(state, "edit"), "saving")

    state = nextSaveState(state, "save_failed")
    assert.equal(saveIndicatorKey(state, true), "notes-save-error")

    state = nextSaveState(state, "save_started")
    state = nextSaveState(state, "save_ok")
    assert.equal(saveIndicatorKey(state, false), "notes-saved")

    // Editing after a successful save must flip back to unsaved.
    assert.equal(saveIndicatorKey(nextSaveState(state, "edit"), true), "notes-dirty")
    assert.equal(nextSaveState("saved", "reset"), "idle")
})

test("relative time buckets stay coarse and never claim the future", () => {
    const now = new Date("2026-03-01T12:00:00Z")
    assert.deepEqual(relativeTime("2026-03-01T11:59:40Z", now), { unit: "now", value: 0 })
    assert.deepEqual(relativeTime("2026-03-01T11:30:00Z", now), { unit: "minute", value: 30 })
    assert.deepEqual(relativeTime("2026-03-01T09:00:00Z", now), { unit: "hour", value: 3 })
    assert.deepEqual(relativeTime("2026-02-27T12:00:00Z", now), { unit: "day", value: 2 })
    assert.deepEqual(relativeTime("2026-01-01T12:00:00Z", now), { unit: "date", value: 59 })
    assert.deepEqual(relativeTime("2026-03-01T12:05:00Z", now), { unit: "now", value: 0 })
    assert.deepEqual(relativeTime("not a date", now), { unit: "date", value: 0 })
    assert.equal(timeUnitKey("minute"), "notes-time-minute")
})

test("absolute date formatting is empty for invalid input and stable otherwise", () => {
    assert.equal(formatAbsoluteDate("nonsense"), "")
    assert.notEqual(formatAbsoluteDate("2026-03-01T12:00:00Z", "en-GB"), "")
})

test("titles and excerpts fall back to placeholders", () => {
    assert.equal(displayTitle("  Report  ", "Untitled"), "Report")
    assert.equal(displayTitle("   ", "Untitled"), "Untitled")
    assert.equal(displayExcerpt("", "No text"), "No text")
    assert.equal(displayExcerpt(" line ", "No text"), "line")
})

test("tag input parsing trims, strips hashes, and de-duplicates case-insensitively", () => {
    assert.deepEqual(parseTagInput("work, Home"), ["work", "Home"])
    assert.deepEqual(parseTagInput(" #work ,, work, #Work "), ["work"])
    assert.deepEqual(parseTagInput("a\nb"), ["a", "b"])
    assert.deepEqual(parseTagInput("   "), [])
    assert.deepEqual(parseTagInput("a,b,c,d", 2), ["a", "b"])
    assert.equal(formatTagInput(["a", "b"]), "a, b")
})

test("dirty detection compares every editable field", () => {
    const note = {
        id: "1",
        revision: 3,
        title: "Title",
        body: "Body",
        folder_id: null,
        tags: ["x"],
        pinned: false,
        created_at: "2026-01-01T00:00:00Z",
        updated_at: "2026-01-01T00:00:00Z",
        deleted_at: null
    }
    const draft = draftFromNote(note)
    assert.equal(isDirty(note, draft), false)
    assert.equal(isDirty(note, { ...draft, title: "Other" }), true)
    assert.equal(isDirty(note, { ...draft, body: "Other" }), true)
    assert.equal(isDirty(note, { ...draft, folder_id: "f" }), true)
    assert.equal(isDirty(note, { ...draft, tags: ["x", "y"] }), true)
    assert.equal(isDirty(note, { ...draft, tags: ["y"] }), true)
    assert.equal(isDirty(note, null), false)
    // A note that does not exist yet is dirty as soon as it has content.
    assert.equal(isDirty(null, { ...draft, title: "" , body: "" }), false)
    assert.equal(isDirty(null, draft), true)
    // Pinning is stored separately and must not look like an unsaved edit.
    assert.equal(isDirty({ ...note, pinned: true }, draft), false)
})

test("query defaults and option keys are stable", () => {
    const query = defaultQuery()
    assert.equal(query.trash, "active")
    assert.equal(query.sort, "updated_desc")
    assert.equal(query.pinned_first, true)
    assert.equal(query.search, "")
    assert.equal(query.folder_id, null)

    for (const sort of NOTE_SORTS) {
        assert.match(sortOptionKey(sort), /^notes-sort-[a-z-]+$/)
    }
    for (const filter of TRASH_FILTERS) {
        assert.equal(trashOptionKey(filter), `notes-filter-${filter}`)
    }
    for (const resolution of CONFLICT_RESOLUTIONS) {
        assert.match(conflictOptionKey(resolution), /^notes-conflict-[a-z-]+$/)
    }
    // Keys must line up with the Fluent message names used by the interface.
    assert.equal(sortOptionKey("title_asc"), "notes-sort-title-asc")
    assert.equal(conflictOptionKey("accept_incoming"), "notes-conflict-accept-incoming")
})

test("folder options skip trashed folders and keep the none entry first", () => {
    const options = folderOptions(
        [
            { id: "1", revision: 1, name: "Work", created_at: "", updated_at: "", deleted_at: null },
            { id: "2", revision: 2, name: "Old", created_at: "", updated_at: "", deleted_at: "2026-01-01T00:00:00Z" }
        ],
        "None"
    )
    assert.deepEqual(options, [
        { value: "", label: "None" },
        { value: "1", label: "Work" }
    ])
})

test("long storage paths are shortened for display", () => {
    assert.equal(shortenPath("C:\\short"), "C:\\short")
    const shortened = shortenPath("C:\\Users\\angel\\AppData\\Roaming\\com.priler.jarvis", 30)
    assert.ok(shortened.length <= 32)
    assert.ok(shortened.includes("com.priler.jarvis"))
    assert.ok(shortened.startsWith("…"))
})

test("the password rule matches the storage layer minimum", () => {
    assert.equal(MIN_MASTER_PASSWORD_LENGTH, 8)
})

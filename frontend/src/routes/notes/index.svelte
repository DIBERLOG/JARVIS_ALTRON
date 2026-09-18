<script lang="ts">
    import { onDestroy, onMount } from "svelte"
    import { Alert, Button, Text } from "@svelteuidev/core"
    import { Download, LockClosed, Plus } from "radix-icons-svelte"

    import ConflictPanel from "@/components/notes/ConflictPanel.svelte"
    import Footer from "@/components/Footer.svelte"
    import HDivider from "@/components/elements/HDivider.svelte"
    import NoteEditor from "@/components/notes/NoteEditor.svelte"
    import NoteList from "@/components/notes/NoteList.svelte"
    import NoteSidebar from "@/components/notes/NoteSidebar.svelte"
    import NotesLocked from "@/components/notes/NotesLocked.svelte"

    import { notesApi } from "@/lib/notes"
    import type {
        Note,
        NoteConflictResolution,
        NoteConflictView,
        NoteDraft,
        NoteFolder,
        NoteQuery,
        NoteSort,
        NoteSummary,
        SaveState,
        StorageStatus
    } from "@/lib/notes-model"
    import {
        AUTOSAVE_DELAY_MS,
        SEARCH_DELAY_MS,
        createDebouncer,
        defaultQuery,
        draftFromNote,
        isDirty,
        nextSaveState,
        shortenPath
    } from "@/lib/notes-model"
    import { translations, translate } from "@/stores"

    $: t = (key: string) => translate($translations, key)

    // ---------------------------------------------------------------- state

    let status: StorageStatus | null = null
    let items: NoteSummary[] = []
    let folders: NoteFolder[] = []
    let tags: string[] = []
    let conflicts: NoteConflictView[] = []
    let selected: Note | null = null
    let draft: NoteDraft | null = null
    let query: NoteQuery = defaultQuery()
    let saveState: SaveState = "idle"
    let loading = true
    let busy = false
    let errorMessage = ""
    let unreadable = 0
    let exportPassword = ""
    let exportOpen = false

    const autosave = createDebouncer<[]>(AUTOSAVE_DELAY_MS, () => {
        void saveNow()
    })
    const search = createDebouncer<[]>(SEARCH_DELAY_MS, () => {
        void refreshList()
    })

    onMount(() => {
        void bootstrap()
    })

    onDestroy(() => {
        // Never lose a pending edit when leaving the page.
        autosave.flush()
        autosave.cancel()
        search.cancel()
    })

    // ------------------------------------------------------------ lifecycle

    async function bootstrap() {
        loading = true
        try {
            status = await notesApi.status()
            if (status.state === "unlocked") {
                await refreshAll()
            }
        } catch (error) {
            errorMessage = describe(error)
        } finally {
            loading = false
        }
    }

    async function refreshAll() {
        await Promise.all([refreshList(), refreshFolders(), refreshTags(), refreshConflicts()])
    }

    async function refreshList() {
        try {
            const result = await notesApi.list(query)
            items = result.items
            unreadable = result.unreadable
        } catch (error) {
            errorMessage = describe(error)
        }
    }

    async function refreshFolders() {
        try {
            folders = await notesApi.folders()
        } catch (error) {
            errorMessage = describe(error)
        }
    }

    async function refreshTags() {
        try {
            tags = await notesApi.tags()
        } catch (error) {
            errorMessage = describe(error)
        }
    }

    async function refreshConflicts() {
        try {
            conflicts = await notesApi.conflicts()
        } catch (error) {
            errorMessage = describe(error)
        }
    }

    function describe(error: unknown): string {
        if (typeof error === "string") return error
        if (error && typeof error === "object" && "message" in error) {
            return String((error as { message: unknown }).message)
        }
        return String(error)
    }

    function resetContent() {
        items = []
        folders = []
        tags = []
        conflicts = []
        selected = null
        draft = null
        unreadable = 0
        saveState = nextSaveState(saveState, "reset")
    }

    // --------------------------------------------------------------- storage

    async function onStorageChanged(event: CustomEvent<StorageStatus>) {
        status = event.detail
        errorMessage = ""
        if (status.state !== "unlocked") {
            resetContent()
            return
        }
        await refreshAll()
    }

    function onStorageFailed(event: CustomEvent<string>) {
        errorMessage = event.detail
    }

    function onSelectNote(event: CustomEvent<string>) {
        void selectNote(event.detail)
    }

    function onTrashNote(event: CustomEvent<string>) {
        void trashNote(event.detail)
    }

    function onRestoreNote(event: CustomEvent<string>) {
        void restoreNote(event.detail)
    }

    function onPinNote(event: CustomEvent<{ id: string; pinned: boolean }>) {
        void togglePin(event.detail.id, event.detail.pinned)
    }

    function onPinOpenNote(event: CustomEvent<boolean>) {
        void pinOpenNote(event.detail)
    }

    async function lock() {
        autosave.flush()
        try {
            status = await notesApi.lock()
        } catch (error) {
            errorMessage = describe(error)
        }
        resetContent()
    }

    async function exportBackup() {
        if (exportPassword.length === 0) {
            errorMessage = t("notes-export-needs-password")
            return
        }
        busy = true
        try {
            const path = await notesApi.exportBackupFile(exportPassword)
            exportPassword = ""
            exportOpen = false
            if (path.length > 0) {
                errorMessage = ""
            }
        } catch (error) {
            errorMessage = describe(error)
        } finally {
            busy = false
        }
    }

    // ----------------------------------------------------------------- notes

    async function selectNote(id: string) {
        autosave.flush()
        try {
            const note = await notesApi.get(id)
            if (!note) {
                await refreshList()
                return
            }
            selected = note
            draft = draftFromNote(note)
            saveState = nextSaveState(saveState, "reset")
            errorMessage = ""
        } catch (error) {
            errorMessage = describe(error)
        }
    }

    async function createNote() {
        autosave.flush()
        busy = true
        try {
            const draftValue: NoteDraft = {
                title: "",
                body: "",
                folder_id: query.folder_id,
                tags: []
            }
            const note = await notesApi.create(draftValue)
            selected = note
            draft = draftFromNote(note)
            saveState = nextSaveState(saveState, "reset")
            await refreshList()
        } catch (error) {
            errorMessage = describe(error)
        } finally {
            busy = false
        }
    }

    function onDraftChange(event: CustomEvent<NoteDraft>) {
        draft = event.detail
        saveState = nextSaveState(saveState, "edit")
        autosave.schedule()
    }

    async function saveNow() {
        if (!selected || !draft) return
        if (!isDirty(selected, draft)) {
            saveState = nextSaveState(saveState, "save_ok")
            return
        }
        const noteId = selected.id
        const pending = draft
        saveState = nextSaveState(saveState, "save_started")
        try {
            const saved = await notesApi.autosave(noteId, pending)
            if (selected && selected.id === saved.id) {
                selected = saved
            }
            saveState = nextSaveState(saveState, "save_ok")
            await refreshList()
        } catch (error) {
            saveState = nextSaveState(saveState, "save_failed")
            errorMessage = describe(error)
        }
    }

    function closeEditor() {
        autosave.flush()
        selected = null
        draft = null
        saveState = nextSaveState(saveState, "reset")
    }

    async function togglePin(id: string, pinned: boolean) {
        try {
            await notesApi.setPinned(id, pinned)
            if (selected && selected.id === id) {
                selected = { ...selected, pinned }
            }
            await refreshList()
        } catch (error) {
            errorMessage = describe(error)
        }
    }

    async function pinOpenNote(pinned: boolean) {
        if (!selected) return
        try {
            const saved = await notesApi.setPinned(selected.id, pinned)
            selected = saved
            await refreshList()
        } catch (error) {
            errorMessage = describe(error)
        }
    }

    async function trashSelected() {
        if (!selected) return
        await trashNote(selected.id)
    }

    async function restoreSelected() {
        if (!selected) return
        await restoreNote(selected.id)
    }

    async function trashNote(id: string) {
        try {
            await notesApi.trash(id)
            if (selected && selected.id === id) {
                const note = await notesApi.get(id)
                selected = note
                draft = note ? draftFromNote(note) : null
            }
            await refreshList()
        } catch (error) {
            errorMessage = describe(error)
        }
    }

    async function restoreNote(id: string) {
        try {
            const note = await notesApi.restore(id)
            if (selected && selected.id === id) {
                selected = note
                draft = draftFromNote(note)
            }
            await refreshList()
        } catch (error) {
            errorMessage = describe(error)
        }
    }

    async function purgeNote() {
        if (!selected) return
        const id = selected.id
        try {
            await notesApi.purge(id)
            closeEditor()
            await refreshList()
        } catch (error) {
            errorMessage = describe(error)
        }
    }

    // --------------------------------------------------------------- folders

    async function onQueryChange(event: CustomEvent<NoteQuery>) {
        const previous = query
        query = event.detail
        if (previous.search !== query.search) {
            // Typing in the search box must not re-scan on every keystroke.
            search.schedule()
            return
        }
        await refreshList()
    }

    async function onSortChange(event: CustomEvent<NoteSort>) {
        query = { ...query, sort: event.detail }
        await refreshList()
    }

    async function createFolder(event: CustomEvent<string>) {
        try {
            const folder = await notesApi.createFolder(event.detail)
            query = { ...query, folder_id: folder.id }
            await Promise.all([refreshFolders(), refreshList()])
        } catch (error) {
            errorMessage = describe(error)
        }
    }

    async function renameFolder(event: CustomEvent<{ id: string; name: string }>) {
        try {
            await notesApi.renameFolder(event.detail.id, event.detail.name)
            await refreshFolders()
        } catch (error) {
            errorMessage = describe(error)
        }
    }

    async function purgeFolder(event: CustomEvent<string>) {
        try {
            await notesApi.purgeFolder(event.detail)
            query = { ...query, folder_id: null }
            await Promise.all([refreshFolders(), refreshList(), refreshTags()])
        } catch (error) {
            errorMessage = describe(error)
        }
    }

    // ------------------------------------------------------------- conflicts

    async function resolveConflict(
        event: CustomEvent<{ conflict: string; resolution: NoteConflictResolution }>
    ) {
        busy = true
        try {
            await notesApi.resolveConflict(event.detail.conflict, event.detail.resolution)
            await Promise.all([refreshConflicts(), refreshList(), refreshTags()])
            if (selected) {
                const refreshed = await notesApi.get(selected.id)
                selected = refreshed
                draft = refreshed ? draftFromNote(refreshed) : null
            }
        } catch (error) {
            errorMessage = describe(error)
        } finally {
            busy = false
        }
    }
</script>

{#if loading}
    <div class="loading">
        <Text size="sm" color="gray">{t('notes-loading')}</Text>
    </div>
{:else if status && status.state !== "unlocked"}
    <NotesLocked
        {status}
        on:changed={onStorageChanged}
        on:failed={onStorageFailed}
    />
    {#if errorMessage}
        <div class="error-slot">
            <Alert title={t('notes-error')} color="red" variant="outline">
                <Text size="xs" color="gray">{errorMessage}</Text>
            </Alert>
        </div>
    {/if}
{:else}
    <div class="notes-page">
        <div class="page-bar">
            <span class="page-title">{t('notes-title')}</span>
            <span class="page-count">{items.length}</span>
            <Button size="xs" color="lime" uppercase on:click={createNote} disabled={busy}>
                <Plus size={12} />
                {t('notes-new')}
            </Button>
            <Button size="xs" color="gray" uppercase on:click={lock}>
                <LockClosed size={12} />
                {t('notes-lock')}
            </Button>
        </div>

        {#if errorMessage}
            <Alert title={t('notes-error')} color="red" variant="outline">
                <Text size="xs" color="gray">{errorMessage}</Text>
            </Alert>
        {/if}

        {#if unreadable > 0}
            <Text size="xs" color="gray">{unreadable} — {t('notes-unreadable')}</Text>
        {/if}

        <ConflictPanel {conflicts} {busy} on:resolve={resolveConflict} />

        <NoteSidebar
            {query}
            {folders}
            on:query={onQueryChange}
            on:createFolder={createFolder}
            on:renameFolder={renameFolder}
            on:purgeFolder={purgeFolder}
        />

        {#if tags.length > 0}
            <div class="tag-rail">
                {#each tags as tag}
                    <button
                        class="tag-chip"
                        class:active={query.tag === tag}
                        on:click={() => onQueryChange(new CustomEvent('query', {
                            detail: { ...query, tag: query.tag === tag ? null : tag }
                        }))}
                    >
                        {tag}
                    </button>
                {/each}
            </div>
        {/if}

        <HDivider noMargin />

        <div class="notes-body">
            {#if selected && draft}
                {#key selected.id}
                    <NoteEditor
                        note={selected}
                        {draft}
                        {folders}
                        {saveState}
                        {busy}
                        on:change={onDraftChange}
                        on:save={saveNow}
                        on:back={closeEditor}
                        on:pin={onPinOpenNote}
                        on:trash={trashSelected}
                        on:restore={restoreSelected}
                        on:purge={purgeNote}
                    />
                {/key}
            {:else}
                <NoteList
                    {items}
                    selectedId={null}
                    sort={query.sort}
                    {loading}
                    on:select={onSelectNote}
                    on:pin={onPinNote}
                    on:trash={onTrashNote}
                    on:restore={onRestoreNote}
                    on:sort={onSortChange}
                />
            {/if}
        </div>

        <HDivider noMargin />

        <div class="storage-bar">
            <Text size="xs" color="gray">
                {t('notes-storage-dir')}: <span class="path">{shortenPath(status?.data_dir ?? "")}</span>
            </Text>
            {#if exportOpen}
                <div class="export-form">
                    <input
                        class="export-input"
                        type="password"
                        placeholder={t('notes-password')}
                        autocomplete="new-password"
                        bind:value={exportPassword}
                    />
                    <Button size="xs" color="cyan" uppercase on:click={exportBackup} disabled={busy}>
                        {t('notes-export')}
                    </Button>
                </div>
            {:else}
                <button class="export-toggle" on:click={() => (exportOpen = true)}>
                    <Download size={12} />
                    {t('notes-export')}
                </button>
            {/if}
        </div>

        <Footer />
    </div>
{/if}

<style lang="scss">
    .loading {
        padding: 30px 0;
        text-align: center;
    }

    .notes-page {
        display: flex;
        flex-direction: column;
        gap: 8px;
        padding-bottom: 16px;
    }

    .page-bar {
        display: flex;
        align-items: center;
        gap: 8px;
    }

    .page-title {
        font-size: 0.8rem;
        letter-spacing: 0.08em;
        text-transform: uppercase;
        color: #ffffff;
    }

    .page-count {
        background: rgba(82, 254, 254, 0.15);
        border-radius: 8px;
        color: #52fefe;
        font-size: 0.6rem;
        padding: 1px 7px;
        margin-right: auto;
    }

    .error-slot {
        margin-top: 12px;
    }

    .tag-rail {
        display: flex;
        flex-wrap: wrap;
        gap: 4px;
    }

    .tag-chip {
        background: rgba(35, 50, 55, 0.7);
        border: 1px solid transparent;
        border-radius: 10px;
        color: rgba(255, 255, 255, 0.65);
        cursor: pointer;
        font-size: 0.58rem;
        padding: 2px 8px;

        &:hover {
            color: #ffffff;
        }

        &.active {
            border-color: rgba(82, 254, 254, 0.45);
            color: #52fefe;
        }
    }

    .notes-body {
        min-height: 240px;
    }

    .storage-bar {
        display: flex;
        flex-direction: column;
        gap: 4px;
        padding-top: 8px;
    }

    .path {
        color: rgba(82, 254, 254, 0.8);
        word-break: break-all;
    }

    .export-toggle {
        align-self: flex-start;
        background: transparent;
        border: none;
        color: rgba(255, 255, 255, 0.55);
        cursor: pointer;
        display: inline-flex;
        align-items: center;
        gap: 5px;
        font-size: 0.6rem;
        letter-spacing: 0.04em;
        text-transform: uppercase;
        padding: 0;

        &:hover {
            color: #52fefe;
        }
    }

    .export-form {
        display: flex;
        gap: 6px;
        align-items: center;
    }

    .export-input {
        flex: 1;
        background: rgba(10, 18, 22, 0.75);
        border: 1px solid rgba(255, 255, 255, 0.1);
        border-radius: 6px;
        color: #ffffff;
        font-size: 0.66rem;
        padding: 5px 7px;

        &:focus {
            outline: none;
            border-color: rgba(82, 254, 254, 0.5);
        }
    }
</style>

<script lang="ts">
    import { onDestroy, onMount } from "svelte"
    import { Alert, Button, Text } from "@svelteuidev/core"
    import { Clipboard, LockClosed, Plus } from "radix-icons-svelte"

    import Footer from "@/components/Footer.svelte"
    import HDivider from "@/components/elements/HDivider.svelte"
    import VaultConflictPanel from "@/components/vault/VaultConflictPanel.svelte"
    import VaultItemEditor from "@/components/vault/VaultItemEditor.svelte"
    import VaultItemList from "@/components/vault/VaultItemList.svelte"
    import VaultLocked from "@/components/vault/VaultLocked.svelte"
    import VaultSecurity from "@/components/vault/VaultSecurity.svelte"

    import { vaultApi } from "@/lib/vault"
    import type {
        ClipboardStatus,
        SaveState,
        VaultConflictResolution,
        VaultConflictView,
        VaultItemDetails,
        VaultItemDraft,
        VaultItemSummary,
        VaultMetadataDraft,
        VaultQuery,
        VaultSort,
        VaultStatus,
        VaultTrashFilter
    } from "@/lib/vault-model"
    import { AUTOSAVE_DELAY_MS, createDebouncer, nextSaveState } from "@/lib/notes-model"
    import {
        DEFAULT_CLIPBOARD_TIMEOUT_SECONDS,
        DEFAULT_IDLE_TIMEOUT_SECONDS,
        VAULT_TRASH_FILTERS,
        clearedView,
        clipboardRemaining,
        defaultQuery,
        hiddenSecrets,
        isClipboardArmed,
        isMetadataDirty,
        isSecretDirty,
        isUnlocked,
        metadataFromDetails,
        normalizeClipboardTimeout,
        normalizeIdleTimeout,
        trashOptionKey
    } from "@/lib/vault-model"
    import { translations, translate } from "@/stores"
    import { invoke } from "@tauri-apps/api/core"

    $: t = (key: string) => translate($translations, key)

    const IDLE_SETTING_KEY = "vault_idle_timeout"
    const CLIPBOARD_SETTING_KEY = "vault_clipboard_timeout"
    const CLIPBOARD_POLL_MS = 1000
    const IDLE_POLL_MS = 5000
    const ACTIVITY_TOUCH_MS = 10000

    let status: VaultStatus | null = null
    let items: VaultItemSummary[] = []
    let tags: string[] = []
    let conflicts: VaultConflictView[] = []
    let selected: VaultItemDetails | null = null
    let metadata: VaultMetadataDraft | null = null
    let revealed = false
    let secrets: { password: string; notes: string } = hiddenSecrets()
    /** The secret as revealed, to detect real edits. */
    let revealedOriginal: { password: string; notes: string } | null = null
    let query: VaultQuery = defaultQuery()
    let saveState: SaveState = "idle"
    let loading = true
    let busy = false
    let errorMessage = ""
    let unreadable = 0
    let idleSeconds = DEFAULT_IDLE_TIMEOUT_SECONDS
    let clipboardSeconds = DEFAULT_CLIPBOARD_TIMEOUT_SECONDS
    let clipboard: ClipboardStatus | null = null

    const autosave = createDebouncer<[]>(AUTOSAVE_DELAY_MS, () => {
        void saveNow()
    })
    const search = createDebouncer<[]>(200, () => {
        void refreshList()
    })

    let clipboardTimer: ReturnType<typeof setInterval> | null = null
    let idleTimer: ReturnType<typeof setInterval> | null = null
    let lastActivity = Date.now()
    let lastTouch = 0

    onMount(() => {
        void bootstrap()
        clipboardTimer = setInterval(() => void refreshClipboard(), CLIPBOARD_POLL_MS)
        idleTimer = setInterval(() => void checkIdle(), IDLE_POLL_MS)
        window.addEventListener("mousemove", onUserActivity)
        window.addEventListener("keydown", onUserActivity)
        window.addEventListener("click", onUserActivity)
    })

    onDestroy(() => {
        if (clipboardTimer) clearInterval(clipboardTimer)
        if (idleTimer) clearInterval(idleTimer)
        window.removeEventListener("mousemove", onUserActivity)
        window.removeEventListener("keydown", onUserActivity)
        window.removeEventListener("click", onUserActivity)
        autosave.flush()
        // Leaving the page must not leave a revealed secret in memory.
        hideSecrets()
    })

    // ------------------------------------------------------------ lifecycle

    async function bootstrap() {
        loading = true
        try {
            await loadSettings()
            status = await vaultApi.status()
            await vaultApi.setIdleTimeout(idleSeconds)
            if (isUnlocked(status)) {
                await refreshAll()
            }
        } catch (error) {
            errorMessage = describe(error)
        } finally {
            loading = false
        }
    }

    async function loadSettings() {
        try {
            const [idle, clipboardValue] = await Promise.all([
                invoke<string>("db_read", { key: IDLE_SETTING_KEY }),
                invoke<string>("db_read", { key: CLIPBOARD_SETTING_KEY })
            ])
            if (idle) idleSeconds = normalizeIdleTimeout(Number(idle))
            if (clipboardValue) clipboardSeconds = normalizeClipboardTimeout(Number(clipboardValue))
        } catch {
            // Settings are conveniences; defaults are fine when unreadable.
        }
    }

    async function saveSetting(key: string, value: number) {
        try {
            await invoke("db_write", { key, val: String(value) })
        } catch {
            // A failed setting write must not break the session.
        }
    }

    async function refreshAll() {
        await Promise.all([refreshList(), refreshTags(), refreshConflicts()])
    }

    async function refreshList() {
        try {
            const result = await vaultApi.list(query)
            items = result.items
            unreadable = result.unreadable
        } catch (error) {
            await handleError(error)
        }
    }

    async function refreshTags() {
        try {
            tags = await vaultApi.tags()
        } catch (error) {
            await handleError(error)
        }
    }

    async function refreshConflicts() {
        try {
            conflicts = await vaultApi.conflicts()
        } catch (error) {
            await handleError(error)
        }
    }

    async function refreshClipboard() {
        try {
            clipboard = await vaultApi.clipboardStatus()
        } catch {
            clipboard = null
        }
    }

    async function handleError(error: unknown) {
        const message = describe(error)
        errorMessage = message
        // A locked storage is the normal end of an idle session, not a failure.
        if (message.includes("locked")) {
            await reloadStatus()
        }
    }

    function onSelectItem(event: CustomEvent<string>) {
        void selectItem(event.detail)
    }

    function onTrashItem(event: CustomEvent<string>) {
        void trashItem(event.detail)
    }

    function onRestoreItem(event: CustomEvent<string>) {
        void restoreItem(event.detail)
    }

    function onFavoriteChange(event: CustomEvent<{ id: string; favorite: boolean }>) {
        void toggleFavorite(event)
    }

    function trashSelected() {
        if (selected) void trashItem(selected.id)
    }

    function restoreSelected() {
        if (selected) void restoreItem(selected.id)
    }

    function onStorageFailed(event: CustomEvent<string>) {
        errorMessage = event.detail
    }

    function onGeneratedCopied() {
        void refreshClipboard()
    }

    function describe(error: unknown): string {
        if (typeof error === "string") return error
        if (error && typeof error === "object" && "message" in error) {
            return String((error as { message: unknown }).message)
        }
        return String(error)
    }

    function clearView() {
        const empty = clearedView()
        items = empty.items
        tags = empty.tags
        conflicts = empty.conflicts
        selected = empty.selected
        metadata = empty.metadata
        revealed = empty.revealed
        secrets = empty.secrets
        revealedOriginal = null
        unreadable = 0
        saveState = nextSaveState(saveState, "reset")
    }

    async function reloadStatus() {
        try {
            status = await vaultApi.status()
        } catch (error) {
            errorMessage = describe(error)
        }
        if (!isUnlocked(status)) {
            clearView()
        }
    }

    // ------------------------------------------------------------ idle lock

    function onUserActivity() {
        lastActivity = Date.now()
        if (Date.now() - lastTouch < ACTIVITY_TOUCH_MS) return
        lastTouch = Date.now()
        vaultApi.touch().catch(() => undefined)
    }

    async function checkIdle() {
        if (!isUnlocked(status)) return
        const localIdle = (Date.now() - lastActivity) / 1000
        if (idleSeconds !== 0 && localIdle >= idleSeconds) {
            // Lock locally right away; the backend enforces the same rule.
            await lockVault()
            return
        }
        try {
            const idle = await vaultApi.idleStatus()
            if (idle.remaining_seconds !== null && idle.remaining_seconds <= 0) {
                await lockVault()
            }
        } catch {
            await reloadStatus()
        }
    }

    async function lockVault() {
        autosave.flush()
        hideSecrets()
        try {
            status = await vaultApi.lock()
        } catch (error) {
            errorMessage = describe(error)
        }
        clearView()
    }

    // --------------------------------------------------------------- storage

    async function onStorageChanged(event: CustomEvent<VaultStatus>) {
        status = event.detail
        errorMessage = ""
        if (!isUnlocked(status)) {
            clearView()
            return
        }
        await refreshAll()
    }

    // ----------------------------------------------------------------- items

    async function selectItem(id: string) {
        autosave.flush()
        hideSecrets()
        try {
            const item = await vaultApi.get(id)
            if (!item) {
                await refreshList()
                return
            }
            selected = item
            metadata = metadataFromDetails(item)
            revealed = false
            secrets = hiddenSecrets()
            revealedOriginal = null
            saveState = nextSaveState(saveState, "reset")
            errorMessage = ""
        } catch (error) {
            errorMessage = describe(error)
        }
    }

    async function createItem() {
        autosave.flush()
        busy = true
        try {
            const draft: VaultItemDraft = {
                name: "",
                username: "",
                password: "",
                urls: [],
                notes: "",
                tags: query.tag ? [query.tag] : [],
                favorite: false
            }
            const item = await vaultApi.create(draft)
            selected = item
            metadata = metadataFromDetails(item)
            revealed = true
            secrets = { password: "", notes: "" }
            revealedOriginal = { password: "", notes: "" }
            saveState = nextSaveState(saveState, "reset")
            await refreshList()
        } catch (error) {
            errorMessage = describe(error)
        } finally {
            busy = false
        }
    }

    function onMetadataChange(event: CustomEvent<VaultMetadataDraft>) {
        metadata = event.detail
        saveState = nextSaveState(saveState, "edit")
        autosave.schedule()
    }

    function onSecretsChange(event: CustomEvent<{ password: string; notes: string }>) {
        secrets = event.detail
        saveState = nextSaveState(saveState, "edit")
        autosave.schedule()
    }

    async function revealSecret() {
        if (!selected) return
        try {
            const result = await vaultApi.reveal(selected.id)
            revealed = true
            secrets = { password: result.password, notes: result.notes }
            revealedOriginal = { password: result.password, notes: result.notes }
            errorMessage = ""
        } catch (error) {
            errorMessage = describe(error)
        }
    }

    function hideSecrets() {
        revealed = false
        secrets = hiddenSecrets()
        revealedOriginal = null
    }

    async function saveNow() {
        if (!selected || !metadata) return
        const secretChanged = isSecretDirty(revealed, revealedOriginal, secrets)
        const metadataChanged = isMetadataDirty(selected, metadata)
        if (!secretChanged && !metadataChanged) {
            saveState = nextSaveState(saveState, "save_ok")
            return
        }
        const id = selected.id
        saveState = nextSaveState(saveState, "save_started")
        try {
            let updated = selected
            if (metadataChanged) {
                // Metadata-only save: an un-revealed secret can never be erased.
                updated = await vaultApi.updateMetadata(id, metadata)
            }
            if (secretChanged) {
                updated = await vaultApi.updateSecrets(id, secrets.password, secrets.notes)
            }
            if (selected && selected.id === id) {
                selected = updated
                metadata = metadataFromDetails(updated)
            }
            if (secretChanged) {
                revealedOriginal = { ...secrets }
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
        hideSecrets()
        selected = null
        metadata = null
        saveState = nextSaveState(saveState, "reset")
    }

    async function toggleFavorite(event: CustomEvent<{ id: string; favorite: boolean }>) {
        try {
            const updated = await vaultApi.setFavorite(event.detail.id, event.detail.favorite)
            if (selected && selected.id === updated.id) {
                selected = updated
                if (metadata) metadata = { ...metadata, favorite: updated.favorite }
            }
            await refreshList()
        } catch (error) {
            errorMessage = describe(error)
        }
    }

    async function trashItem(id: string) {
        try {
            await vaultApi.trash(id)
            hideSecrets()
            if (selected && selected.id === id) {
                selected = await vaultApi.get(id)
                metadata = selected ? metadataFromDetails(selected) : null
            }
            await refreshList()
        } catch (error) {
            errorMessage = describe(error)
        }
    }

    async function restoreItem(id: string) {
        try {
            const updated = await vaultApi.restore(id)
            if (selected && selected.id === id) {
                selected = updated
                metadata = metadataFromDetails(updated)
            }
            await refreshList()
        } catch (error) {
            errorMessage = describe(error)
        }
    }

    async function purgeItem() {
        if (!selected) return
        const id = selected.id
        try {
            await vaultApi.purge(id)
            closeEditor()
            await refreshList()
        } catch (error) {
            errorMessage = describe(error)
        }
    }

    // --------------------------------------------------------- filters, tags

    function setTrash(filter: VaultTrashFilter) {
        query = { ...query, trash: filter, tag: null }
        void refreshList()
    }

    function toggleFavorites() {
        query = { ...query, favorites_only: !query.favorites_only }
        void refreshList()
    }

    function toggleTag(tag: string) {
        query = { ...query, tag: query.tag === tag ? null : tag, trash: "active" }
        void refreshList()
    }

    function onSortChange(event: CustomEvent<VaultSort>) {
        query = { ...query, sort: event.detail }
        void refreshList()
    }

    function onSearchInput(event: Event) {
        query = { ...query, search: (event.currentTarget as HTMLInputElement).value }
        search.schedule()
    }

    // ----------------------------------------------------------- clipboard

    async function copyUsername() {
        if (!selected) return
        try {
            clipboard = await vaultApi.copyUsername(selected.id, clipboardSeconds)
            errorMessage = ""
        } catch (error) {
            errorMessage = describe(error)
        }
    }

    async function copyPassword() {
        if (!selected) return
        try {
            // The command performs the copy in Rust; no secret comes back.
            clipboard = await vaultApi.copyPassword(selected.id, clipboardSeconds)
            errorMessage = ""
        } catch (error) {
            errorMessage = describe(error)
        }
    }

    async function clearClipboard() {
        try {
            clipboard = await vaultApi.clipboardClear()
        } catch (error) {
            errorMessage = describe(error)
        }
    }

    // -------------------------------------------------------------- security

    async function onIdleChange(event: CustomEvent<number>) {
        idleSeconds = normalizeIdleTimeout(event.detail)
        await saveSetting(IDLE_SETTING_KEY, idleSeconds)
        try {
            await vaultApi.setIdleTimeout(idleSeconds)
        } catch (error) {
            errorMessage = describe(error)
        }
    }

    async function onClipboardChange(event: CustomEvent<number>) {
        clipboardSeconds = normalizeClipboardTimeout(event.detail)
        await saveSetting(CLIPBOARD_SETTING_KEY, clipboardSeconds)
    }

    // -------------------------------------------------------------- conflicts

    async function resolveConflict(
        event: CustomEvent<{ conflict: string; resolution: VaultConflictResolution }>
    ) {
        busy = true
        try {
            await vaultApi.resolveConflict(event.detail.conflict, event.detail.resolution)
            await Promise.all([refreshConflicts(), refreshList(), refreshTags()])
            if (selected) {
                const refreshed = await vaultApi.get(selected.id)
                selected = refreshed
                metadata = refreshed ? metadataFromDetails(refreshed) : null
            }
        } catch (error) {
            errorMessage = describe(error)
        } finally {
            busy = false
        }
    }

    $: unlocked = isUnlocked(status)
    $: armed = isClipboardArmed(clipboard)
    $: remaining = clipboardRemaining(clipboard)
</script>

{#if loading}
    <div class="loading"><Text size="sm" color="gray">{t('vault-loading')}</Text></div>
{:else if status && !unlocked}
    <VaultLocked {status} on:changed={onStorageChanged} on:failed={onStorageFailed} />
    {#if errorMessage}
        <div class="error-slot">
            <Alert title={t('vault-error')} color="red" variant="outline">
                <Text size="xs" color="gray">{errorMessage}</Text>
            </Alert>
        </div>
    {/if}
{:else if status}
    <div class="vault-page">
        <div class="page-bar">
            <span class="page-title">{t('vault-title')}</span>
            <span class="page-count">{items.length}</span>
            <Button size="xs" color="lime" uppercase on:click={createItem} disabled={busy}>
                <Plus size={12} />
                {t('vault-new')}
            </Button>
            <Button size="xs" color="gray" uppercase on:click={lockVault}>
                <LockClosed size={12} />
                {t('vault-lock')}
            </Button>
        </div>

        {#if armed}
            <div class="clipboard-banner">
                <Clipboard size={12} />
                <Text size="xs" color="gray">{t('vault-clipboard-armed')} · {remaining}</Text>
                <button class="clear-button" on:click={clearClipboard}>{t('vault-clipboard-clear')}</button>
            </div>
        {/if}

        {#if errorMessage}
            <Alert title={t('vault-error')} color="red" variant="outline">
                <Text size="xs" color="gray">{errorMessage}</Text>
            </Alert>
        {/if}

        {#if unreadable > 0}
            <Text size="xs" color="gray">{unreadable} — {t('vault-unreadable')}</Text>
        {/if}

        <VaultConflictPanel {conflicts} {busy} on:resolve={resolveConflict} />

        <div class="chips">
            {#each VAULT_TRASH_FILTERS as filter}
                <button class="chip" class:active={query.trash === filter} on:click={() => setTrash(filter)}>
                    {t(trashOptionKey(filter))}
                </button>
            {/each}
            <button class="chip" class:active={query.favorites_only} on:click={toggleFavorites}>
                {t('vault-filter-favorites')}
            </button>
        </div>

        {#if tags.length > 0}
            <div class="chips">
                {#each tags as tag}
                    <button class="chip tag" class:active={query.tag === tag} on:click={() => toggleTag(tag)}>
                        {tag}
                    </button>
                {/each}
            </div>
        {/if}

        <input
            class="search-input"
            type="text"
            placeholder={t('vault-search')}
            value={query.search}
            on:input={onSearchInput}
        />

        <HDivider noMargin />

        <div class="vault-body">
            {#if selected && metadata}
                {#key selected.id}
                    <VaultItemEditor
                        item={selected}
                        {metadata}
                        {saveState}
                        {busy}
                        {revealed}
                        {secrets}
                        clipboardTimeout={clipboardSeconds}
                        clipboardArmed={armed}
                        clipboardRemaining={remaining}
                        on:metadata={onMetadataChange}
                        on:secrets={onSecretsChange}
                        on:reveal={revealSecret}
                        on:hide={hideSecrets}
                        on:save={saveNow}
                        on:back={closeEditor}
                        on:copyUsername={copyUsername}
                        on:copyPassword={copyPassword}
                        on:clipboardGenerated={onGeneratedCopied}
                        on:clipboardClear={clearClipboard}
                        on:trash={trashSelected}
                        on:restore={restoreSelected}
                        on:purge={purgeItem}
                    />
                {/key}
            {:else}
                <VaultItemList
                    {items}
                    selectedId={null}
                    sort={query.sort}
                    {loading}
                    on:select={onSelectItem}
                    on:favorite={onFavoriteChange}
                    on:trash={onTrashItem}
                    on:restore={onRestoreItem}
                    on:sort={onSortChange}
                />
            {/if}
        </div>

        <HDivider noMargin />

        <VaultSecurity
            {status}
            {idleSeconds}
            {clipboardSeconds}
            on:idle={onIdleChange}
            on:clipboard={onClipboardChange}
            on:changed={onStorageChanged}
            on:failed={onStorageFailed}
        />

        <Footer />
    </div>
{/if}

<style lang="scss">
    .loading {
        padding: 30px 0;
        text-align: center;
    }

    .vault-page {
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

    .clipboard-banner {
        display: flex;
        align-items: center;
        gap: 6px;
        background: rgba(82, 254, 254, 0.08);
        border-radius: 6px;
        padding: 4px 8px;
    }

    .clear-button {
        background: transparent;
        border: none;
        color: #52fefe;
        cursor: pointer;
        font-size: 0.6rem;
        letter-spacing: 0.04em;
        text-transform: uppercase;
        padding: 0;
        margin-left: auto;
    }

    .error-slot {
        margin-top: 12px;
    }

    .chips {
        display: flex;
        flex-wrap: wrap;
        gap: 4px;
    }

    .chip {
        background: rgba(35, 50, 55, 0.7);
        border: 1px solid transparent;
        border-radius: 12px;
        color: rgba(255, 255, 255, 0.7);
        cursor: pointer;
        font-size: 0.62rem;
        letter-spacing: 0.03em;
        padding: 3px 9px;

        &:hover {
            background: rgba(82, 254, 254, 0.12);
            color: #ffffff;
        }

        &.active {
            background: rgba(82, 254, 254, 0.18);
            border-color: rgba(82, 254, 254, 0.45);
            color: #52fefe;
        }

        &.tag {
            font-size: 0.58rem;
            max-width: 140px;
            overflow: hidden;
            text-overflow: ellipsis;
            white-space: nowrap;
        }
    }

    .search-input {
        width: 100%;
        background: rgba(10, 18, 22, 0.75);
        border: 1px solid rgba(255, 255, 255, 0.1);
        border-radius: 6px;
        color: #ffffff;
        font-size: 0.72rem;
        padding: 7px 8px;

        &::placeholder {
            color: rgba(255, 255, 255, 0.35);
        }

        &:focus {
            outline: none;
            border-color: rgba(82, 254, 254, 0.5);
        }
    }

    .vault-body {
        min-height: 240px;
    }
</style>

<script lang="ts">
    /**
     * The memory management page.
     *
     * It is the only place that talks to `memoryApi`: the components render and
     * dispatch, and every decision that needs a round trip (the secret confirmation,
     * the destructive confirmations, the refresh after a write) is made here.
     *
     * Two rules hold the page together:
     *
     * * nothing is read or counted while the shared storage is locked, and the counts
     *   the backend reports for a locked store are never rendered;
     * * a refusal caused by the secret filter is shown as a warning that names only
     *   kinds, and the retry is a separate, explicit click.
     */
    import { onMount } from "svelte"
    import { Alert, Text } from "@svelteuidev/core"

    import Footer from "@/components/Footer.svelte"
    import HDivider from "@/components/elements/HDivider.svelte"
    import MemoryBudgetPanel from "@/components/memory/MemoryBudgetPanel.svelte"
    import MemoryCandidateList from "@/components/memory/MemoryCandidateList.svelte"
    import MemoryConflictPanel from "@/components/memory/MemoryConflictPanel.svelte"
    import MemoryConversationDetail from "@/components/memory/MemoryConversationDetail.svelte"
    import MemoryConversationList from "@/components/memory/MemoryConversationList.svelte"
    import MemoryFactEditor from "@/components/memory/MemoryFactEditor.svelte"
    import MemoryFactList from "@/components/memory/MemoryFactList.svelte"
    import MemoryLocked from "@/components/memory/MemoryLocked.svelte"
    import MemoryNotice from "@/components/memory/MemoryNotice.svelte"
    import MemorySecretWarning from "@/components/memory/MemorySecretWarning.svelte"
    import MemorySettingsPanel from "@/components/memory/MemorySettings.svelte"
    import MemoryStatsStrip from "@/components/memory/MemoryStatsStrip.svelte"

    import { memoryApi } from "@/lib/memory"
    import type {
        BudgetView,
        ConversationDetails,
        ConversationView,
        FactDraft,
        FactQuery,
        FactView,
        MemoryCategory,
        MemoryConflictView,
        MemoryScope,
        MemorySettings,
        MemoryStatusView,
        Persona,
        SecretKind
    } from "@/lib/memory-model"
    import {
        SECRET_KINDS,
        bulkDeleteConfirmationKey,
        canUseFacts,
        canWriteHistory,
        contextWarningKey,
        conversationTitle,
        defaultFactQuery,
        defaultScopeFor,
        exportWarningKeys,
        importWarningKeys,
        isKeyMissing,
        isSecretConfirmation,
        isSummarizing,
        largeMemoryWarningKey,
        matchesFact,
        memoryIsUsable,
        needsSetup,
        needsUnlock,
        normalizeSettings,
        sortFacts
    } from "@/lib/memory-model"
    import { translations, translate } from "@/stores"

    $: t = (key: string) => translate($translations, key)

    const PAGE_LIMIT = 200
    const SEARCH_DELAY_MS = 200
    /** The confirmation keys of the per-row destructive actions. */
    const DESTRUCTIVE_SCOPES = ["history", "conversation", "fact"] as const

    let loading = true
    let busy = false
    let errorMessage = ""
    let notice = ""
    let noticeKeys: string[] = []
    let noticeDetail = ""

    let status: MemoryStatusView | null = null
    let settings: MemorySettings | null = null

    let facts: FactView[] = []
    let factsLoading = false
    let query: FactQuery = defaultFactQuery()
    let searchTimer: ReturnType<typeof setTimeout> | null = null

    let candidates: FactView[] = []
    let candidatesLoading = false

    let conversations: ConversationView[] = []
    let includeArchived = false
    let selectedConversationId: string | null = null
    let details: ConversationDetails | null = null
    /** The profile of the last conversation that was opened, when there was one. */
    let activeProfile: Persona | null = null

    let budget: BudgetView | null = null
    let conflicts: MemoryConflictView[] = []

    /** The entry the editor is showing; `undefined` means the editor is closed. */
    let editing: FactView | null | undefined = undefined

    /** The refused save that is waiting for a secret confirmation. */
    let secretKinds: SecretKind[] = []
    let secretContext = ""
    let secretRetry: (() => Promise<void>) | null = null

    // ------------------------------------------------------------ derived views

    $: usable = memoryIsUsable(status)
    $: historyWritable = canWriteHistory(status)
    $: factsUsable = canUseFacts(status)
    $: largeWarning = status ? largeMemoryWarningKey(status.memory.stats) : null
    $: conversationTitles = conversations.reduce<Record<string, string>>((titles, conversation) => {
        titles[conversation.id] = conversationTitle(conversation.title, t('memory-new-conversation'))
        return titles
    }, {})
    /**
     * The scope a suggested entry belongs to by default: the profile of the
     * conversation it came from, and the shared scope when no profile is known.
     */
    $: candidateScope = activeProfile ? defaultScopeFor(activeProfile) : "global"
    /** The instant filter and the order of the loaded page. */
    $: visibleFacts = sortFacts(facts.filter((fact) => matchesFact(fact, query)))

    // ---------------------------------------------------------------- lifecycle

    onMount(() => {
        void bootstrap()
        return () => {
            if (searchTimer) clearTimeout(searchTimer)
        }
    })

    async function bootstrap() {
        loading = true
        try {
            status = await memoryApi.status()
            settings = normalizeSettings(status.memory.settings)
            if (!needsUnlock(status) && !needsSetup(status) && !isKeyMissing(status)) {
                await refreshAll()
            }
        } catch (error) {
            errorMessage = describe(error)
        } finally {
            loading = false
        }
    }

    async function refreshStatus() {
        status = await memoryApi.status()
        settings = normalizeSettings(status.memory.settings)
    }

    async function refreshAll() {
        await refreshStatus()
        await Promise.all([refreshFacts(), refreshCandidates(), refreshConversations(), refreshBudget(), refreshConflicts()])
    }

    /**
     * A locked storage answers with a lock error instead of data; the page turns that
     * into the gate rather than into a failure, because an idle lock is normal.
     */
    async function handleError(error: unknown) {
        const message = describe(error)
        if (message.toLowerCase().includes("locked")) {
            errorMessage = ""
            await refreshStatus()
            return
        }
        errorMessage = message
    }

    function describe(error: unknown): string {
        if (typeof error === "string") return error
        if (error && typeof error === "object" && "message" in error) {
            return String((error as { message: unknown }).message)
        }
        return String(error)
    }

    function clearNotice() {
        notice = ""
        noticeKeys = []
        noticeDetail = ""
    }

    // --------------------------------------------------------------- the gate

    async function onUnlocked() {
        clearNotice()
        errorMessage = ""
        try {
            await refreshAll()
        } catch (error) {
            await handleError(error)
        }
    }

    /** Locks the shared storage, so the notes and the vault lock with it. */
    async function lockStorage() {
        busy = true
        try {
            await memoryApi.lock()
            await refreshStatus()
            clearLists()
        } catch (error) {
            await handleError(error)
        } finally {
            busy = false
        }
    }

    function clearLists() {
        facts = []
        candidates = []
        conversations = []
        details = null
        selectedConversationId = null
        activeProfile = null
        budget = null
        conflicts = []
        editing = undefined
    }

    // ---------------------------------------------------------------- settings

    async function saveSettings(next: MemorySettings) {
        busy = true
        try {
            status = await memoryApi.updateSettings(next)
            settings = normalizeSettings(status.memory.settings)
            notice = t('memory-settings-saved')
            noticeKeys = []
            if (!status.memory.unlocked) {
                clearLists()
            } else if (!editing) {
                await refreshAll()
            }
        } catch (error) {
            await handleError(error)
        } finally {
            busy = false
        }
    }

    // ------------------------------------------------------------------- facts

    async function refreshFacts() {
        factsLoading = true
        try {
            // The store applies the same filters; the local mirror keeps the list
            // responsive while the request is in flight.
            facts = await memoryApi.listFacts({
                ...query,
                include_deleted: true,
                state: null,
                limit: PAGE_LIMIT,
                offset: 0
            })
        } catch (error) {
            await handleError(error)
        } finally {
            factsLoading = false
        }
    }

    function onQuery(next: FactQuery) {
        query = next
        void refreshFacts()
    }

    function onSearch(value: string) {
        query = { ...query, search: value }
        if (searchTimer) clearTimeout(searchTimer)
        searchTimer = setTimeout(() => {
            void refreshFacts()
        }, SEARCH_DELAY_MS)
    }

    function newFact() {
        editing = null
        secretContext = ""
    }

    /**
     * Saves a draft, answering the secret gate.
     *
     * The first attempt never carries the confirmation flag: the backend refuses a
     * suspicious text, the page shows the kinds it named, and only the explicit
     * "save anyway" repeats the call with the flag set.
     */
    async function saveDraft(draft: FactDraft) {
        busy = true
        const target = editing
        try {
            if (target) {
                await memoryApi.updateFact(target.id, draft)
            } else {
                await memoryApi.createFact(draft)
            }
            editing = undefined
            secretContext = ""
            secretKinds = []
            notice = t('memory-settings-saved')
            noticeKeys = []
            await refreshFacts()
            await refreshStatus()
        } catch (error) {
            if (await offerSecretRetry(error, 'memory-fact-save', async () => {
                await saveDraft({ ...draft, accept_secret_warning: true })
            })) {
                return
            }
            await handleError(error)
        } finally {
            busy = false
        }
    }

    /** Toggles one flag, or removes the entry, always through the store. */
    async function setFactUsage(fact: FactView, changes: { pinned?: boolean; disabled?: boolean }) {
        busy = true
        try {
            await memoryApi.setFactUsage(fact.id, changes.pinned ?? fact.pinned, changes.disabled ?? fact.disabled)
            await refreshFacts()
        } catch (error) {
            await handleError(error)
        } finally {
            busy = false
        }
    }

    async function removeFact(id: string) {
        busy = true
        try {
            await memoryApi.deleteFact(id)
            if (editing && editing.id === id) editing = undefined
            await refreshFacts()
            await refreshStatus()
        } catch (error) {
            await handleError(error)
        } finally {
            busy = false
        }
    }

    async function restoreFact(id: string) {
        busy = true
        try {
            await memoryApi.restoreFact(id)
            await refreshFacts()
            await refreshStatus()
        } catch (error) {
            await handleError(error)
        } finally {
            busy = false
        }
    }

    async function purgeFact(id: string) {
        busy = true
        try {
            await memoryApi.purgeFact(id)
            if (editing && editing.id === id) editing = undefined
            await refreshFacts()
            await refreshStatus()
        } catch (error) {
            await handleError(error)
        } finally {
            busy = false
        }
    }

    // -------------------------------------------------------------- candidates

    async function refreshCandidates() {
        candidatesLoading = true
        try {
            candidates = await memoryApi.listCandidates(null)
        } catch (error) {
            await handleError(error)
        } finally {
            candidatesLoading = false
        }
    }

    async function approveCandidate(event: CustomEvent<{ id: string; draft: FactDraft }>) {
        const { id, draft } = event.detail
        busy = true
        try {
            await memoryApi.approveCandidate(id, draft)
            noticeKeys = []
            notice = t('memory-settings-saved')
            await Promise.all([refreshCandidates(), refreshFacts(), refreshStatus()])
        } catch (error) {
            if (await offerSecretRetry(error, 'memory-candidate-approve', async () => {
                await memoryApi.approveCandidate(id, { ...draft, accept_secret_warning: true })
                await Promise.all([refreshCandidates(), refreshFacts(), refreshStatus()])
            })) {
                return
            }
            await handleError(error)
        } finally {
            busy = false
        }
    }

    async function rejectCandidate(event: CustomEvent<string>) {
        busy = true
        try {
            await memoryApi.rejectCandidate(event.detail)
            await refreshCandidates()
            await refreshStatus()
        } catch (error) {
            await handleError(error)
        } finally {
            busy = false
        }
    }

    // --------------------------------------------------------------- the gate

    /**
     * Turns a refusal into the secret warning and keeps the call for the confirmation.
     *
     * Only the kinds the backend reports are kept: the matched text never reaches the
     * interface, so this function has nothing to hide.
     */
    async function offerSecretRetry(
        error: unknown,
        contextKey: string,
        retry: () => Promise<void>
    ): Promise<boolean> {
        const message = describe(error)
        if (!isSecretConfirmation(message)) return false
        secretKinds = secretKindsFrom(message)
        secretContext = contextKey
        secretRetry = retry
        errorMessage = ""
        return true
    }

    async function confirmSecret() {
        const retry = secretRetry
        secretRetry = null
        secretKinds = []
        secretContext = ""
        if (!retry) return
        try {
            await retry()
        } catch (error) {
            await handleError(error)
        }
    }

    function cancelSecret() {
        secretRetry = null
        secretKinds = []
        secretContext = ""
    }

    /**
     * The kinds behind a refusal.
     *
     * The backend reports kinds, never values, and the message names them in the
     * backend's own words; an unrecognised wording falls back to the generic label,
     * which is what a user must see anyway.
     */
    function secretKindsFrom(message: string): SecretKind[] {
        const found: SecretKind[] = []
        if (message.includes("private key")) found.push("private_key")
        if (message.includes("API token")) found.push("api_token")
        if (message.includes("JSON Web Token")) found.push("jwt")
        if (message.includes("recovery code")) found.push("recovery_code")
        if (message.includes("assigned password")) found.push("password_assignment")
        if (message.includes("long random-looking token")) found.push("high_entropy_token")
        if (message.includes("payment card")) found.push("payment_card")
        // Keep the order of the model's own list, so the warning reads the same way.
        return SECRET_KINDS.filter((kind) => found.includes(kind))
    }

    // ------------------------------------------------------------ conversations

    async function refreshConversations() {
        try {
            conversations = await memoryApi.listConversations({
                include_archived: includeArchived,
                limit: PAGE_LIMIT,
                offset: 0
            })
        } catch (error) {
            await handleError(error)
        }
    }

    async function openConversation(id: string) {
        try {
            details = await memoryApi.openConversation(id, 0, PAGE_LIMIT)
            selectedConversationId = id
            activeProfile = details.conversation.profile
            errorMessage = ""
        } catch (error) {
            await handleError(error)
        }
    }

    function selectConversation(event: CustomEvent<string>) {
        void openConversation(event.detail)
    }

    async function createConversation() {
        busy = true
        try {
            const conversation = await memoryApi.createConversation("jarvis", "")
            includeArchived = true
            await refreshConversations()
            await openConversation(conversation.id)
        } catch (error) {
            await handleError(error)
        } finally {
            busy = false
        }
    }

    async function renameConversation(event: CustomEvent<{ id: string; title: string }>) {
        busy = true
        try {
            await memoryApi.renameConversation(event.detail.id, event.detail.title)
            await refreshConversations()
            if (selectedConversationId === event.detail.id) {
                await openConversation(event.detail.id)
            }
        } catch (error) {
            await handleError(error)
        } finally {
            busy = false
        }
    }

    async function setArchived(event: CustomEvent<{ archived: boolean; id: string | null }>) {
        const { archived, id } = event.detail
        if (id === null) {
            includeArchived = archived
            await refreshConversations()
            return
        }
        busy = true
        try {
            await memoryApi.archiveConversation(id, archived)
            await refreshConversations()
        } catch (error) {
            await handleError(error)
        } finally {
            busy = false
        }
    }

    async function deleteConversation(event: CustomEvent<string>) {
        busy = true
        try {
            await memoryApi.deleteConversation(event.detail)
            if (selectedConversationId === event.detail) {
                selectedConversationId = null
                details = null
            }
            await refreshConversations()
            await refreshStatus()
        } catch (error) {
            await handleError(error)
        } finally {
            busy = false
        }
    }

    async function clearConversation(event: CustomEvent<string>) {
        busy = true
        try {
            await memoryApi.clearConversation(event.detail)
            notice = t('memory-clear-done')
            noticeKeys = []
            if (selectedConversationId === event.detail) {
                await openConversation(event.detail)
            }
            await refreshConversations()
            await refreshStatus()
        } catch (error) {
            await handleError(error)
        } finally {
            busy = false
        }
    }

    /** Every conversation at once, after the explicit confirmation of the list. */
    async function clearHistory() {
        busy = true
        try {
            await memoryApi.clearHistory(true)
            details = null
            selectedConversationId = null
            notice = t('memory-clear-done')
            noticeKeys = []
            noticeDetail = ""
            await Promise.all([refreshConversations(), refreshStatus()])
        } catch (error) {
            await handleError(error)
        } finally {
            busy = false
        }
    }

    /** Only the page may ask the model; the panel just requests it. */
    async function summarize() {
        if (!selectedConversationId) return
        const id = selectedConversationId
        busy = true
        try {
            await memoryApi.summarize(id)
            await refreshStatus()
            if (selectedConversationId === id) {
                await openConversation(id)
            }
        } catch (error) {
            await handleError(error)
        } finally {
            busy = false
        }
    }

    // ------------------------------------------------------------------ budget

    async function refreshBudget() {
        try {
            budget = await memoryApi.contextBudget()
        } catch (error) {
            await handleError(error)
        }
    }

    // --------------------------------------------------------------- conflicts

    async function refreshConflicts() {
        try {
            conflicts = await memoryApi.conflicts()
        } catch (error) {
            await handleError(error)
        }
    }

    async function resolveConflict(
        event: CustomEvent<{ conflict: string; resolution: "keep_current" | "accept_incoming" }>
    ) {
        busy = true
        try {
            await memoryApi.resolveConflict(event.detail.conflict, event.detail.resolution)
            await Promise.all([refreshConflicts(), refreshFacts(), refreshCandidates()])
        } catch (error) {
            await handleError(error)
        } finally {
            busy = false
        }
    }

    // ------------------------------------------------------------ export/import

    /** Export requires an explicit confirmation before the command is called. */
    async function exportBackup() {
        busy = true
        clearNotice()
        try {
            const result = await memoryApi.exportBackup(true)
            noticeKeys = exportWarningKeys(result, result.path.length === 0)
            noticeDetail = result.path.length === 0 ? "" : `${t('memory-export-done')} ${result.records}`
            notice = ""
        } catch (error) {
            await handleError(error)
        } finally {
            busy = false
        }
    }

    /** Import asks first, then reports what it did, including secret suspects. */
    async function importBackup() {
        busy = true
        clearNotice()
        try {
            const outcome = await memoryApi.importBackup()
            noticeKeys = importWarningKeys(outcome)
            noticeDetail = ""
            notice = ""
            await Promise.all([refreshFacts(), refreshCandidates(), refreshConversations(), refreshStatus(), refreshConflicts()])
        } catch (error) {
            await handleError(error)
        } finally {
            busy = false
        }
    }

    /** The keys of the warnings the backend reported for the last context. */
    $: contextWarningKeys = status && status.memory.linear_search_cost_warning
        ? [contextWarningKey(status.memory.linear_search_cost_warning)]
        : []
</script>

{#if loading}
    <div class="loading"><Text size="sm" color="gray">{t('memory-stats-title')}</Text></div>
{:else if status && usable}
    <div class="memory-body">
        <div class="page-bar">
            <span class="page-title">{t('memory-title')}</span>
            <Text size="xs" color="gray">{t('memory-local-note')}</Text>
        </div>

        <MemoryStatsStrip stats={status.memory.stats} />

        {#if largeWarning}
            <MemoryNotice titleKey={largeWarning} messageKeys={[]} />
        {/if}

        {#if !historyWritable || !factsUsable}
            <MemoryNotice titleKey={t('memory-chat-no-storage')} messageKeys={[]} />
        {/if}

        {#if errorMessage}
            <Alert title={t('memory-warning-generic')} color="red" variant="outline">
                <Text size="xs" color="gray">{errorMessage}</Text>
            </Alert>
        {/if}

        <MemoryNotice
            titleKey={noticeKeys.length > 0 ? noticeKeys[0] : notice}
            messageKeys={noticeKeys.slice(1)}
            detail={noticeDetail}
        />

        <MemoryConflictPanel {conflicts} {busy} on:resolve={resolveConflict} />

        <div class="columns">
            <MemoryFactList
                facts={visibleFacts}
                {query}
                selectedId={editing ? editing.id : null}
                {busy}
                loading={factsLoading}
                {conversationTitles}
                on:query={(event) => onQuery(event.detail)}
                on:search={(event) => onSearch(event.detail)}
                on:create={newFact}
                on:select={(event) => (editing = facts.find((fact) => fact.id === event.detail) ?? null)}
                on:pin={(event) => setFactUsage(event.detail, { pinned: !event.detail.pinned })}
                on:disable={(event) => setFactUsage(event.detail, { disabled: !event.detail.disabled })}
                on:remove={(event) => removeFact(event.detail)}
                on:restore={(event) => restoreFact(event.detail)}
                on:purge={(event) => purgeFact(event.detail)}
            />

            <div class="side">
                {#if editing !== undefined}
                    {#key editing ? editing.id : 'new'}
                        <MemoryFactEditor
                            fact={editing}
                            defaultScope={defaultScopeFor("jarvis")}
                            defaultCategory="personal_fact"
                            {busy}
                            on:save={(event) => saveDraft(event.detail)}
                            on:cancel={() => (editing = undefined)}
                        />
                    {/key}
                {/if}

                <MemoryBudgetPanel {budget} facts={facts} />

                {#if contextWarningKeys.length > 0}
                    <MemoryNotice titleKey={contextWarningKeys[0]} messageKeys={[]} />
                {/if}

                <div class="backup-zone">
                    <Text size="xs" color="gray">{t('memory-export-confirm')}</Text>
                    <div class="danger-actions">
                        <button class="mini" disabled={busy} on:click={exportBackup}>
                            {t('memory-export')}
                        </button>
                        <button class="mini" disabled={busy} on:click={lockStorage}>
                            {t('memory-lock')}
                        </button>
                    </div>

                    <Text size="xs" color="gray">{t('memory-confirm-import')}</Text>
                    <div class="danger-actions">
                        <button class="mini" disabled={busy} on:click={importBackup}>
                            {t('memory-import')}
                        </button>
                    </div>
                </div>
            </div>
        </div>

        <MemoryCandidateList
            {candidates}
            defaultScope={candidateScope}
            {busy}
            loading={candidatesLoading}
            on:approve={approveCandidate}
            on:reject={rejectCandidate}
        />

        <HDivider noMargin />

        <div class="columns">
            <MemoryConversationList
                {conversations}
                selectedId={selectedConversationId}
                {includeArchived}
                {busy}
                on:select={selectConversation}
                on:create={createConversation}
                on:archived={setArchived}
                on:rename={renameConversation}
                on:remove={deleteConversation}
                on:clear={clearConversation}
                on:clearAll={clearHistory}
            />

            <div class="side">
                {#if details}
                    {#key details.conversation.id}
                        <MemoryConversationDetail
                            {details}
                            summarizing={isSummarizing(status, details.conversation.id)}
                            {busy}
                            on:summarize={summarize}
                            on:close={() => {
                                details = null
                                selectedConversationId = null
                                activeProfile = null
                            }}
                        />
                    {/key}
                {:else}
                    <div class="empty-side">
                        <Text size="xs" color="gray">{t('memory-conversation-none')}</Text>
                        <Text size="xs" color="gray">{t('memory-profile-switch')}</Text>
                        <Text size="xs" color="gray">{t('memory-local-note')}</Text>
                    </div>
                {/if}
            </div>
        </div>

        {#if settings}
            <MemorySettingsPanel
                settings={settings}
                {busy}
                {notice}
                on:save={(event) => saveSettings(event.detail)}
            />
        {/if}

        <HDivider noMargin />

        <Footer />
    </div>
{:else if status}
    {#if settings}
        <MemorySettingsPanel
            settings={settings}
            {busy}
            {notice}
            on:save={(event) => saveSettings(event.detail)}
        />
    {/if}
    <HDivider noMargin />
    <MemoryLocked {status} {busy} on:unlocked={onUnlocked} on:failed={(event) => (errorMessage = event.detail)} />
    {#if errorMessage}
        <Alert title={t('memory-warning-generic')} color="red" variant="outline">
            <Text size="xs" color="gray">{errorMessage}</Text>
        </Alert>
    {/if}
    <Footer />
{:else}
    <div class="loading"><Text size="sm" color="gray">{t('memory-stats-title')}</Text></div>
{/if}

{#if secretRetry}
    <MemorySecretWarning
        kinds={secretKinds}
        contextKey={secretContext}
        {busy}
        on:confirm={confirmSecret}
        on:cancel={cancelSecret}
    />
{/if}

<style lang="scss">
    .loading {
        padding: 30px 0;
        text-align: center;
    }

    .memory-body {
        display: flex;
        flex-direction: column;
        gap: 8px;
        padding-bottom: 16px;
    }

    .page-bar {
        align-items: baseline;
        display: flex;
        flex-wrap: wrap;
        gap: 10px;
    }

    .page-title {
        color: #ffffff;
        font-size: 0.8rem;
        letter-spacing: 0.08em;
        text-transform: uppercase;
    }

    .columns {
        align-items: start;
        display: grid;
        gap: 10px;
        grid-template-columns: minmax(0, 1.15fr) minmax(0, 1fr);
    }

    .side {
        display: flex;
        flex-direction: column;
        gap: 8px;
        min-width: 0;
    }

    .empty-side {
        background: rgba(20, 30, 35, 0.4);
        border: 1px dashed rgba(255, 255, 255, 0.1);
        border-radius: 10px;
        display: flex;
        flex-direction: column;
        gap: 3px;
        padding: 12px;
    }

    .backup-zone {
        background: rgba(20, 30, 35, 0.45);
        border: 1px solid rgba(255, 255, 255, 0.08);
        border-radius: 10px;
        display: flex;
        flex-direction: column;
        gap: 4px;
        padding: 9px;
    }

    .danger-actions {
        display: flex;
        flex-wrap: wrap;
        gap: 8px;
        margin: 3px 0;
    }

    .mini {
        background: transparent;
        border: none;
        color: rgba(255, 255, 255, 0.65);
        cursor: pointer;
        font-size: 0.6rem;
        letter-spacing: 0.04em;
        padding: 0;
        text-transform: uppercase;

        &:hover {
            color: #52fefe;
        }

        &.danger:hover {
            color: #ff6b6b;
        }

        &:disabled {
            cursor: default;
            opacity: 0.5;
        }
    }

    @media (max-width: 1100px) {
        .columns {
            grid-template-columns: minmax(0, 1fr);
        }
    }
</style>

<script lang="ts">
    /**
     * Streaming chat with the local model, backed by the encrypted AI memory.
     *
     * The component only talks to the local gateway and to the memory commands: it
     * never starts a process, never calls the model server directly, and never reads
     * the database. Tokens arrive on a Tauri channel, so the answer is rendered as it
     * is produced.
     *
     * Two rules from the memory stage shape this file:
     *
     * * the user's message is stored **before** the request is sent, and the answer is
     *   stored only when it actually finished — a cancelled answer is kept only if the
     *   user asks for it, and a failure stores nothing;
     * * a stored context (facts, summary, earlier messages) is built by the backend
     *   and sent as user-level data. The profile system prompt is added by the gateway
     *   and is never part of this component, so memory can never replace it.
     */
    import { afterUpdate, onMount, onDestroy } from "svelte"

    import { translate, translations } from "@/stores"
    import { generationChannel, localAiApi } from "@/lib/local-ai"
    import {
        applyGenerationEvent,
        beginExchange,
        beginRetry,
        buildRequest,
        canGenerate,
        canSend,
        canStart,
        canStop,
        defaultSettings,
        emptyChatView,
        failExchange,
        formatDuration,
        formatUsage,
        lastAnswer,
        normalizeSettings,
        profileLabelKey,
        serverSummary,
        stateLabelKey,
        thinkingAvailable,
        thinkingLabelKey,
        validateDraft,
        PROFILES,
        THINKING_MODES
    } from "@/lib/local-ai-model"
    import type {
        AiProfile,
        ChatView,
        GenerationEvent,
        LocalAiSettings,
        LocalAiStatus,
        ThinkingMode
    } from "@/lib/local-ai-model"
    import { memoryApi } from "@/lib/memory"
    import {
        canWriteHistory,
        categoryLabelKey,
        contextSectionLabelKey,
        contextWarningKey,
        conversationTitle,
        defaultScopeFor,
        isSecretConfirmation,
        isSummarizing,
        memoryIsUsable,
        needsSetup,
        needsUnlock,
        scopeLabelKey,
        secretKindLabelKey,
        MEMORY_SCOPES
    } from "@/lib/memory-model"
    import type {
        ContextPlanView,
        ConversationView,
        FactView,
        MemoryScope,
        MemoryStatusView,
        MessageStatus,
        SecretKind
    } from "@/lib/memory-model"
    import { autocorrectApi } from "@/lib/autocorrect"
    import type {
        AutocorrectStatus,
        CheckReport,
        Correction,
        ImprovementMode,
        Language,
        TextImprovementPreview,
        UndoStatus
    } from "@/lib/autocorrect-model"
    import { CHAT_SCOPE } from "@/lib/autocorrect-model"
    import SpellingPanel from "@/components/notes/SpellingPanel.svelte"
    import TextImprovementPanel from "@/components/ai/TextImprovementPanel.svelte"

    import { Button, Text } from "@svelteuidev/core"

    $: t = (key: string) => translate($translations, key)

    let settings: LocalAiSettings = defaultSettings()
    let status: LocalAiStatus | null = null
    let chat: ChatView = emptyChatView()
    let draft = ""
    let busy = false
    let actionError = ""
    let messageBox: HTMLDivElement | null = null
    let statusTimer: ReturnType<typeof setInterval> | null = null

    // --- memory state
    let memoryStatus: MemoryStatusView | null = null
    let conversations: ConversationView[] = []
    let activeConversation: ConversationView | null = null
    let showArchived = false
    let useMemoryForRequest = true
    let lastPlan: ContextPlanView | null = null
    let showSources = false
    let candidates: FactView[] = []
    let candidateScopes: Record<string, MemoryScope> = {}
    let memoryNotice = ""
    let secretWarning: { kinds: SecretKind[]; retry: () => Promise<void> } | null = null
    /**
     * The turn that is in flight, so a retry never stores the question twice.
     *
     * A retry of the same question in the same conversation reuses the stored row; a
     * new question stores a new one. The flag is cleared as soon as the answer is
     * stored, so the next question is always a fresh turn.
     */
    let pendingTurn: { conversationId: string; prompt: string; stored: boolean } | null = null
    let partialAnswer: { text: string } | null = null

    // ----------------------------------------------------- draft spelling

    /**
     * Spelling and improvement state for the draft.
     *
     * The draft is not encrypted storage, so the check runs on whatever is typed; the
     * corrections and the preview are still applied by the backend under a version
     * guard, and nothing is stored in the browser.
     */
    let spellStatus: AutocorrectStatus | null = null
    let spellReport: CheckReport | null = null
    let spellCheckedText = ""
    let spellUndo: UndoStatus | null = null
    let spellBusy = false
    let spellError = ""
    let improveOpen = false
    let improvePreview: TextImprovementPreview | null = null
    let improveBusy = false
    let improveError = ""

    let spellTimer: ReturnType<typeof setTimeout> | null = null

    $: spellEnabled = spellStatus?.settings.enabled === true && spellStatus?.settings.check_chat === true
    $: improveOffered = spellStatus?.settings.ai_improvement === true

    /** The settings are incomplete or refused, so the server cannot start. */
    $: draftIssues = validateDraft(settings)
    $: blocked = draftIssues.some((issue) => issue.level === "blocked")
    $: configured = settings.server.server_path.trim().length > 0 && settings.server.model_path.trim().length > 0
    $: sendable = canSend(chat, status, draft, settings)
    // Only warn about reasoning once a server is running and cannot honour the
    // preference: a stopped server says nothing about support.
    $: thinkingUnsupported = canGenerate(status) && !thinkingAvailable(status, settings.thinking)
    $: memoryUsable = memoryIsUsable(memoryStatus)
    $: memoryWritable = canWriteHistory(memoryStatus)
    $: memoryLocked = needsUnlock(memoryStatus)
    $: memoryUninitialized = needsSetup(memoryStatus)
    $: summarizing = isSummarizing(memoryStatus, activeConversation?.id ?? null)

    onMount(async () => {
        await refreshSettings()
        await refreshStatus()
        await refreshMemory()
        await refreshSpelling()
        // A slow status refresh keeps uptime, capabilities, and a crashed server
        // visible; it is never used to follow a generation, which streams.
        statusTimer = setInterval(() => {
            if (status && status.state !== "stopped") {
                void refreshStatus()
            }
        }, 5000)
    })

    onDestroy(() => {
        if (statusTimer) clearInterval(statusTimer)
        statusTimer = null
        if (spellTimer) clearTimeout(spellTimer)
        spellTimer = null
    })

    // ------------------------------------------------------- draft spelling

    async function refreshSpelling() {
        try {
            const view = await autocorrectApi.status()
            spellStatus = view.autocorrect
        } catch {
            // A missing spelling layer never blocks the chat.
            spellStatus = null
        }
    }

    /** Checks the draft on a pause, never on every keystroke. */
    function scheduleSpellCheck() {
        if (spellTimer) clearTimeout(spellTimer)
        if (!spellEnabled) return
        spellTimer = setTimeout(() => {
            spellTimer = null
            void runSpellCheck()
        }, spellStatus?.settings.debounce_ms ?? 600)
    }

    async function runSpellCheck() {
        if (!spellEnabled || draft.trim().length === 0) {
            spellReport = null
            spellCheckedText = ""
            return
        }
        const text = draft
        try {
            const view = await autocorrectApi.check("chat", text)
            spellReport = view.report
            spellCheckedText = text
            spellError = ""
            spellUndo = await autocorrectApi.undoStatus(CHAT_SCOPE)
        } catch (error) {
            spellError = describeError(error)
        }
    }

    async function applyCorrections(event: CustomEvent<Correction[]>) {
        if (!spellReport) return
        if (spellCheckedText !== draft) {
            spellError = t("autocorrect-stale")
            await runSpellCheck()
            return
        }
        spellBusy = true
        try {
            const batch = await autocorrectApi.apply(
                CHAT_SCOPE,
                draft,
                spellReport.version,
                event.detail
            )
            draft = batch.after
            spellError = batch.skipped.length > 0 ? t("autocorrect-skipped") : ""
            await runSpellCheck()
        } catch (error) {
            spellError = describeError(error)
        }
        spellBusy = false
    }

    async function undoCorrection() {
        spellBusy = true
        try {
            const outcome = await autocorrectApi.undo(CHAT_SCOPE, draft)
            draft = outcome.text
            await runSpellCheck()
        } catch (error) {
            spellError = describeError(error)
        }
        spellBusy = false
    }

    async function rememberWord(event: CustomEvent<{ word: string; language: Language }>) {
        spellBusy = true
        try {
            await autocorrectApi.addWord(event.detail.word, event.detail.language)
            await runSpellCheck()
            await refreshSpelling()
        } catch (error) {
            spellError = describeError(error)
        }
        spellBusy = false
    }

    async function ignoreWord(event: CustomEvent<string>) {
        try {
            await autocorrectApi.ignoreWord(event.detail)
            await runSpellCheck()
        } catch (error) {
            spellError = describeError(error)
        }
    }

    function openImprovement() {
        improveOpen = true
        improvePreview = null
        improveError = ""
    }

    function closeImprovement() {
        improveOpen = false
        improvePreview = null
        improveError = ""
    }

    /** Produces a preview of the draft. The draft is not changed by this call. */
    async function requestImprovement(event: CustomEvent<{ mode: ImprovementMode; instruction: string }>) {
        improveBusy = true
        improveError = ""
        try {
            improvePreview = await autocorrectApi.improveText({
                text: draft,
                mode: event.detail.mode,
                instruction: event.detail.instruction.length > 0 ? event.detail.instruction : null
            })
        } catch (error) {
            improvePreview = null
            improveError = describeError(error)
        }
        improveBusy = false
    }

    async function cancelImprovement() {
        try {
            await autocorrectApi.cancelImprovement()
        } catch (error) {
            improveError = describeError(error)
        }
        improveBusy = false
    }

    /** Applies a preview the user confirmed. The draft changes only here. */
    async function applyImprovement() {
        if (!improvePreview) return
        improveBusy = true
        try {
            const batch = await autocorrectApi.applyImprovement(
                CHAT_SCOPE,
                draft,
                improvePreview,
                improvePreview.version_before
            )
            draft = batch.after
            improvePreview = null
            improveOpen = false
            await runSpellCheck()
        } catch (error) {
            improveError = describeError(error)
        }
        improveBusy = false
    }

    async function refreshSettings() {
        try {
            settings = normalizeSettings(await localAiApi.getConfig())
        } catch (error) {
            actionError = describeError(error)
        }
    }

    async function refreshStatus() {
        try {
            status = await localAiApi.status()
            if (status.last_error && !chat.error) actionError = status.last_error
        } catch (error) {
            actionError = describeError(error)
        }
    }

    async function refreshMemory() {
        try {
            memoryStatus = await memoryApi.status()
            if (!memoryStatus.memory.unlocked) {
                // Locking must close every open memory detail in the interface: the
                // decrypted text is gone in Rust, so it must not stay on screen.
                activeConversation = null
                conversations = []
                candidates = []
                lastPlan = null
                partialAnswer = null
                chat = emptyChatView()
                memoryNotice = t("memory-chat-no-storage")
                return
            }
            await loadConversations()
        } catch (error) {
            memoryStatus = null
            memoryNotice = describeError(error)
        }
    }

    async function loadConversations() {
        if (!memoryStatus || !memoryStatus.memory.unlocked) {
            conversations = []
            return
        }
        try {
            const list = await memoryApi.listConversations({
                include_archived: showArchived,
                limit: 100,
                offset: 0
            })
            conversations = list
            if (activeConversation) {
                const refreshed = list.find((entry) => entry.id === activeConversation?.id) ?? null
                activeConversation = refreshed
            }
        } catch (error) {
            memoryNotice = describeError(error)
        }
    }

    function describeError(error: unknown): string {
        if (typeof error === "string") return error
        if (error instanceof Error) return error.message
        return String(error)
    }

    /** Persists the two switches that the chat itself owns. */
    async function persistSwitches() {
        try {
            await localAiApi.saveConfig(settings)
        } catch (error) {
            actionError = describeError(error)
        }
    }

    async function onProfileChange(event: Event) {
        const value = (event.target as HTMLSelectElement).value as AiProfile
        settings = { ...settings, profile: value }
        await persistSwitches()
        if (activeConversation) {
            // Memory is scoped per assistant, so the profile of a conversation is fixed.
            // Switching starts a new conversation instead of mixing the two areas.
            activeConversation = null
            chat = emptyChatView()
            candidates = []
            lastPlan = null
            memoryNotice = t("memory-profile-switch")
        }
    }

    async function onThinkingChange(event: Event) {
        const value = (event.target as HTMLSelectElement).value as ThinkingMode
        settings = { ...settings, thinking: value }
        await persistSwitches()
    }

    async function startServer() {
        busy = true
        actionError = ""
        try {
            status = await localAiApi.start()
        } catch (error) {
            actionError = describeError(error)
            await refreshStatus()
        }
        busy = false
    }

    async function stopServer() {
        busy = true
        actionError = ""
        try {
            status = await localAiApi.stop()
            chat = { ...chat, generating: false }
        } catch (error) {
            actionError = describeError(error)
        }
        busy = false
    }

    async function restartServer() {
        busy = true
        actionError = ""
        try {
            status = await localAiApi.restart()
        } catch (error) {
            actionError = describeError(error)
            await refreshStatus()
        }
        busy = false
    }

    function scrollToLatest() {
        if (messageBox) messageBox.scrollTop = messageBox.scrollHeight
    }

    // Scrolling after the DOM update keeps the newest token visible while the
    // answer streams, without a timer and without polling.
    afterUpdate(scrollToLatest)

    // --- conversations

    function titleForPrompt(prompt: string): string {
        const firstLine = prompt.split("\n")[0].trim()
        if (firstLine.length <= 60) return firstLine
        return `${firstLine.slice(0, 59)}…`
    }

    async function ensureConversation(prompt: string): Promise<ConversationView | null> {
        if (!memoryWritable) return null
        if (activeConversation) return activeConversation
        try {
            const created = await memoryApi.createConversation(
                settings.profile,
                conversationTitle(titleForPrompt(prompt), t("memory-new-conversation"))
            )
            activeConversation = created
            conversations = [created, ...conversations]
            return created
        } catch (error) {
            memoryNotice = describeError(error)
            return null
        }
    }

    async function openConversation(event: Event) {
        const id = (event.target as HTMLSelectElement).value
        if (!id) {
            activeConversation = null
            chat = emptyChatView()
            candidates = []
            lastPlan = null
            return
        }
        await loadConversation(id)
    }

    async function loadConversation(id: string) {
        try {
            const details = await memoryApi.openConversation(id, 0, 200)
            activeConversation = details.conversation
            candidates = details.candidates
            chat = {
                ...emptyChatView(),
                entries: details.page.messages.map((message) => ({
                    role: message.role,
                    text: message.content,
                    // Reasoning is never stored, so there is nothing to restore here.
                    thinking: ""
                }))
            }
            lastPlan = null
            partialAnswer = null
        } catch (error) {
            memoryNotice = describeError(error)
        }
    }

    async function newConversation() {
        activeConversation = null
        chat = emptyChatView()
        candidates = []
        lastPlan = null
        partialAnswer = null
        memoryNotice = ""
    }

    async function renameConversation() {
        if (!activeConversation) return
        const title = window.prompt(t("memory-rename"), activeConversation.title)
        if (!title) return
        try {
            activeConversation = await memoryApi.renameConversation(activeConversation.id, title)
            await loadConversations()
        } catch (error) {
            memoryNotice = describeError(error)
        }
    }

    async function archiveConversation(archived: boolean) {
        if (!activeConversation) return
        try {
            await memoryApi.archiveConversation(activeConversation.id, archived)
            await loadConversations()
        } catch (error) {
            memoryNotice = describeError(error)
        }
    }

    async function deleteConversation() {
        if (!activeConversation) return
        if (!window.confirm(t("memory-confirm-conversation"))) return
        try {
            await memoryApi.deleteConversation(activeConversation.id)
            await newConversation()
            await refreshMemory()
        } catch (error) {
            memoryNotice = describeError(error)
        }
    }

    // --- sending

    async function send() {
        if (!sendable) return
        const prompt = draft.trim()
        const retry =
            pendingTurn !== null &&
            pendingTurn.prompt === prompt &&
            pendingTurn.conversationId === (activeConversation?.id ?? "")
        const conversation = await ensureConversation(prompt)
        const conversationId = conversation?.id ?? ""

        // The user's message is stored once, before the request goes out. A retry of
        // the same question reuses that row instead of storing a second copy.
        let stored = retry && pendingTurn?.stored === true
        if (!retry && memoryWritable) {
            stored = await storeUserMessage(conversationId, prompt)
        } else if (!retry && !memoryWritable) {
            memoryNotice = t("memory-chat-no-storage")
        }
        pendingTurn = { conversationId, prompt, stored }

        draft = ""
        chat = retry ? beginRetry(chat, Date.now()) : beginExchange(chat, prompt, Date.now())
        partialAnswer = null
        candidates = []

        // The backend builds the stored context; without memory the local view is used.
        let request = buildRequest(chat, settings, prompt)
        lastPlan = null
        if (memoryUsable && conversationId.length > 0) {
            try {
                const plan = await memoryApi.buildContext(
                    conversationId,
                    prompt,
                    useMemoryForRequest,
                    true
                )
                lastPlan = plan
                if (plan.messages.length > 0) {
                    request = {
                        ...request,
                        messages: plan.messages
                    }
                }
            } catch (error) {
                memoryNotice = describeError(error)
            }
        } else if (!memoryWritable) {
            memoryNotice = t("memory-chat-no-storage")
        }

        const channel = generationChannel((event: GenerationEvent) => {
            chat = applyGenerationEvent(chat, event, Date.now())
            if (event.type === "completed") {
                void finishAnswer(conversationId, event.cancelled ? "cancelled" : "completed", event.cancelled)
            } else if (event.type === "cancelled") {
                partialAnswer = { text: lastAnswer(chat)?.text ?? "" }
            } else if (event.type === "failed") {
                void finishAnswer(conversationId, "failed", false)
            }
        })

        try {
            await localAiApi.generate(channel, request)
        } catch (error) {
            chat = failExchange(chat, describeError(error))
        }
    }

    /** Stores the question; returns whether it was stored. */
    async function storeUserMessage(conversationId: string, prompt: string): Promise<boolean> {
        if (conversationId.length === 0 || !memoryWritable) return false
        try {
            await memoryApi.appendUserMessage(conversationId, prompt)
            return true
        } catch (error) {
            memoryNotice = `${t("memory-not-saved")} ${describeError(error)}`
            return false
        }
    }

    /**
     * Stores what actually happened.
     *
     * A cancelled or failed answer is never stored as a completed one, and a failure
     * stores nothing at all: the question stays and the chat continues.
     */
    async function finishAnswer(conversationId: string, status: MessageStatus, allowPartial: boolean) {
        const answer = lastAnswer(chat)
        const text = answer?.text ?? ""
        if (conversationId.length === 0 || !memoryWritable) return
        if (text.trim().length === 0) return
        if (status === "failed") {
            memoryNotice = t("memory-answer-failed")
            await refreshMemory()
            return
        }
        if (status === "cancelled" && allowPartial) {
            // The partial answer is kept only when the user asks for it.
            partialAnswer = { text }
            return
        }
        await storeAssistantMessage(conversationId, text, status, false)
        await afterAnswerStored(conversationId)
    }

    async function storeAssistantMessage(
        conversationId: string,
        text: string,
        status: MessageStatus,
        partial: boolean
    ) {
        try {
            await memoryApi.appendAssistantMessage(conversationId, text, status, partial)
        } catch (error) {
            memoryNotice = `${t("memory-not-saved")} ${describeError(error)}`
            return
        }
        pendingTurn = null
    }

    /** Keeps the partial answer of a cancelled generation, on request. */
    async function keepPartialAnswer() {
        if (!partialAnswer || !activeConversation) return
        const text = partialAnswer.text
        partialAnswer = null
        if (text.trim().length === 0) return
        await storeAssistantMessage(activeConversation.id, text, "cancelled", true)
        await afterAnswerStored(activeConversation.id)
    }

    /** Summaries and memory candidates happen after the answer is stored. */
    async function afterAnswerStored(conversationId: string) {
        await loadConversations()
        try {
            const settings = await memoryApi.getSettings()
            if (settings.suggest_facts) {
                candidates = await memoryApi.suggestCandidates(conversationId)
            }
            if (settings.auto_summaries) {
                await memoryApi.summarize(conversationId)
            }
        } catch (error) {
            memoryNotice = describeError(error)
        }
        await refreshMemory()
    }

    async function stopGeneration() {
        try {
            await localAiApi.cancel()
        } catch (error) {
            actionError = describeError(error)
        }
    }

    function clearConversation() {
        chat = emptyChatView()
        actionError = ""
        lastPlan = null
        partialAnswer = null
    }

    // --- candidates and secret warnings

    function candidateScope(candidate: FactView): MemoryScope {
        return candidateScopes[candidate.id] ?? defaultScopeFor(settings.profile)
    }

    function setCandidateScope(candidateId: string, scope: MemoryScope) {
        candidateScopes = { ...candidateScopes, [candidateId]: scope }
    }

    function onCandidateScopeChange(candidateId: string, event: Event) {
        const value = (event.currentTarget as HTMLSelectElement).value as MemoryScope
        setCandidateScope(candidateId, value)
    }

    async function approveCandidate(candidate: FactView) {
        const approved = await runWithSecretGate(() =>
            memoryApi.approveCandidate(candidate.id, {
                scope: candidateScope(candidate),
                category: candidate.category,
                content: candidate.content,
                pinned: false,
                disabled: false,
                accept_secret_warning: false
            })
        )
        // A candidate refused by the secret gate stays in the list, so the user can
        // confirm it after reading the warning.
        if (approved) {
            candidates = candidates.filter((entry) => entry.id !== candidate.id)
            await refreshMemory()
        }
    }

    async function rejectCandidate(candidate: FactView) {
        try {
            await memoryApi.rejectCandidate(candidate.id)
            candidates = candidates.filter((entry) => entry.id !== candidate.id)
        } catch (error) {
            memoryNotice = describeError(error)
        }
    }

    /**
     * Runs an action that may be refused by the secret filter.
     *
     * The warning names the kinds the backend recognized and never the matched text,
     * and repeating the action is what counts as the user's confirmation. Returns
     * whether the action went through.
     */
    async function runWithSecretGate(action: () => Promise<unknown>): Promise<boolean> {
        try {
            await action()
            secretWarning = null
            return true
        } catch (error) {
            const message = describeError(error)
            if (isSecretConfirmation(message)) {
                const kinds = extractSecretKinds(message)
                secretWarning = {
                    kinds,
                    retry: async () => {
                        await action()
                        secretWarning = null
                    }
                }
            } else {
                memoryNotice = message
            }
            return false
        }
    }

    /** Reads the kinds out of a content-free warning, without any matched text. */
    function extractSecretKinds(message: string): SecretKind[] {
        const lowered = message.toLowerCase()
        const kinds: SecretKind[] = []
        const table: Array<[string, SecretKind]> = [
            ["private key", "private_key"],
            ["api token", "api_token"],
            ["json web token", "jwt"],
            ["recovery code", "recovery_code"],
            ["assigned password", "password_assignment"],
            ["random-looking token", "high_entropy_token"],
            ["payment card", "payment_card"]
        ]
        for (const [needle, kind] of table) {
            if (lowered.includes(needle)) kinds.push(kind)
        }
        return kinds
    }
</script>

<div class="local-ai">
    <div class="ai-header">
        <div class="ai-title">
            <span class="ai-dot" class:live={canGenerate(status)} />
            <Text weight={600}>{t('ai-chat-title')}</Text>
            <span class="ai-state">{status ? t(stateLabelKey(status.state)) : t('ai-chat-state-stopped')}</span>
        </div>
        <div class="ai-controls">
            {#if canStart(status)}
                <Button
                    size="xs"
                    color="lime"
                    uppercase
                    on:click={startServer}
                    disabled={busy || blocked}
                >
                    {t('ai-chat-start')}
                </Button>
            {/if}
            {#if canStop(status)}
                <Button size="xs" color="gray" uppercase on:click={stopServer} disabled={busy}>
                    {t('ai-chat-stop-server')}
                </Button>
                <Button size="xs" color="gray" uppercase on:click={restartServer} disabled={busy}>
                    {t('ai-chat-restart')}
                </Button>
            {/if}
            {#if chat.entries.length > 0}
                <Button size="xs" color="gray" variant="subtle" uppercase on:click={clearConversation}>
                    {t('ai-chat-clear')}
                </Button>
            {/if}
        </div>
    </div>

    <p class="ai-note">{t('ai-chat-local-note')}</p>

    <div class="ai-switches">
        <label class="ai-field">
            <span>{t('ai-chat-profile')}</span>
            <select value={settings.profile} on:change={onProfileChange} disabled={busy}>
                {#each PROFILES as profile}
                    <option value={profile}>{t(profileLabelKey(profile))}</option>
                {/each}
            </select>
            <small>{t('ai-chat-profile-desc')}</small>
        </label>

        <label class="ai-field">
            <span>{t('ai-chat-thinking')}</span>
            <select value={settings.thinking} on:change={onThinkingChange} disabled={busy}>
                {#each THINKING_MODES as mode}
                    <option value={mode}>{t(thinkingLabelKey(mode))}</option>
                {/each}
            </select>
            <small>{t('ai-chat-thinking-desc')}</small>
        </label>
    </div>

    {#if thinkingUnsupported}
        <p class="ai-warn">{t('ai-chat-thinking-unavailable')}</p>
    {/if}

    {#if !configured}
        <p class="ai-warn">{t('ai-chat-needs-config')}</p>
    {:else if !canGenerate(status) && status && status.state !== 'starting'}
        <p class="ai-warn">{t('ai-chat-not-running')}</p>
    {/if}

    {#if status}
        <p class="ai-server">
            {serverSummary(status)}
            {#if status.uptime}· {t('ai-chat-uptime')} {status.uptime}{/if}
        </p>
        <div class="ai-capabilities">
            <span class:on={status.capabilities.streaming}>{t('ai-chat-cap-streaming')}</span>
            <span class:on={status.capabilities.thinking_switch}>{t('ai-chat-cap-thinking')}</span>
            <span class:on={status.capabilities.reasoning_field_observed}>{t('ai-chat-cap-reasoning')}</span>
        </div>
        {#if status.out_of_memory_hint}
            <p class="ai-warn">{t('ai-chat-out-of-memory')}</p>
        {/if}
        {#if status.stderr_tail.length > 0}
            <details class="ai-stderr">
                <summary>{t('ai-chat-stderr')}{status.stderr_truncated ? ' …' : ''}</summary>
                <pre>{status.stderr_tail.join("\n")}</pre>
            </details>
        {/if}
    {/if}

    <!-- memory: conversation list, indicator, and the per-request switch -->
    <div class="ai-memory">
        <div class="ai-memory-row">
            <span class="ai-memory-label">{t('memory-conversations')}</span>
            <select value={activeConversation?.id ?? ""} on:change={openConversation}>
                <option value="">{t('memory-new-conversation')}</option>
                {#each conversations as conversation}
                    <option value={conversation.id}>
                        {conversationTitle(conversation.title, t('memory-new-conversation'))}
                    </option>
                {/each}
            </select>
            <Button size="xs" color="gray" variant="subtle" uppercase on:click={newConversation}>
                {t('memory-new-conversation')}
            </Button>
            {#if conversations.length === 0}
                <span class="ai-memory-hint">{t('memory-conversation-none')}</span>
            {/if}
        </div>

        <div class="ai-memory-row">
            <label class="ai-check">
                <input
                    type="checkbox"
                    bind:checked={showArchived}
                    on:change={() => loadConversations()}
                />
                <span>{t('memory-show-archived')}</span>
            </label>
            {#if activeConversation}
                <span class="ai-memory-hint">{t(profileLabelKey(activeConversation.profile))}</span>
                <Button size="xs" color="gray" variant="subtle" uppercase on:click={renameConversation}>
                    {t('memory-rename')}
                </Button>
                {#if activeConversation.archived_at}
                    <Button size="xs" color="gray" variant="subtle" uppercase on:click={() => archiveConversation(false)}>
                        {t('memory-restore')}
                    </Button>
                {:else}
                    <Button size="xs" color="gray" variant="subtle" uppercase on:click={() => archiveConversation(true)}>
                        {t('memory-archive')}
                    </Button>
                {/if}
                <Button size="xs" color="red" variant="subtle" uppercase on:click={deleteConversation}>
                    {t('memory-delete')}
                </Button>
            {/if}
        </div>

        <div class="ai-memory-row">
            <label class="ai-check">
                <input type="checkbox" bind:checked={useMemoryForRequest} />
                <span>{useMemoryForRequest ? t('memory-chat-memory-on') : t('memory-chat-memory-off')}</span>
            </label>
            <span class="ai-memory-hint">{t('memory-chat-toggle')}</span>
            {#if summarizing}
                <span class="ai-memory-hint">{t('memory-summary-generating')}</span>
            {/if}
        </div>

        {#if memoryLocked}
            <p class="ai-warn">{t('memory-chat-no-storage')} {t('memory-storage-locked')}</p>
        {:else if memoryUninitialized}
            <p class="ai-warn">{t('memory-needs-setup-hint')}</p>
        {:else if memoryStatus && !memoryStatus.memory.settings.enabled}
            <p class="ai-warn">{t('memory-settings-off-note')}</p>
        {/if}
        {#if memoryNotice}
            <p class="ai-warn">{memoryNotice}</p>
        {/if}
    </div>

    {#if actionError}
        <p class="ai-warn">{actionError}</p>
    {/if}

    <div class="ai-messages" bind:this={messageBox}>
        {#each chat.entries as entry, index (index)}
            <div class="ai-message" class:user={entry.role === "user"}>
                {#if entry.role === "assistant" && entry.thinking.length > 0}
                    <details class="ai-thinking-box">
                        <summary>{t('ai-chat-reasoning')}</summary>
                        <pre>{entry.thinking}</pre>
                    </details>
                {/if}
                <div class="ai-bubble">{entry.text}</div>
            </div>
        {/each}
        {#if chat.generating}
            <p class="ai-generating">{t('ai-chat-generating')}</p>
        {/if}
        {#if chat.cancelled}
            <p class="ai-warn">{t('ai-chat-cancelled')}</p>
            {#if memoryWritable}
                {#if partialAnswer && partialAnswer.text.trim().length > 0}
                    <p class="ai-memory-hint">{t('memory-answer-cancelled-partial')}</p>
                    <Button size="xs" color="gray" variant="outline" uppercase on:click={keepPartialAnswer}>
                        {t('memory-keep-partial')}
                    </Button>
                {:else}
                    <p class="ai-memory-hint">{t('memory-answer-cancelled')}</p>
                {/if}
            {/if}
        {/if}
        {#if !chat.generating && !chat.cancelled && chat.finishReason === null && chat.error === null && chat.entries.length > 0 && chat.entries[chat.entries.length - 1].text.trim().length === 0}
            <p class="ai-warn">{t('ai-chat-no-answer')}</p>
        {/if}
    </div>

    <!-- what the backend actually put into the request, without the system prompt -->
    {#if lastPlan}
        <div class="ai-memory">
            <button class="ai-link" on:click={() => (showSources = !showSources)}>
                {t('memory-context-title')} · {t('memory-context-facts-used')}: {lastPlan.used_facts.length}
            </button>
            {#if showSources}
                <div class="ai-sources">
                    <p class="ai-memory-hint">
                        {t('memory-context-estimated')}: {lastPlan.estimated_tokens}
                    </p>
                    <ul>
                        {#each lastPlan.sections as section, index (index)}
                            <li>
                                {t(contextSectionLabelKey(section))}
                            </li>
                        {/each}
                    </ul>
                    {#if lastPlan.used_facts.length === 0 && !lastPlan.summary_used}
                        <p class="ai-memory-hint">{t('memory-context-none')}</p>
                    {/if}
                    {#each lastPlan.used_facts as fact}
                        <p class="ai-source-fact">
                            <span class="ai-tag">{t(scopeLabelKey(fact.scope))}</span>
                            <span class="ai-tag">{t(categoryLabelKey(fact.category))}</span>
                            {fact.excerpt}
                        </p>
                    {/each}
                    {#if lastPlan.dropped_facts > 0}
                        <p class="ai-memory-hint">
                            {lastPlan.dropped_facts} {t('memory-context-dropped-facts')}
                        </p>
                    {/if}
                    {#if lastPlan.dropped_messages > 0}
                        <p class="ai-memory-hint">
                            {lastPlan.dropped_messages} {t('memory-context-dropped-messages')}
                        </p>
                    {/if}
                    {#each lastPlan.warnings as warning}
                        <p class="ai-warn">{t(contextWarningKey(warning.code))}</p>
                    {/each}
                </div>
            {/if}
        </div>
    {/if}

    <!-- memory candidates proposed for this conversation -->
    {#if candidates.length > 0}
        <div class="ai-memory">
            <span class="ai-memory-label">{t('memory-candidates')}</span>
            <p class="ai-memory-hint">{t('memory-candidate-scope-hint')}</p>
            {#each candidates as candidate (candidate.id)}
                <div class="ai-candidate">
                    <p class="ai-candidate-text">{candidate.content}</p>
                    <div class="ai-memory-row">
                        <span class="ai-tag">{t(categoryLabelKey(candidate.category))}</span>
                        <span class="ai-memory-hint">
                            {t('memory-candidate-confidence')}: {Math.round(candidate.confidence * 100)}%
                        </span>
                        <select
                            value={candidateScope(candidate)}
                            on:change={(event) => onCandidateScopeChange(candidate.id, event)}
                        >
                            {#each MEMORY_SCOPES as scope}
                                <option value={scope}>{t(scopeLabelKey(scope))}</option>
                            {/each}
                        </select>
                        <Button size="xs" color="lime" uppercase on:click={() => approveCandidate(candidate)}>
                            {t('memory-candidate-approve')}
                        </Button>
                        <Button size="xs" color="gray" variant="subtle" uppercase on:click={() => rejectCandidate(candidate)}>
                            {t('memory-candidate-reject')}
                        </Button>
                    </div>
                </div>
            {/each}
        </div>
    {:else if activeConversation && candidates.length === 0}
        <p class="ai-memory-hint">{t('memory-candidates-none')}</p>
    {/if}

    <!-- the secret filter asks before anything suspicious is remembered -->
    {#if secretWarning}
        <div class="ai-secret">
            <Text weight={600}>{t('memory-secret-warning-title')}</Text>
            <p class="ai-warn">{t('memory-secret-warning-body')}</p>
            <p class="ai-memory-hint">
                {secretWarning.kinds.map((kind) => t(secretKindLabelKey(kind))).join(", ")}
            </p>
            <div class="ai-memory-row">
                <Button size="xs" color="red" uppercase on:click={() => secretWarning?.retry()}>
                    {t('memory-secret-confirm')}
                </Button>
                <Button size="xs" color="gray" variant="subtle" uppercase on:click={() => (secretWarning = null)}>
                    {t('memory-secret-cancel')}
                </Button>
            </div>
        </div>
    {/if}

    {#if spellEnabled && draft.trim().length > 0}
        <SpellingPanel
            report={spellReport}
            status={spellStatus}
            undo={spellUndo}
            currentText={draft}
            checkedText={spellCheckedText}
            busy={spellBusy}
            actionError={spellError}
            allowImprove={improveOffered}
            on:apply={applyCorrections}
            on:ignore={ignoreWord}
            on:add={rememberWord}
            on:undo={undoCorrection}
            on:improve={openImprovement}
            on:reload={refreshSpelling}
        />
    {/if}

    {#if improveOpen && improveOffered}
        <TextImprovementPanel
            preview={improvePreview}
            text={draft}
            busy={improveBusy}
            actionError={improveError}
            on:request={requestImprovement}
            on:apply={applyImprovement}
            on:cancel={cancelImprovement}
            on:close={closeImprovement}
        />
    {/if}

    <div class="ai-input">
        <textarea
            bind:value={draft}
            placeholder={t('ai-chat-input')}
            rows="3"
            disabled={chat.generating}
            on:input={scheduleSpellCheck}
            on:keydown={(event) => {
                if (event.key === "Enter" && !event.shiftKey) {
                    event.preventDefault()
                    void send()
                }
            }}
        />
        <div class="ai-input-actions">
            {#if chat.generating}
                <Button size="xs" color="red" uppercase on:click={stopGeneration}>
                    {t('ai-chat-stop')}
                </Button>
            {:else}
                <Button size="xs" color="lime" uppercase on:click={send} disabled={!sendable}>
                    {t('ai-chat-send')}
                </Button>
            {/if}
            <span class="ai-meta">
                {#if chat.elapsedMs > 0}
                    {t('ai-chat-elapsed')} {formatDuration(chat.elapsedMs)}
                {/if}
                {#if formatUsage(chat.usage)}
                    · {t('ai-chat-tokens')} {formatUsage(chat.usage)}
                {/if}
            </span>
        </div>
    </div>
</div>

<style lang="scss">
.local-ai {
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
    padding: 1rem;
    background: rgba(20, 28, 32, 0.75);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 10px;
}

.ai-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 0.75rem;
    flex-wrap: wrap;
}

.ai-title {
    display: flex;
    align-items: center;
    gap: 0.5rem;
}

.ai-dot {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    background: rgba(255, 255, 255, 0.25);

    &.live {
        background: #52fefe;
        box-shadow: 0 0 8px rgba(82, 254, 254, 0.7);
    }
}

.ai-state {
    font-size: 0.7rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: rgba(255, 255, 255, 0.45);
}

.ai-controls {
    display: flex;
    gap: 0.4rem;
    flex-wrap: wrap;
}

.ai-note {
    margin: 0;
    font-size: 0.72rem;
    color: rgba(255, 255, 255, 0.45);
}

.ai-switches {
    display: flex;
    gap: 0.75rem;
    flex-wrap: wrap;
}

.ai-field {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    flex: 1 1 190px;

    span {
        font-size: 0.72rem;
        color: rgba(255, 255, 255, 0.7);
    }

    select {
        background: rgba(30, 40, 45, 0.9);
        color: #fff;
        border: 1px solid rgba(255, 255, 255, 0.12);
        border-radius: 6px;
        padding: 0.35rem 0.5rem;
        font-size: 0.8rem;
        font-family: inherit;
    }

    small {
        font-size: 0.65rem;
        color: rgba(255, 255, 255, 0.35);
    }
}

.ai-memory {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    padding: 0.5rem;
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 8px;
    background: rgba(0, 0, 0, 0.2);
}

.ai-memory-row {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    flex-wrap: wrap;

    select {
        background: rgba(30, 40, 45, 0.9);
        color: #fff;
        border: 1px solid rgba(255, 255, 255, 0.12);
        border-radius: 6px;
        padding: 0.25rem 0.4rem;
        font-size: 0.75rem;
        font-family: inherit;
    }
}

.ai-memory-label {
    font-size: 0.72rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: rgba(255, 255, 255, 0.55);
}

.ai-memory-hint {
    margin: 0;
    font-size: 0.68rem;
    color: rgba(255, 255, 255, 0.4);
}

.ai-check {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    font-size: 0.72rem;
    color: rgba(255, 255, 255, 0.7);

    input {
        accent-color: #52fefe;
    }
}

.ai-link {
    background: none;
    border: none;
    color: #52fefe;
    font-size: 0.72rem;
    text-align: left;
    cursor: pointer;
    padding: 0;
}

.ai-sources {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    font-size: 0.7rem;
    color: rgba(255, 255, 255, 0.55);

    ul {
        margin: 0;
        padding-left: 1.1rem;
    }
}

.ai-source-fact {
    margin: 0;
    color: rgba(255, 255, 255, 0.7);
}

.ai-tag {
    display: inline-block;
    margin-right: 0.3rem;
    padding: 0 0.3rem;
    border: 1px solid rgba(255, 255, 255, 0.15);
    border-radius: 4px;
    font-size: 0.6rem;
    text-transform: uppercase;
    color: rgba(255, 255, 255, 0.5);
}

.ai-candidate {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    padding: 0.35rem;
    border: 1px solid rgba(82, 254, 254, 0.2);
    border-radius: 6px;
}

.ai-candidate-text {
    margin: 0;
    font-size: 0.78rem;
    color: rgba(255, 255, 255, 0.8);
}

.ai-secret {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    padding: 0.5rem;
    border: 1px solid rgba(255, 107, 107, 0.4);
    border-radius: 8px;
    background: rgba(255, 107, 107, 0.08);
}

.ai-warn {
    margin: 0;
    font-size: 0.75rem;
    color: #ffb84d;
}

.ai-server {
    margin: 0;
    font-size: 0.72rem;
    color: rgba(255, 255, 255, 0.55);
    font-family: monospace;
}

.ai-capabilities {
    display: flex;
    gap: 0.5rem;
    flex-wrap: wrap;

    span {
        font-size: 0.62rem;
        text-transform: uppercase;
        letter-spacing: 0.04em;
        padding: 0.1rem 0.4rem;
        border-radius: 4px;
        border: 1px solid rgba(255, 255, 255, 0.12);
        color: rgba(255, 255, 255, 0.35);

        &.on {
            color: #52fefe;
            border-color: rgba(82, 254, 254, 0.4);
        }
    }
}

.ai-stderr {
    font-size: 0.7rem;
    color: rgba(255, 255, 255, 0.5);

    summary {
        cursor: pointer;
    }

    pre {
        max-height: 140px;
        overflow: auto;
        white-space: pre-wrap;
        word-break: break-all;
        margin: 0.35rem 0 0;
        padding: 0.4rem;
        background: rgba(0, 0, 0, 0.35);
        border-radius: 6px;
    }
}

.ai-messages {
    display: flex;
    flex-direction: column;
    gap: 0.45rem;
    max-height: 320px;
    min-height: 60px;
    overflow-y: auto;
    padding: 0.4rem;
    background: rgba(0, 0, 0, 0.25);
    border-radius: 8px;
}

.ai-message {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 0.25rem;

    &.user {
        align-items: flex-end;

        .ai-bubble {
            background: rgba(82, 254, 254, 0.12);
            border-color: rgba(82, 254, 254, 0.25);
        }
    }
}

.ai-bubble {
    max-width: 85%;
    padding: 0.45rem 0.6rem;
    border-radius: 8px;
    border: 1px solid rgba(255, 255, 255, 0.1);
    background: rgba(30, 40, 45, 0.85);
    font-size: 0.82rem;
    line-height: 1.35;
    white-space: pre-wrap;
    word-break: break-word;
}

.ai-thinking-box {
    max-width: 85%;
    font-size: 0.72rem;
    color: rgba(255, 255, 255, 0.5);

    summary {
        cursor: pointer;
    }

    pre {
        white-space: pre-wrap;
        word-break: break-word;
        margin: 0.3rem 0 0;
        max-height: 180px;
        overflow: auto;
        padding: 0.4rem;
        background: rgba(0, 0, 0, 0.35);
        border-radius: 6px;
    }
}

.ai-generating {
    margin: 0;
    font-size: 0.72rem;
    color: rgba(255, 255, 255, 0.5);
    animation: pulse 1.4s ease-in-out infinite;
}

@keyframes pulse {
    50% {
        opacity: 0.4;
    }
}

.ai-input {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;

    textarea {
        width: 100%;
        resize: vertical;
        background: rgba(30, 40, 45, 0.9);
        color: #fff;
        border: 1px solid rgba(255, 255, 255, 0.12);
        border-radius: 8px;
        padding: 0.5rem;
        font-family: inherit;
        font-size: 0.82rem;

        &:focus {
            outline: none;
            border-color: rgba(82, 254, 254, 0.4);
        }
    }
}

.ai-input-actions {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    flex-wrap: wrap;
}

.ai-meta {
    font-size: 0.68rem;
    color: rgba(255, 255, 255, 0.4);
}
</style>

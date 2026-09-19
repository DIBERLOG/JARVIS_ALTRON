<script lang="ts">
    /**
     * One conversation: its messages, its summary, and the candidates it produced.
     *
     * A message carries the status the backend stored, so a cancelled or failed answer
     * is shown as what it is instead of looking like a completed one. The summary is
     * regenerated on request, and only the page can ask the model, so this component
     * dispatches that intent.
     */
    import { createEventDispatcher } from "svelte"
    import { Button, Text } from "@svelteuidev/core"

    import type { ConversationDetails, MessageView } from "@/lib/memory-model"
    import {
        answerNoteKey,
        candidateStateLabelKey,
        messageStatusLabelKey,
        scopeLabelKey
    } from "@/lib/memory-model"
    import { translations, translate } from "@/stores"

    export let details: ConversationDetails
    export let summarizing = false
    export let busy = false

    const dispatch = createEventDispatcher<{ summarize: void; close: void }>()

    $: t = (key: string) => translate($translations, key)
    $: conversation = details.conversation
    $: summary = details.summary
    $: messages = details.page.messages
    $: candidates = details.candidates
    /** The suggested entries of this conversation that already became memory. */
    $: approvedCandidates = candidates.filter((candidate) => candidate.state === "approved")

    function roleLabel(message: MessageView): string {
        return message.role === "user" ? t('memory-message-you') : t('memory-message-assistant')
    }

    function noteLabel(message: MessageView): string | null {
        const key = answerNoteKey(message.status, message.partial)
        return key ? t(key) : null
    }
</script>

<div class="detail">
    <div class="detail-head">
        <span class="panel-title">{t('memory-conversations')}</span>
        <span class="message-count">{messages.length} · {details.page.total}</span>
        <Button size="xs" color="gray" uppercase on:click={() => dispatch('close')}>
            {t('memory-secret-cancel')}
        </Button>
    </div>

    <div class="summary">
        <div class="summary-head">
            <span class="sub-title">{t('memory-summary')}</span>
            <Button size="xs" color="cyan" uppercase disabled={busy || summarizing} on:click={() => dispatch('summarize')}>
                {t('memory-summary-regenerate')}
            </Button>
        </div>

        {#if summarizing}
            <Text size="xs" color="gray">{t('memory-summary-generating')}</Text>
        {:else if summary}
            {#if summary.stale}
                <Text size="xs" color="gray">{t('memory-summary-stale')}</Text>
            {/if}
            <div class="summary-text">{summary.summary}</div>
        {:else}
            <Text size="xs" color="gray">{t('memory-summary-none')}</Text>
        {/if}
    </div>

    <div class="messages">
        {#if messages.length === 0}
            <Text size="xs" color="gray">{t('memory-context-none')}</Text>
        {:else}
            {#each messages as message (message.id)}
                <div class="message" class:user={message.role === "user"}>
                    <div class="message-head">
                        <span class="role">{roleLabel(message)}</span>
                        <span class="status">{t(messageStatusLabelKey(message.status))}</span>
                        <span class="time">{message.created_at}</span>
                    </div>
                    <div class="message-body">{message.content}</div>
                    {#if noteLabel(message)}
                        <div class="note">{noteLabel(message)}</div>
                    {/if}
                    {#if message.content.length === 0}
                        <div class="note">{t('memory-not-saved')}</div>
                    {/if}
                    {#if message.partial}
                        <div class="note">{t('memory-keep-partial')}</div>
                    {/if}
                </div>
            {/each}
        {/if}
    </div>

    <div class="candidates">
        <span class="sub-title">{t('memory-candidates')}</span>
        <Text size="xs" color="gray">{t('memory-chat-facts-used')}: {String(approvedCandidates.length)}</Text>
        {#if candidates.length === 0}
            <Text size="xs" color="gray">{t('memory-candidates-none')}</Text>
        {:else}
            {#each candidates as candidate (candidate.id)}
                <div class="candidate">
                    <span class="tag">{t(candidateStateLabelKey(candidate.state))}</span>
                    <span class="tag">{t(scopeLabelKey(candidate.scope))}</span>
                    <span class="candidate-text">{candidate.content}</span>
                </div>
            {/each}
        {/if}
    </div>

    <Text size="xs" color="gray">{t('memory-profile-switch')}</Text>

    <Text size="xs" color="gray">{t('memory-context-dropped-messages')}</Text>
</div>

<style lang="scss">
    .detail {
        background: rgba(20, 30, 35, 0.5);
        border: 1px solid rgba(82, 254, 254, 0.2);
        border-radius: 10px;
        display: flex;
        flex-direction: column;
        gap: 8px;
        min-width: 0;
        padding: 10px;
    }

    .detail-head,
    .summary-head {
        align-items: center;
        display: flex;
        gap: 6px;
    }

    .panel-title,
    .sub-title {
        color: #ffffff;
        font-size: 0.7rem;
        letter-spacing: 0.08em;
        text-transform: uppercase;
    }

    .sub-title {
        flex: 1;
        font-size: 0.64rem;
    }

    .message-count {
        background: rgba(82, 254, 254, 0.15);
        border-radius: 8px;
        color: #52fefe;
        font-size: 0.6rem;
        margin-right: auto;
        padding: 1px 7px;
    }

    .summary {
        background: rgba(15, 22, 26, 0.6);
        border-radius: 8px;
        display: flex;
        flex-direction: column;
        gap: 4px;
        padding: 7px 8px;
    }

    .summary-text {
        color: rgba(255, 255, 255, 0.8);
        font-size: 0.7rem;
        line-height: 1.4;
        white-space: pre-wrap;
    }

    .messages {
        display: flex;
        flex-direction: column;
        gap: 5px;
        max-height: 320px;
        overflow-y: auto;
    }

    .message {
        background: rgba(15, 22, 26, 0.6);
        border-left: 2px solid rgba(140, 120, 255, 0.5);
        border-radius: 6px;
        padding: 5px 8px;

        &.user {
            border-left-color: rgba(82, 254, 254, 0.5);
        }
    }

    .message-head {
        align-items: center;
        color: rgba(255, 255, 255, 0.45);
        display: flex;
        font-size: 0.56rem;
        gap: 8px;
    }

    .role {
        color: rgba(255, 255, 255, 0.7);
        letter-spacing: 0.04em;
        text-transform: uppercase;
    }

    .message-body {
        color: rgba(255, 255, 255, 0.85);
        font-size: 0.7rem;
        line-height: 1.4;
        margin-top: 3px;
        white-space: pre-wrap;
        word-break: break-word;
    }

    .note {
        color: #ffb347;
        font-size: 0.58rem;
        margin-top: 3px;
    }

    .candidates {
        display: flex;
        flex-direction: column;
        gap: 4px;
    }

    .candidate {
        align-items: center;
        display: flex;
        flex-wrap: wrap;
        gap: 5px;
        background: rgba(15, 22, 26, 0.6);
        border-radius: 6px;
        padding: 5px 8px;
    }

    .candidate-text {
        color: rgba(255, 255, 255, 0.8);
        font-size: 0.68rem;
    }

    .tag {
        background: rgba(82, 254, 254, 0.12);
        border-radius: 8px;
        color: rgba(82, 254, 254, 0.85);
        font-size: 0.56rem;
        padding: 0 6px;
    }
</style>

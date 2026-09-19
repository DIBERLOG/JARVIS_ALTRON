<script lang="ts">
    /**
     * The entry form, used for a new memory entry and for editing an existing one.
     *
     * The component never calls the backend: it builds a `FactDraft` and lets the page
     * decide, because only the page can answer the secret confirmation and refresh the
     * list afterwards.
     */
    import { createEventDispatcher } from "svelte"
    import { Button, Switch, Text } from "@svelteuidev/core"

    import type { FactDraft, FactView, MemoryCategory, MemoryScope } from "@/lib/memory-model"
    import {
        MEMORY_CATEGORIES,
        MEMORY_SCOPES,
        categoryLabelKey,
        sourceLabelKey,
        scopeLabelKey
    } from "@/lib/memory-model"
    import { translations, translate } from "@/stores"

    /** `null` creates a new entry. */
    export let fact: FactView | null = null
    export let defaultScope: MemoryScope = "jarvis"
    export let defaultCategory: MemoryCategory = "personal_fact"
    export let busy = false

    const dispatch = createEventDispatcher<{ save: FactDraft; cancel: void }>()

    $: t = (key: string) => translate($translations, key)

    let scope: MemoryScope = fact ? fact.scope : defaultScope
    let category: MemoryCategory = fact ? fact.category : defaultCategory
    let content = fact ? fact.content : ""
    let pinned = fact ? fact.pinned : false
    let disabled = fact ? fact.disabled : false

    // A different entry means a different form: reopening the editor must not keep the
    // previous text.
    $: currentId = fact ? fact.id : null
    let loadedId: string | null = fact ? fact.id : null
    $: if (currentId !== loadedId) {
        loadedId = currentId
        scope = fact ? fact.scope : defaultScope
        category = fact ? fact.category : defaultCategory
        content = fact ? fact.content : ""
        pinned = fact ? fact.pinned : false
        disabled = fact ? fact.disabled : false
    }

    function onScope(event: Event) {
        scope = (event.currentTarget as HTMLSelectElement).value as MemoryScope
    }

    function onCategory(event: Event) {
        category = (event.currentTarget as HTMLSelectElement).value as MemoryCategory
    }

    function submit() {
        if (content.trim().length === 0) return
        dispatch("save", {
            scope,
            category,
            content,
            pinned,
            disabled,
            // The page adds this flag only after an explicit confirmation.
            accept_secret_warning: false
        })
    }
</script>

<div class="editor">
    <div class="editor-head">
        <span class="panel-title">{fact ? t('memory-fact-edit') : t('memory-fact-new')}</span>
    </div>

    <div class="fields">
        <label class="field">
            <Text size="xs" color="gray">{t('memory-fact-scope')}</Text>
            <select value={scope} on:change={onScope}>
                {#each MEMORY_SCOPES as option}
                    <option value={option}>{t(scopeLabelKey(option))}</option>
                {/each}
            </select>
        </label>

        <label class="field">
            <Text size="xs" color="gray">{t('memory-fact-category')}</Text>
            <select value={category} on:change={onCategory}>
                {#each MEMORY_CATEGORIES as option}
                    <option value={option}>{t(categoryLabelKey(option))}</option>
                {/each}
            </select>
        </label>
    </div>

    <label class="field">
        <Text size="xs" color="gray">{t('memory-fact-content')}</Text>
        <textarea
            class="content"
            spellcheck="false"
            placeholder={t('memory-fact-content')}
            bind:value={content}
        ></textarea>
    </label>

    <Text size="xs" color="gray">{t('memory-secret-filter-note')}</Text>

    <div class="switches">
        <Switch label={t('memory-fact-pinned')} bind:checked={pinned} />
        <Switch label={t('memory-fact-disabled')} bind:checked={disabled} />
    </div>

    {#if disabled}
        <Text size="xs" color="gray">{t('memory-fact-disabled-hint')}</Text>
    {/if}

    {#if fact}
        <Text size="xs" color="gray">
            {t('memory-fact-source')}: {t(sourceLabelKey(fact.source))}
        </Text>
    {/if}

    <div class="actions">
        <Button size="xs" color="lime" uppercase disabled={busy || content.trim().length === 0} on:click={submit}>
            {t('memory-fact-save')}
        </Button>
        <Button size="xs" color="gray" uppercase on:click={() => dispatch('cancel')}>
            {t('memory-secret-cancel')}
        </Button>
    </div>
</div>

<style lang="scss">
    .editor {
        background: rgba(20, 30, 35, 0.6);
        border: 1px solid rgba(82, 254, 254, 0.2);
        border-radius: 10px;
        display: flex;
        flex-direction: column;
        gap: 6px;
        padding: 10px;
    }

    .editor-head {
        align-items: center;
        display: flex;
        gap: 6px;
    }

    .panel-title {
        color: #ffffff;
        flex: 1;
        font-size: 0.68rem;
        letter-spacing: 0.08em;
        text-transform: uppercase;
    }

    .fields {
        display: grid;
        gap: 6px;
        grid-template-columns: 1fr 1fr;
    }

    .field {
        display: flex;
        flex-direction: column;
        gap: 3px;

        select,
        textarea {
            width: 100%;
            background: rgba(10, 18, 22, 0.75);
            border: 1px solid rgba(255, 255, 255, 0.1);
            border-radius: 6px;
            color: #ffffff;
            font-size: 0.68rem;
            padding: 6px 7px;

            &:focus {
                outline: none;
                border-color: rgba(82, 254, 254, 0.5);
            }
        }
    }

    .content {
        min-height: 84px;
        resize: vertical;
    }

    .switches {
        display: flex;
        flex-wrap: wrap;
        gap: 14px;
    }

    .actions {
        display: flex;
        gap: 6px;
    }
</style>

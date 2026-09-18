<script lang="ts">
    import { createEventDispatcher } from "svelte"
    import { Button, Switch, Text } from "@svelteuidev/core"
    import { ArrowLeft, Check, Trash, Update } from "radix-icons-svelte"

    import type { Note, NoteDraft, NoteFolder, SaveState } from "@/lib/notes-model"
    import {
        folderOptions,
        formatAbsoluteDate,
        formatTagInput,
        parseTagInput,
        relativeTime,
        saveIndicatorKey,
        timeUnitKey
    } from "@/lib/notes-model"
    import { translations, translate } from "@/stores"

    export let note: Note
    export let draft: NoteDraft
    export let folders: NoteFolder[] = []
    export let saveState: SaveState = "idle"
    export let busy = false

    const dispatch = createEventDispatcher<{
        change: NoteDraft
        save: void
        back: void
        pin: boolean
        trash: void
        restore: void
        purge: void
    }>()

    $: t = (key: string) => translate($translations, key)
    $: folderChoices = folderOptions(folders, t("notes-none"))
    $: updatedLabel = relativeTime(note.updated_at)
    $: pinned = note.pinned

    // Local text state for the tag field: re-parsing on every keystroke would
    // delete a separator the user just typed.
    let tagsText = formatTagInput(draft.tags)
    let confirmingPurge = false
    let confirmTimer: ReturnType<typeof setTimeout> | null = null

    function emit(patch: Partial<NoteDraft>) {
        dispatch("change", { ...draft, ...patch })
    }

    function onTagsInput(event: Event) {
        const value = (event.currentTarget as HTMLInputElement).value
        tagsText = value
        emit({ tags: parseTagInput(value) })
    }

    function onTitleInput(event: Event) {
        emit({ title: (event.currentTarget as HTMLInputElement).value })
    }

    function onBodyInput(event: Event) {
        emit({ body: (event.currentTarget as HTMLTextAreaElement).value })
    }

    function onFolderChange(event: Event) {
        const value = (event.currentTarget as HTMLSelectElement).value
        emit({ folder_id: value.length > 0 ? value : null })
    }

    function requestPurge() {
        confirmingPurge = true
        if (confirmTimer) clearTimeout(confirmTimer)
        confirmTimer = setTimeout(() => (confirmingPurge = false), 5000)
    }

    function confirmPurge() {
        if (confirmTimer) clearTimeout(confirmTimer)
        confirmingPurge = false
        dispatch("purge")
    }
</script>

<div class="editor">
    <div class="editor-bar">
        <button class="back" on:click={() => dispatch('back')} title={t('notes-back')}>
            <ArrowLeft size={14} />
        </button>

        <span class="state" class:error={saveState === 'error'} class:saving={saveState === 'saving'}>
            {#if saveState === "saved"}
                <Check size={12} />
            {/if}
            {t(saveIndicatorKey(saveState, saveState === "dirty" || saveState === "error"))}
        </span>

        <Switch
            label={pinned ? t("notes-pinned") : t("notes-pin")}
            bind:checked={pinned}
            on:change={() => dispatch("pin", pinned)}
        />
    </div>

    <input
        class="title"
        type="text"
        placeholder={t('notes-untitled')}
        value={draft.title}
        on:input={onTitleInput}
        on:blur={() => dispatch('save')}
    />

    <textarea
        class="body"
        placeholder={t('notes-body-placeholder')}
        spellcheck="false"
        value={draft.body}
        on:input={onBodyInput}
        on:blur={() => dispatch('save')}
    ></textarea>

    <div class="field">
        <Text size="xs" color="gray">{t('notes-folder')}</Text>
        <select class="select" value={draft.folder_id ?? ""} on:change={onFolderChange}>
            {#each folderChoices as choice}
                <option value={choice.value}>{choice.label}</option>
            {/each}
        </select>
    </div>

    <div class="field">
        <Text size="xs" color="gray">{t('notes-tags')}</Text>
        <input
            class="tags-input"
            type="text"
            placeholder={t('notes-tags-placeholder')}
            value={tagsText}
            on:input={onTagsInput}
            on:blur={() => dispatch('save')}
        />
    </div>

    <div class="dates">
        <Text size="xs" color="gray">
            {t('notes-updated')}: {formatAbsoluteDate(note.updated_at)}
            {#if updatedLabel.unit !== "date"}
                - {updatedLabel.unit === "now" ? t("notes-time-now") : `${updatedLabel.value} ${t(timeUnitKey(updatedLabel.unit))}`}
            {/if}
        </Text>
        <Text size="xs" color="gray">{t('notes-revision')}: {note.revision}</Text>
    </div>

    <div class="actions">
        {#if note.deleted_at}
            <Button size="xs" color="cyan" uppercase on:click={() => dispatch('restore')}>
                <Update size={13} />
                {t('notes-restore')}
            </Button>
            {#if confirmingPurge}
                <Button size="xs" color="red" uppercase on:click={confirmPurge}>
                    {t('notes-delete-confirm')}
                </Button>
            {:else}
                <Button size="xs" color="gray" uppercase on:click={requestPurge}>
                    <Trash size={13} />
                    {t('notes-delete-forever')}
                </Button>
            {/if}
        {:else}
            <Button size="xs" color="gray" uppercase on:click={() => dispatch('trash')}>
                <Trash size={13} />
                {t('notes-trash')}
            </Button>
        {/if}

        <Button size="xs" color="lime" uppercase on:click={() => dispatch('save')} disabled={busy}>
            {t('notes-save-now')}
        </Button>
    </div>
</div>

<style lang="scss">
    .editor {
        display: flex;
        flex-direction: column;
        gap: 6px;
    }

    .editor-bar {
        display: flex;
        align-items: center;
        gap: 8px;
    }

    .back {
        background: rgba(35, 50, 55, 0.7);
        border: none;
        border-radius: 6px;
        color: #ffffff;
        cursor: pointer;
        display: flex;
        padding: 5px 7px;

        &:hover {
            background: rgba(82, 254, 254, 0.15);
            color: #52fefe;
        }
    }

    .state {
        display: inline-flex;
        align-items: center;
        gap: 4px;
        font-size: 0.6rem;
        letter-spacing: 0.04em;
        text-transform: uppercase;
        color: rgba(120, 255, 170, 0.85);
        flex: 1;

        &.saving {
            color: rgba(255, 214, 102, 0.9);
        }

        &.error {
            color: #ff6b6b;
        }
    }

    .title {
        width: 100%;
        background: rgba(10, 18, 22, 0.75);
        border: 1px solid rgba(255, 255, 255, 0.1);
        border-radius: 6px;
        color: #ffffff;
        font-size: 0.85rem;
        font-weight: 500;
        padding: 8px;

        &:focus {
            outline: none;
            border-color: rgba(82, 254, 254, 0.5);
        }
    }

    .body {
        width: 100%;
        min-height: 200px;
        resize: vertical;
        background: rgba(10, 18, 22, 0.75);
        border: 1px solid rgba(255, 255, 255, 0.1);
        border-radius: 6px;
        color: rgba(255, 255, 255, 0.9);
        font-size: 0.72rem;
        line-height: 1.5;
        padding: 8px;

        &:focus {
            outline: none;
            border-color: rgba(82, 254, 254, 0.5);
        }
    }

    .field {
        display: flex;
        flex-direction: column;
        gap: 3px;
    }

    .select,
    .tags-input {
        width: 100%;
        background: rgba(10, 18, 22, 0.75);
        border: 1px solid rgba(255, 255, 255, 0.1);
        border-radius: 6px;
        color: #ffffff;
        font-size: 0.68rem;
        padding: 6px 8px;

        &:focus {
            outline: none;
            border-color: rgba(82, 254, 254, 0.5);
        }
    }

    .dates {
        display: flex;
        justify-content: space-between;
        gap: 8px;
    }

    .actions {
        display: flex;
        gap: 6px;
        flex-wrap: wrap;
        margin-top: 2px;
    }
</style>

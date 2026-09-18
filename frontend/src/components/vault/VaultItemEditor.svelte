<script lang="ts">
    import { createEventDispatcher, onDestroy } from "svelte"
    import { Button, Switch, Text } from "@svelteuidev/core"
    import {
        ArrowLeft,
        Check,
        Clipboard,
        EyeClosed,
        EyeOpen,
        Person,
        Trash,
        Update
    } from "radix-icons-svelte"

    import VaultGenerator from "@/components/vault/VaultGenerator.svelte"
    import type { SaveState, VaultItemDetails, VaultMetadataDraft } from "@/lib/vault-model"
    import {
        MASK_CHARACTER,
        formatUrlInput,
        hiddenSecrets,
        itemTitle,
        maskedSecret,
        parseUrlInput,
        saveIndicatorKey
    } from "@/lib/vault-model"
    import { translations, translate } from "@/stores"

    export let item: VaultItemDetails
    export let metadata: VaultMetadataDraft
    export let saveState: SaveState = "idle"
    export let busy = false
    /** Whether the user explicitly asked to see the secret. */
    export let revealed = false
    /** Revealed secret values; empty while hidden. */
    export let secrets: { password: string; notes: string } = hiddenSecrets()
    export let clipboardTimeout = 30
    export let clipboardArmed = false
    export let clipboardRemaining = 0

    const dispatch = createEventDispatcher<{
        metadata: VaultMetadataDraft
        secrets: { password: string; notes: string }
        reveal: void
        hide: void
        save: void
        back: void
        copyUsername: void
        copyPassword: void
        clipboardGenerated: void
        clipboardClear: void
        trash: void
        restore: void
        purge: void
    }>()

    $: t = (key: string) => translate($translations, key)
    $: urlText = formatUrlInput(metadata.urls)
    $: passwordField = revealed ? secrets.password : maskedSecret(secrets.password, false)
    $: passwordLength = revealed ? secrets.password.length : 0

    let confirmingPurge = false
    let confirmTimer: ReturnType<typeof setTimeout> | null = null

    onDestroy(() => {
        if (confirmTimer) clearTimeout(confirmTimer)
        // Leaving the editor must not keep a revealed secret in the parent state.
        if (revealed) dispatch("hide")
    })

    function emit(patch: Partial<VaultMetadataDraft>) {
        dispatch("metadata", { ...metadata, ...patch })
    }

    function onNameInput(event: Event) {
        emit({ name: (event.currentTarget as HTMLInputElement).value })
    }

    function onUsernameInput(event: Event) {
        emit({ username: (event.currentTarget as HTMLInputElement).value })
    }

    function onUrlsInput(event: Event) {
        emit({ urls: parseUrlInput((event.currentTarget as HTMLTextAreaElement).value) })
    }

    function onTagsInput(event: Event) {
        const tags = (event.currentTarget as HTMLInputElement).value
        emit({ tags: tags.split(/[,\n]/).map((tag) => tag.trim()).filter((tag) => tag.length > 0) })
    }

    function onPasswordInput(event: Event) {
        dispatch("secrets", { ...secrets, password: (event.currentTarget as HTMLInputElement).value })
    }

    function onNotesInput(event: Event) {
        dispatch("secrets", { ...secrets, notes: (event.currentTarget as HTMLTextAreaElement).value })
    }

    function onGenerated(password: string) {
        // The generated password is placed in the editor and stays unsaved until
        // the user saves the item.
        revealed = true
        dispatch("secrets", { ...secrets, password })
    }

    /** A password generated straight onto the clipboard: nothing to show here. */
    function onGeneratedCopy() {
        dispatch("clipboardGenerated")
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
        <button class="back" on:click={() => dispatch('back')} title={t('vault-back')}>
            <ArrowLeft size={14} />
        </button>
        <span class="state" class:error={saveState === 'error'} class:saving={saveState === 'saving'}>
            {#if saveState === "saved"}
                <Check size={12} />
            {/if}
            {t(saveIndicatorKey(saveState, saveState === "dirty" || saveState === "error"))}
        </span>
        <Switch
            label={metadata.favorite ? t("vault-favorited") : t("vault-favorite")}
            bind:checked={metadata.favorite}
            on:change={() => emit({ favorite: metadata.favorite })}
        />
    </div>

    <input
        class="field-input title"
        type="text"
        placeholder={t('vault-name-placeholder')}
        value={metadata.name}
        on:input={onNameInput}
        on:blur={() => dispatch('save')}
    />

    <div class="row">
        <input
            class="field-input"
            type="text"
            placeholder={t('vault-username-placeholder')}
            value={metadata.username}
            on:input={onUsernameInput}
            on:blur={() => dispatch('save')}
        />
        <button class="icon-button" title={t('vault-copy-username')} on:click={() => dispatch('copyUsername')}>
            <Person size={13} />
            <Clipboard size={11} />
        </button>
    </div>

    <div class="row">
        <input
            class="field-input password"
            type="text"
            readonly={!revealed}
            placeholder={t('vault-password-placeholder')}
            value={passwordField}
            on:input={onPasswordInput}
            on:blur={() => dispatch('save')}
        />
        <button
            class="icon-button"
            title={revealed ? t('vault-hide') : t('vault-reveal')}
            on:click={() => (revealed ? dispatch('hide') : dispatch('reveal'))}
        >
            {#if revealed}
                <EyeClosed size={13} />
            {:else}
                <EyeOpen size={13} />
            {/if}
        </button>
        <button class="icon-button" title={t('vault-copy-password')} on:click={() => dispatch('copyPassword')}>
            <Clipboard size={13} />
        </button>
    </div>

    {#if !revealed}
        <Text size="xs" color="gray">
            {t('vault-secret-hidden')} {MASK_CHARACTER.repeat(3)}
        </Text>
    {:else}
        <Text size="xs" color="gray">{t('vault-password-length')}: {passwordLength}</Text>
    {/if}

    <div class="field">
        <Text size="xs" color="gray">{t('vault-urls')}</Text>
        <textarea
            class="field-input urls"
            placeholder={t('vault-urls-placeholder')}
            spellcheck="false"
            value={urlText}
            on:input={onUrlsInput}
            on:blur={() => dispatch('save')}
        ></textarea>
    </div>

    <div class="field">
        <Text size="xs" color="gray">{t('vault-tags')}</Text>
        <input
            class="field-input"
            type="text"
            placeholder={t('vault-tags-placeholder')}
            value={metadata.tags.join(", ")}
            on:input={onTagsInput}
            on:blur={() => dispatch('save')}
        />
    </div>

    <div class="field">
        <Text size="xs" color="gray">{t('vault-notes')}</Text>
        <textarea
            class="field-input notes"
            placeholder={revealed ? t('vault-notes-placeholder') : t('vault-notes-hidden')}
            readonly={!revealed}
            spellcheck="false"
            value={revealed ? secrets.notes : ""}
            on:input={onNotesInput}
            on:blur={() => dispatch('save')}
        ></textarea>
    </div>

    {#if clipboardArmed}
        <div class="clipboard-bar">
            <Clipboard size={12} />
            <Text size="xs" color="gray">
                {t('vault-clipboard-armed')} · {clipboardRemaining}
            </Text>
            <button class="clear-button" on:click={() => dispatch('clipboardClear')}>
                {t('vault-clipboard-clear')}
            </button>
        </div>
    {/if}

    <VaultGenerator
        {busy}
        {clipboardTimeout}
        on:use={(event) => onGenerated(event.detail)}
        on:copied={onGeneratedCopy}
    />

    <div class="actions">
        {#if item.deleted_at}
            <Button size="xs" color="cyan" uppercase on:click={() => dispatch('restore')}>
                <Update size={13} />
                {t('vault-restore')}
            </Button>
            {#if confirmingPurge}
                <Button size="xs" color="red" uppercase on:click={confirmPurge}>
                    {t('vault-delete-confirm')}
                </Button>
            {:else}
                <Button size="xs" color="gray" uppercase on:click={requestPurge}>
                    <Trash size={13} />
                    {t('vault-delete-forever')}
                </Button>
            {/if}
        {:else}
            <Button size="xs" color="gray" uppercase on:click={() => dispatch('trash')}>
                <Trash size={13} />
                {t('vault-trash')}
            </Button>
        {/if}

        <Button size="xs" color="lime" uppercase on:click={() => dispatch('save')} disabled={busy}>
            {t('vault-save-now')}
        </Button>
    </div>

    <Text size="xs" color="gray">{itemTitle(item.name, t('vault-untitled'))} · r{item.revision}</Text>
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

    .row {
        display: flex;
        align-items: center;
        gap: 4px;
    }

    .field {
        display: flex;
        flex-direction: column;
        gap: 3px;
    }

    .field-input {
        width: 100%;
        background: rgba(10, 18, 22, 0.75);
        border: 1px solid rgba(255, 255, 255, 0.1);
        border-radius: 6px;
        color: #ffffff;
        font-size: 0.7rem;
        padding: 7px 8px;

        &:focus {
            outline: none;
            border-color: rgba(82, 254, 254, 0.5);
        }

        &[readonly] {
            color: rgba(255, 255, 255, 0.45);
            letter-spacing: 0.1em;
        }
    }

    .title {
        font-size: 0.85rem;
        font-weight: 500;
    }

    .password {
        font-family: monospace;
    }

    .urls {
        min-height: 46px;
        resize: vertical;
        font-size: 0.66rem;
    }

    .notes {
        min-height: 74px;
        resize: vertical;
        font-size: 0.68rem;
    }

    .icon-button {
        display: inline-flex;
        align-items: center;
        gap: 2px;
        background: rgba(35, 50, 55, 0.7);
        border: none;
        border-radius: 6px;
        color: rgba(255, 255, 255, 0.75);
        cursor: pointer;
        padding: 7px 8px;

        &:hover {
            background: rgba(82, 254, 254, 0.15);
            color: #52fefe;
        }
    }

    .clipboard-bar {
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

    .actions {
        display: flex;
        gap: 6px;
        flex-wrap: wrap;
        margin-top: 2px;
    }
</style>

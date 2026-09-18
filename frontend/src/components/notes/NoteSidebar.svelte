<script lang="ts">
    import { createEventDispatcher } from "svelte"
    import { Button, Input } from "@svelteuidev/core"
    import { MagnifyingGlass, Pencil1, Plus, Star, StarFilled, Trash } from "radix-icons-svelte"

    import type { NoteFolder, NoteQuery, TrashFilter } from "@/lib/notes-model"
    import { TRASH_FILTERS, trashOptionKey } from "@/lib/notes-model"
    import { translations, translate } from "@/stores"

    export let query: NoteQuery
    export let folders: NoteFolder[]

    const dispatch = createEventDispatcher<{
        query: NoteQuery
        createFolder: string
        renameFolder: { id: string; name: string }
        purgeFolder: string
    }>()

    $: t = (key: string) => translate($translations, key)
    $: activeFolders = folders.filter((folder) => !folder.deleted_at)
    $: selectedFolder = activeFolders.find((folder) => folder.id === query.folder_id) ?? null

    let folderFormOpen = false
    let folderName = ""
    let renameOpen = false
    let renameValue = ""

    function update(patch: Partial<NoteQuery>) {
        dispatch("query", { ...query, ...patch })
    }

    function selectTrash(filter: TrashFilter) {
        update({ trash: filter })
    }

    function selectFolder(id: string | null) {
        renameOpen = false
        update({ folder_id: id })
    }

    function submitFolder() {
        const name = folderName.trim()
        if (name.length === 0) return
        dispatch("createFolder", name)
        folderName = ""
        folderFormOpen = false
    }

    function submitRename() {
        if (!selectedFolder) return
        const name = renameValue.trim()
        if (name.length === 0) return
        dispatch("renameFolder", { id: selectedFolder.id, name })
        renameOpen = false
    }

    function openRename() {
        if (!selectedFolder) return
        renameValue = selectedFolder.name
        renameOpen = true
    }

    function onSearchInput(event: Event) {
        update({ search: (event.currentTarget as HTMLInputElement).value })
    }
</script>

<div class="sidebar">
    <div class="row search-row">
        <span class="search-icon"><MagnifyingGlass size={14} /></span>
        <input
            class="search-input"
            type="text"
            placeholder={t('notes-search')}
            value={query.search}
            on:input={onSearchInput}
        />
    </div>

    <div class="chips">
        {#each TRASH_FILTERS as filter}
            <button
                class="chip"
                class:active={query.trash === filter && !query.folder_id && !query.tag}
                on:click={() => selectTrash(filter)}
            >
                {t(trashOptionKey(filter))}
            </button>
        {/each}

        <button
            class="chip"
            class:active={query.pinned_first}
            title={t('notes-pinned')}
            on:click={() => update({ pinned_first: !query.pinned_first })}
        >
            {#if query.pinned_first}
                <StarFilled size={12} />
            {:else}
                <Star size={12} />
            {/if}
        </button>
    </div>

    <div class="chips folders">
        <button
            class="chip folder-chip"
            class:active={query.folder_id === null}
            on:click={() => selectFolder(null)}
        >
            {t('notes-all-folders')}
        </button>
        {#each activeFolders as folder}
            <button
                class="chip folder-chip"
                class:active={query.folder_id === folder.id}
                on:click={() => selectFolder(folder.id)}
            >
                {folder.name}
            </button>
        {/each}
        <button class="chip icon-only" title={t('notes-new-folder')} on:click={() => (folderFormOpen = !folderFormOpen)}>
            <Plus size={12} />
        </button>
    </div>

    {#if folderFormOpen}
        <div class="row inline-form">
            <Input
                size="xs"
                placeholder={t('notes-folder-name')}
                variant="filled"
                bind:value={folderName}
            />
            <Button size="xs" color="cyan" uppercase on:click={submitFolder}>{t('notes-create')}</Button>
        </div>
    {/if}

    {#if selectedFolder}
        <div class="row folder-tools">
            <span class="folder-name">{selectedFolder.name}</span>
            <button class="link" on:click={openRename} title={t('notes-rename')}>
                <Pencil1 size={13} />
            </button>
            <button
                class="link danger"
                title={t('notes-delete-folder')}
                on:click={() => dispatch('purgeFolder', selectedFolder.id)}
            >
                <Trash size={13} />
            </button>
        </div>
        {#if renameOpen}
            <div class="row inline-form">
                <Input size="xs" variant="filled" bind:value={renameValue} />
                <Button size="xs" color="cyan" uppercase on:click={submitRename}>{t('notes-rename')}</Button>
            </div>
        {/if}
    {/if}
</div>

<style lang="scss">
    .sidebar {
        display: flex;
        flex-direction: column;
        gap: 6px;
        margin-bottom: 10px;
    }

    .row {
        display: flex;
        align-items: center;
        gap: 6px;
    }

    .search-row {
        position: relative;
    }

    .search-icon {
        position: absolute;
        left: 8px;
        top: 50%;
        transform: translateY(-50%);
        color: rgba(255, 255, 255, 0.4);
        display: flex;
    }

    .search-input {
        width: 100%;
        background: rgba(10, 18, 22, 0.75);
        border: 1px solid rgba(255, 255, 255, 0.1);
        border-radius: 6px;
        color: #ffffff;
        font-size: 0.72rem;
        padding: 7px 8px 7px 28px;

        &::placeholder {
            color: rgba(255, 255, 255, 0.35);
        }

        &:focus {
            outline: none;
            border-color: rgba(82, 254, 254, 0.5);
        }
    }

    .chips {
        display: flex;
        flex-wrap: wrap;
        gap: 4px;
    }

    .chip {
        display: inline-flex;
        align-items: center;
        gap: 4px;
        background: rgba(35, 50, 55, 0.7);
        border: 1px solid transparent;
        border-radius: 12px;
        color: rgba(255, 255, 255, 0.7);
        font-size: 0.62rem;
        letter-spacing: 0.03em;
        padding: 3px 9px;
        cursor: pointer;
        max-width: 140px;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;

        &:hover {
            background: rgba(82, 254, 254, 0.12);
            color: #ffffff;
        }

        &.active {
            background: rgba(82, 254, 254, 0.18);
            border-color: rgba(82, 254, 254, 0.45);
            color: #52fefe;
        }
    }

    .folder-chip {
        font-size: 0.6rem;
    }

    .icon-only {
        padding: 3px 6px;
    }

    .inline-form {
        gap: 4px;
    }

    .folder-tools {
        gap: 8px;
        font-size: 0.65rem;
        color: rgba(255, 255, 255, 0.6);
    }

    .folder-name {
        flex: 1;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }

    .link {
        background: transparent;
        border: none;
        color: rgba(255, 255, 255, 0.55);
        cursor: pointer;
        display: flex;
        padding: 2px;

        &:hover {
            color: #52fefe;
        }

        &.danger:hover {
            color: #ff6b6b;
        }
    }
</style>

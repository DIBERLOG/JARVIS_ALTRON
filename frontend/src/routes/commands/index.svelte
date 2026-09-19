<script lang="ts">
    /**
     * The commands that really reach the listener.
     *
     * The page is built from the voice host's own answer: the packs it loaded with
     * its own parser, the phrases they carry, the slots they need, the risk level the
     * safety gate will read, and — separately — whether a phrase reaches a command
     * (MATCHED) and whether its executor exists in this build (EXECUTABLE). A pack
     * the loader did not read is listed with a reason instead of being quietly
     * absent, because that is exactly the question this page exists to answer.
     *
     * Nothing here executes anything. The only thing the page can send is a phrase to
     * check, which the voice host compares and forgets.
     */
    import { onMount } from "svelte"
    import { Button, Group, Input, Space, Text } from "@svelteuidev/core"

    import HDivider from "@/components/elements/HDivider.svelte"
    import Footer from "@/components/Footer.svelte"
    import CommandStages from "@/components/settings/CommandStages.svelte"
    import PhraseCheck from "@/components/settings/PhraseCheck.svelte"
    import {
        RISKS,
        STATUSES,
        categoryKey,
        commandCatalogApi,
        filterEntries,
        packReasonKey,
        riskKey,
        statusKey,
        titleOf,
        unavailableKey
    } from "@/lib/command-catalog"
    import type { CommandCatalog, CatalogEntry } from "@/lib/command-catalog"
    import { translations, translate } from "@/stores"

    $: t = (key: string) => translate($translations, key)

    type Tab = "commands" | "phrase" | "diagnostics"

    let catalog: CommandCatalog | null = null
    let tab: Tab = "commands"
    let query = ""
    let category = ""
    let status = ""
    let risk = ""
    let busy = false
    let error = ""
    let expanded = ""
    /** The language the catalogue was read in, so a change reloads it. */
    let loadedFor = ""
    let unreadableOpen = false

    $: entries = catalog ? filterEntries(catalog.entries, category, query, status, risk) : []
    $: categories = catalog ? catalog.categories : []
    $: ready = entries.filter((entry) => entry.status === "ready").length

    onMount(() => {
        void load()
    })

    // The catalogue is read in one language: when the language changes, the phrases
    // and the descriptions have to be read again.
    $: if ($translations && loadedFor && loadedFor !== $translations.language) {
        void load()
    }

    async function load() {
        busy = true
        try {
            catalog = await commandCatalogApi.load()
            loadedFor = $translations ? $translations.language : catalog.language
            error = ""
        } catch (failure) {
            error = typeof failure === "string" ? failure : String(failure)
        } finally {
            busy = false
        }
    }

    function toggle(id: string) {
        expanded = expanded === id ? "" : id
    }

    /** The four questions, each with its own indicator. */
    function indicators(entry: CatalogEntry): { key: string; ok: boolean }[] {
        return [
            { key: "command-indicator-recognized", ok: entry.recognized },
            { key: "command-indicator-executable", ok: entry.executor_ready },
            { key: "command-indicator-allowed", ok: entry.allowed },
            { key: "command-indicator-verified", ok: entry.verified }
        ]
    }
</script>

<Space h="xl" />
<Text weight={700} size="xl">{t("commands-title")}</Text>
<Space h="xs" />
<Text size="sm" color="dimmed">{t("commands-subtitle")}</Text>
<Space h="md" />

{#if error}
    <Text size="sm" color="red">{error}</Text>
    <Space h="sm" />
{/if}

<Group spacing="xs">
    <Button size="sm" variant={tab === "commands" ? "filled" : "default"} on:click={() => (tab = "commands")}>
        {t("commands-tab-commands")}
    </Button>
    <Button size="sm" variant={tab === "phrase" ? "filled" : "default"} on:click={() => (tab = "phrase")}>
        {t("commands-tab-phrase")}
    </Button>
    <Button
        size="sm"
        variant={tab === "diagnostics" ? "filled" : "default"}
        on:click={() => (tab = "diagnostics")}
    >
        {t("commands-tab-diagnostics")}
    </Button>
</Group>
<Space h="md" />

{#if tab === "commands"}
    <Group spacing="xs" align="end">
        <Input size="md" placeholder={t("commands-search")} bind:value={query} />
        <Button size="sm" variant="default" loading={busy} on:click={load}>
            {t("commands-refresh")}
        </Button>
    </Group>
    <Space h="sm" />

    <Text size="sm" weight={600}>{t("commands-filter-category")}</Text>
    <Group spacing="xs">
        <Button size="xs" variant={category === "" ? "filled" : "default"} on:click={() => (category = "")}>
            {t("commands-all-categories")}
        </Button>
        {#each categories as key (key)}
            <Button size="xs" variant={category === key ? "filled" : "default"} on:click={() => (category = key)}>
                {t(categoryKey(key))}
            </Button>
        {/each}
    </Group>
    <Space h="xs" />

    <Group spacing="lg" align="start">
        <div>
            <Text size="sm" weight={600}>{t("commands-filter-status")}</Text>
            <Group spacing="xs">
                <Button size="xs" variant={status === "" ? "filled" : "default"} on:click={() => (status = "")}>
                    {t("commands-filter-any")}
                </Button>
                {#each STATUSES as key (key)}
                    <Button size="xs" variant={status === key ? "filled" : "default"} on:click={() => (status = key)}>
                        {t(statusKey(key))}
                    </Button>
                {/each}
            </Group>
        </div>
        <div>
            <Text size="sm" weight={600}>{t("commands-filter-risk")}</Text>
            <Group spacing="xs">
                <Button size="xs" variant={risk === "" ? "filled" : "default"} on:click={() => (risk = "")}>
                    {t("commands-filter-any")}
                </Button>
                {#each RISKS as key (key)}
                    <Button size="xs" variant={risk === key ? "filled" : "default"} on:click={() => (risk = key)}>
                        {t(riskKey(key))}
                    </Button>
                {/each}
            </Group>
        </div>
    </Group>
    <Space h="sm" />

    {#if catalog}
        <Text size="sm" color="dimmed">
            {entries.length} {t("commands-count-label")} · {ready} {t("commands-ready-label")}
        </Text>
        <Space h="sm" />

        {#if entries.length === 0}
            <Text size="md" color="dimmed">
                {catalog.entries.length === 0 ? t("commands-empty") : t("commands-no-match")}
            </Text>
        {:else}
            <div class="cards">
                {#each entries as entry (entry.pack + "/" + entry.id)}
                    <div class="card" class:open={expanded === entry.pack + "/" + entry.id}>
                        <Group spacing="xs">
                            <span class="category-dot" title={t(categoryKey(entry.category))}></span>
                            <Text weight={600} size="md">{titleOf(entry)}</Text>
                        </Group>
                        <Text size="sm" color="dimmed">{entry.id} · {entry.pack}</Text>

                        <Group spacing="xs">
                            <span class="badge" class:bad={entry.status !== "ready"}>
                                {t(statusKey(entry.status))}
                            </span>
                            <span class="badge subtle">{t(riskKey(entry.risk_level))}</span>
                            {#if entry.requires_confirmation}
                                <span class="badge subtle">{t("commands-confirmation-yes")}</span>
                            {/if}
                            <span class="badge subtle">
                                {entry.phrases.length} {t("commands-phrases-label")}
                            </span>
                        </Group>

                        <Space h="xs" />
                        <Group spacing="md">
                            {#each indicators(entry) as indicator (indicator.key)}
                                <Text size="xs" color={indicator.ok ? "green" : "red"}>
                                    {indicator.ok ? "●" : "○"} {t(indicator.key)}
                                </Text>
                            {/each}
                        </Group>

                        {#if !entry.enabled && entry.unavailable_reason}
                            <Text size="sm" color="orange">{t(unavailableKey(entry.unavailable_reason))}</Text>
                        {/if}

                        <Space h="xs" />
                        <Button size="xs" variant="subtle" on:click={() => toggle(entry.pack + "/" + entry.id)}>
                            {expanded === entry.pack + "/" + entry.id
                                ? t("commands-collapse")
                                : t("commands-expand")}
                        </Button>

                        {#if expanded === entry.pack + "/" + entry.id}
                            {#if entry.description}
                                <Text size="sm">{entry.description}</Text>
                            {/if}
                            {#if entry.phrases.length > 0}
                                <Text size="sm" weight={600}>{t("commands-phrases")}</Text>
                                <ul class="phrases">
                                    {#each entry.phrases as phrase (phrase)}
                                        <li><Text size="sm">{phrase}</Text></li>
                                    {/each}
                                </ul>
                            {/if}
                            {#if entry.slots.length > 0}
                                <Text size="sm" color="dimmed">
                                    {t("commands-slots")}:
                                    {entry.slots.map((slot) => slot.name).join(", ")}
                                </Text>
                            {/if}
                            <Text size="sm" color="dimmed">
                                {t("commands-source-label")}:
                                {entry.source === "settings" ? t("commands-source-settings") : t("commands-pack")}
                            </Text>
                        {/if}
                    </div>
                {/each}
            </div>
        {/if}

        {#if catalog.unreadable.length > 0}
            <Space h="md" />
            <Button size="xs" variant="subtle" on:click={() => (unreadableOpen = !unreadableOpen)}>
                {unreadableOpen ? t("commands-collapse") : t("commands-expand")}
                · {t("commands-unreadable-title")} ({catalog.unreadable.length})
            </Button>
            <Text size="sm" color="dimmed">{t("commands-unreadable-hint")}</Text>
            {#if unreadableOpen}
                <ul class="phrases">
                    {#each catalog.unreadable as pack (pack.pack)}
                        <li>
                            <Text size="sm">{pack.pack} — {t(packReasonKey(pack.reason))}</Text>
                        </li>
                    {/each}
                </ul>
            {/if}
        {/if}
    {/if}
{:else if tab === "phrase"}
    <PhraseCheck />
{:else}
    <CommandStages />
    <Space h="sm" />
    <Text size="sm" color="dimmed">{t("commands-diagnostics-hint")}</Text>
{/if}

<HDivider />
<Footer />

<style>
    /* Three to four cards on a wide window, two in the middle, one when narrow. */
    .cards {
        display: grid;
        grid-template-columns: repeat(auto-fill, minmax(17rem, 1fr));
        gap: 0.85rem;
        align-items: start;
    }

    @media (max-width: 1100px) {
        .cards {
            grid-template-columns: repeat(2, minmax(0, 1fr));
        }
    }

    @media (max-width: 700px) {
        .cards {
            grid-template-columns: minmax(0, 1fr);
        }
    }

    .card {
        border: 1px solid rgba(255, 255, 255, 0.14);
        border-radius: 12px;
        padding: 0.85rem;
        display: flex;
        flex-direction: column;
        gap: 0.3rem;
        font-size: 0.95rem;
        min-width: 0;
        overflow-wrap: anywhere;
    }

    .card.open {
        border-color: rgba(45, 212, 191, 0.55);
    }

    .category-dot {
        width: 0.6rem;
        height: 0.6rem;
        border-radius: 999px;
        background: #2dd4bf;
        flex: 0 0 auto;
    }

    .badge {
        border-radius: 999px;
        padding: 0.05rem 0.5rem;
        font-size: 0.75rem;
        border: 1px solid rgba(45, 212, 191, 0.5);
        color: #2dd4bf;
        white-space: nowrap;
    }

    .badge.bad {
        border-color: rgba(251, 146, 60, 0.6);
        color: #fb923c;
    }

    .badge.subtle {
        border-color: rgba(255, 255, 255, 0.18);
        color: inherit;
        opacity: 0.85;
    }

    .phrases {
        margin: 0.15rem 0 0 0;
        padding-left: 1.1rem;
    }
</style>

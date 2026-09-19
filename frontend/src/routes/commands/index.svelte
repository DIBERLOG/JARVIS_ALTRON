<script lang="ts">
    /**
     * The commands that really reach the listener.
     *
     * The page is built from the voice host's own answer: the packs it loaded with
     * its own parser, the phrases they carry, the slots they need, the risk level
     * the safety gate will read, whether a command needs a spoken confirmation, and
     * — when a command cannot run — why. A pack the loader did not read is listed
     * with a reason instead of being quietly absent, because that is exactly the
     * question this page exists to answer.
     *
     * Nothing here executes anything. The only thing the page can send is a phrase
     * to check, which the voice host compares and forgets.
     */
    import { onMount } from "svelte"
    import { Button, Group, Input, Space, Text } from "@svelteuidev/core"

    import HDivider from "@/components/elements/HDivider.svelte"
    import Footer from "@/components/Footer.svelte"
    import PhraseCheck from "@/components/settings/PhraseCheck.svelte"
    import {
        categoryKey,
        commandCatalogApi,
        filterEntries,
        packReasonKey,
        riskKey,
        unavailableKey
    } from "@/lib/command-catalog"
    import type { CommandCatalog, CatalogEntry } from "@/lib/command-catalog"
    import { translations, translate } from "@/stores"

    $: t = (key: string) => translate($translations, key)

    let catalog: CommandCatalog | null = null
    let query = ""
    let category = ""
    let busy = false
    let error = ""
    /** The language the catalogue was read in, so a change reloads it. */
    let loadedFor = ""

    $: entries = catalog ? filterEntries(catalog.entries, category, query) : []
    $: categories = catalog ? catalog.categories : []

    onMount(() => {
        void load()
    })

    // The catalogue is read in one language: when the language changes, the
    // phrases and the descriptions have to be read again.
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

    function choose(next: string) {
        category = next
    }

    /** Every phrase of a card, as the pack writes it. */
    function phrases(entry: CatalogEntry): string[] {
        return entry.phrases
    }
</script>

<Space h="xl" />
<Text weight={700} size="xl">{t("commands-title")}</Text>
<Space h="xs" />
<Text size="sm" color="dimmed">{t("commands-subtitle")}</Text>
<Space h="sm" />

{#if error}
    <Text size="sm" color="red">{error}</Text>
{/if}

<Group spacing="xs" align="end">
    <Input size="sm" placeholder={t("commands-search")} bind:value={query} />
    <Button size="sm" variant="default" loading={busy} on:click={load}>
        {t("commands-refresh")}
    </Button>
</Group>
<Space h="xs" />

<Group spacing="xs">
    <Button size="xs" variant={category === "" ? "filled" : "default"} on:click={() => choose("")}>
        {t("commands-all-categories")}
    </Button>
    {#each categories as key (key)}
        <Button
            size="xs"
            variant={category === key ? "filled" : "default"}
            on:click={() => choose(key)}
        >
            {t(categoryKey(key))}
        </Button>
    {/each}
</Group>
<Space h="xs" />

{#if catalog}
    <Text size="xs" color="dimmed"
        >{entries.length} {t("commands-count-label")}</Text
    >
    <Space h="sm" />

    {#if entries.length === 0}
        <Text size="sm" color="dimmed">
            {query ? t("commands-no-match") : t("commands-empty")}
        </Text>
    {:else}
        <div class="cards">
            {#each entries as entry (entry.pack + "/" + entry.id)}
                <div class="card">
                    <Group spacing="xs">
                        <Text weight={600}>{entry.id}</Text>
                        <Text size="xs" color="dimmed">{t(categoryKey(entry.category))}</Text>
                        {#if entry.source === "settings"}
                            <Text size="xs" color="blue">{t("commands-source-settings")}</Text>
                        {:else}
                            <Text size="xs" color="dimmed">{t("commands-pack")}: {entry.pack}</Text>
                        {/if}
                    </Group>

                    <Group spacing="xs">
                        <Text size="xs" color={entry.risk_level === "safe" ? "dimmed" : "orange"}>
                            {t("commands-risk")}: {t(riskKey(entry.risk_level))}
                        </Text>
                        <Text size="xs" color={entry.requires_confirmation ? "orange" : "dimmed"}>
                            {t("commands-confirmation")}:
                            {entry.requires_confirmation
                                ? t("commands-confirmation-yes")
                                : t("commands-confirmation-no")}
                        </Text>
                        <Text size="xs" color={entry.enabled ? "green" : "red"}>
                            {entry.enabled ? t("commands-enabled") : t("commands-disabled")}
                        </Text>
                    </Group>

                    {#if !entry.enabled && entry.unavailable_reason}
                        <Text size="xs" color="red">
                            {t(unavailableKey(entry.unavailable_reason))}
                        </Text>
                    {/if}

                    {#if entry.description}
                        <Text size="xs" color="dimmed">{entry.description}</Text>
                    {/if}

                    {#if phrases(entry).length > 0}
                        <Text size="xs" weight={600}>{t("commands-phrases")}</Text>
                        <ul class="phrases">
                            {#each phrases(entry) as phrase (phrase)}
                                <li><Text size="xs">{phrase}</Text></li>
                            {/each}
                        </ul>
                    {/if}

                    {#if entry.slots.length > 0}
                        <Text size="xs" color="dimmed">
                            {t("commands-slots")}:
                            {entry.slots.map((slot) => slot.name).join(", ")}
                        </Text>
                    {/if}
                </div>
            {/each}
        </div>
    {/if}

    {#if catalog.unreadable.length > 0}
        <Space h="md" />
        <Text weight={600}>{t("commands-unreadable-title")}</Text>
        <Text size="xs" color="dimmed">{t("commands-unreadable-hint")}</Text>
        <ul class="phrases">
            {#each catalog.unreadable as pack (pack.pack)}
                <li>
                    <Text size="xs">{pack.pack} — {t(packReasonKey(pack.reason))}</Text>
                </li>
            {/each}
        </ul>
    {/if}
{/if}

<Space h="md" />
<PhraseCheck />

<HDivider />
<Footer />

<style>
    .cards {
        display: grid;
        grid-template-columns: repeat(auto-fill, minmax(19rem, 1fr));
        gap: 0.6rem;
    }

    .card {
        border: 1px solid rgba(255, 255, 255, 0.12);
        border-radius: 10px;
        padding: 0.6rem;
        display: flex;
        flex-direction: column;
        gap: 0.15rem;
    }

    .phrases {
        margin: 0.1rem 0 0 0;
        padding-left: 1rem;
    }
</style>

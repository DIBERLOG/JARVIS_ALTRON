<script lang="ts">
    /**
     * Settings for the local spelling and text-improvement features.
     *
     * Everything on this page is a switch or a number, so it is never secret and never
     * encrypted: it stays readable while the storage is locked, which is exactly when a
     * user decides whether checking should run at all.
     *
     * The two switches that change text are off by default and are labelled as such:
     * `safe_autocorrect` only ever applies an unambiguous repair, and `ai_improvement`
     * only ever *proposes* a change that the user then confirms in a preview.
     */
    import { onMount } from "svelte"

    import { Alert, Button, Space, Switch, Text } from "@svelteuidev/core"

    import { translate, translations } from "@/stores"
    import { autocorrectApi, wordQuery } from "@/lib/autocorrect"
    import type {
        AutocorrectSettings,
        AutocorrectStatusView,
        Language,
        UserDictionaryEntry
    } from "@/lib/autocorrect-model"
    import {
        LANGUAGE_MODES,
        LANGUAGES,
        defaultSettings,
        dictionaryStateKey,
        languageLabelKey,
        languageModeLabelKey,
        normalizeSettings,
        wordIsAcceptable
    } from "@/lib/autocorrect-model"

    $: t = (key: string) => translate($translations, key)

    let status: AutocorrectStatusView | null = null
    let settings: AutocorrectSettings = defaultSettings()
    let words: UserDictionaryEntry[] = []
    let newWord = ""
    let newWordLanguage: Language = "russian"
    let search = ""
    let rulePattern = ""
    let ruleReplacement = ""
    let ruleAuto = false
    let busy = false
    let saved = false
    let actionError = ""
    let exportConfirm = false

    $: autocorrect = status?.autocorrect ?? null
    $: dictionaries = autocorrect?.dictionaries ?? []
    $: stats = autocorrect?.user ?? null
    $: locked = autocorrect !== null && !autocorrect.unlocked

    onMount(async () => {
        await load()
    })

    async function load() {
        busy = true
        try {
            status = await autocorrectApi.status()
            settings = normalizeSettings(await autocorrectApi.getSettings())
            if (status.autocorrect.unlocked) {
                words = await autocorrectApi.listWords(wordQuery("", null))
            }
            actionError = ""
        } catch (error) {
            actionError = describeError(error)
        }
        busy = false
    }

    function describeError(error: unknown): string {
        if (typeof error === "string") return error
        if (error instanceof Error) return error.message
        return String(error)
    }

    async function save() {
        busy = true
        try {
            settings = normalizeSettings(settings)
            status = await autocorrectApi.updateSettings(settings)
            settings = normalizeSettings(await autocorrectApi.getSettings())
            saved = true
            actionError = ""
            setTimeout(() => {
                saved = false
            }, 4000)
        } catch (error) {
            actionError = describeError(error)
        }
        busy = false
    }

    async function reloadDictionaries() {
        try {
            status = await autocorrectApi.reloadDictionaries()
        } catch (error) {
            actionError = describeError(error)
        }
    }

    async function searchWords() {
        if (locked) return
        try {
            words = await autocorrectApi.listWords(wordQuery(search, null))
        } catch (error) {
            actionError = describeError(error)
        }
    }

    async function addWord() {
        if (!wordIsAcceptable(newWord)) return
        busy = true
        try {
            await autocorrectApi.addWord(newWord.trim(), newWordLanguage)
            newWord = ""
            await searchWords()
            status = await autocorrectApi.status()
            actionError = ""
        } catch (error) {
            actionError = describeError(error)
        }
        busy = false
    }

    async function removeWord(id: string) {
        busy = true
        try {
            await autocorrectApi.removeWord(id)
            await searchWords()
            status = await autocorrectApi.status()
        } catch (error) {
            actionError = describeError(error)
        }
        busy = false
    }

    async function addRule() {
        if (rulePattern.trim().length === 0 || ruleReplacement.trim().length === 0) return
        busy = true
        try {
            settings = normalizeSettings(
                await autocorrectApi.addRule(rulePattern, ruleReplacement, ruleAuto)
            )
            rulePattern = ""
            ruleReplacement = ""
            ruleAuto = false
            actionError = ""
        } catch (error) {
            actionError = describeError(error)
        }
        busy = false
    }

    async function removeRule(index: number) {
        busy = true
        try {
            settings = normalizeSettings(await autocorrectApi.removeRule(index))
        } catch (error) {
            actionError = describeError(error)
        }
        busy = false
    }

    async function importWords() {
        busy = true
        try {
            await autocorrectApi.importWordsFile(newWordLanguage)
            await searchWords()
            status = await autocorrectApi.status()
        } catch (error) {
            actionError = describeError(error)
        }
        busy = false
    }

    async function exportWords() {
        if (!exportConfirm) {
            exportConfirm = true
            return
        }
        exportConfirm = false
        busy = true
        try {
            await autocorrectApi.exportWordsFile(true)
        } catch (error) {
            actionError = describeError(error)
        }
        busy = false
    }

    async function exportBackup() {
        busy = true
        try {
            await autocorrectApi.exportWordsBackup()
        } catch (error) {
            actionError = describeError(error)
        }
        busy = false
    }

    /**
     * The dictionary folder is a plain optional path.
     *
     * The cast lives here rather than in the markup: a TypeScript assertion inside a
     * template expression is not something the Svelte parser accepts.
     */
    function onDictionaryDirInput(event: Event) {
        const value = (event.currentTarget as HTMLInputElement).value
        settings.dictionary_dir = value.trim().length > 0 ? value : null
    }
</script>

<div class="autocorrect-settings">
    {#if actionError}
        <Alert title={t('autocorrect-error')} color="red" variant="outline">
            <Text size="xs" color="gray">{actionError}</Text>
        </Alert>
    {/if}

    {#if locked}
        <Alert title={t('autocorrect-locked-title')} color="#868E96" variant="outline">
            <Text size="xs" color="gray">{t('autocorrect-locked-body')}</Text>
        </Alert>
    {/if}

    <Text size="sm" weight={600}>{t('autocorrect-settings-title')}</Text>
    <Text size="xs" color="gray">{t('autocorrect-settings-hint')}</Text>

    <div class="ac-switches">
        <Switch label={t('autocorrect-enabled')} bind:checked={settings.enabled} />
        <Switch label={t('autocorrect-check-notes')} bind:checked={settings.check_notes} />
        <Switch label={t('autocorrect-check-chat')} bind:checked={settings.check_chat} />
        <Switch
            label={t('autocorrect-safe-autocorrect')}
            bind:checked={settings.safe_autocorrect}
        />
        <Switch label={t('autocorrect-ai-improvement')} bind:checked={settings.ai_improvement} />
    </div>
    <Text size="xs" color="gray">{t('autocorrect-safe-autocorrect-hint')}</Text>
    <Text size="xs" color="gray">{t('autocorrect-ai-improvement-hint')}</Text>
    <Text size="xs" color="gray">{t('autocorrect-require-preview-hint')}</Text>

    <label class="ac-field">
        <span>{t('autocorrect-language-mode')}</span>
        <select class="ac-select" bind:value={settings.language}>
            {#each LANGUAGE_MODES as mode}
                <option value={mode}>{t(languageModeLabelKey(mode))}</option>
            {/each}
        </select>
        <small>{t('autocorrect-language-mode-hint')}</small>
    </label>

    <div class="ac-grid">
        <label class="ac-field">
            <span>{t('autocorrect-max-suggestions')}</span>
            <input class="ac-input" type="number" min="1" max="8" bind:value={settings.max_suggestions} />
        </label>
        <label class="ac-field">
            <span>{t('autocorrect-max-issues')}</span>
            <input class="ac-input" type="number" min="10" max="500" bind:value={settings.max_issues} />
        </label>
        <label class="ac-field">
            <span>{t('autocorrect-debounce')}</span>
            <input class="ac-input" type="number" min="0" max="5000" bind:value={settings.debounce_ms} />
            <small>{t('autocorrect-debounce-hint')}</small>
        </label>
        <label class="ac-field">
            <span>{t('autocorrect-timeout')}</span>
            <input class="ac-input" type="number" min="100" max="30000" bind:value={settings.timeout_ms} />
            <small>{t('autocorrect-timeout-hint')}</small>
        </label>
    </div>

    <label class="ac-field">
        <span>{t('autocorrect-dictionary-dir')}</span>
        <input
            class="ac-input"
            type="text"
            placeholder={autocorrect?.dictionary_dir ?? ""}
            value={settings.dictionary_dir ?? ""}
            on:input={onDictionaryDirInput}
        />
        <small>{t('autocorrect-dictionary-dir-hint')}</small>
    </label>

    <div class="ac-row">
        <Button size="xs" color="lime" uppercase on:click={save} disabled={busy}>
            {t('autocorrect-save')}
        </Button>
        <Button size="xs" color="gray" uppercase on:click={reloadDictionaries} disabled={busy}>
            {t('autocorrect-reload')}
        </Button>
        {#if saved}
            <Text size="xs" color="green">{t('autocorrect-saved')}</Text>
        {/if}
    </div>

    <Space h="sm" />
    <Text size="sm" weight={600}>{t('autocorrect-dictionaries-title')}</Text>
    {#each LANGUAGES as language}
        {@const entry = dictionaries.find((state) => state.language === language) ?? null}
        <div class="ac-dict">
            <span class="ac-dict-name">{t(languageLabelKey(language))}</span>
            <span class="ac-dict-state">
                {entry ? t(dictionaryStateKey(entry)) : t('autocorrect-dictionary-missing')}
                {#if entry?.words}
                    · {entry.words}
                {/if}
            </span>
            {#if entry?.state === "invalid" && entry.reason}
                <span class="ac-dict-reason">{entry.reason}</span>
            {/if}
            {#if entry?.state === "missing" && entry.expected}
                <span class="ac-dict-reason">{entry.expected.join(" · ")}</span>
            {/if}
        </div>
    {/each}
    <Text size="xs" color="gray">{t('autocorrect-dictionaries-hint')}</Text>
    {#if autocorrect}
        <Text size="xs" color="gray">{autocorrect.dictionary_dir}</Text>
    {/if}

    <Space h="sm" />
    <Text size="sm" weight={600}>{t('autocorrect-words-title')}</Text>
    <Text size="xs" color="gray">
        {t('autocorrect-words-hint')}
        {#if stats}
            · {stats.words} {t('autocorrect-words-count')}
            {#if stats.ignored > 0}
                · {stats.ignored} {t('autocorrect-ignored-count')}
            {/if}
            {#if stats.unreadable > 0}
                · {stats.unreadable} {t('autocorrect-unreadable-count')}
            {/if}
        {/if}
    </Text>

    <div class="ac-row">
        <input class="ac-input" type="text" placeholder={t('autocorrect-word-placeholder')} bind:value={newWord} />
        <select class="ac-select short" bind:value={newWordLanguage}>
            {#each LANGUAGES as language}
                <option value={language}>{t(languageLabelKey(language))}</option>
            {/each}
        </select>
        <Button size="xs" color="cyan" uppercase on:click={addWord} disabled={busy || !wordIsAcceptable(newWord) || locked}>
            {t('autocorrect-word-add')}
        </Button>
    </div>

    <div class="ac-row">
        <input
            class="ac-input"
            type="text"
            placeholder={t('autocorrect-word-search')}
            bind:value={search}
            on:input={searchWords}
        />
        <Button size="xs" color="gray" uppercase on:click={importWords} disabled={busy || locked}>
            {t('autocorrect-words-import')}
        </Button>
        <Button size="xs" color="gray" uppercase on:click={exportWords} disabled={busy || locked}>
            {exportConfirm ? t('autocorrect-words-export-confirm') : t('autocorrect-words-export')}
        </Button>
        <Button size="xs" color="gray" uppercase on:click={exportBackup} disabled={busy || locked}>
            {t('autocorrect-words-export-backup')}
        </Button>
    </div>
    <Text size="xs" color="gray">{t('autocorrect-words-export-hint')}</Text>

    {#if words.length > 0}
        <div class="ac-word-list">
            {#each words as word (word.id)}
                <span class="ac-word">
                    {word.word}
                    <span class="ac-word-meta">{t(languageLabelKey(word.language))}</span>
                    <button class="ac-word-remove" on:click={() => removeWord(word.id)} disabled={busy}>
                        ×
                    </button>
                </span>
            {/each}
        </div>
    {/if}

    <Space h="sm" />
    <Text size="sm" weight={600}>{t('autocorrect-rules-title')}</Text>
    <Text size="xs" color="gray">{t('autocorrect-rules-hint')}</Text>
    <div class="ac-row">
        <input class="ac-input" type="text" placeholder={t('autocorrect-rule-pattern')} bind:value={rulePattern} />
        <input class="ac-input" type="text" placeholder={t('autocorrect-rule-replacement')} bind:value={ruleReplacement} />
        <Switch label={t('autocorrect-rule-auto')} bind:checked={ruleAuto} />
        <Button size="xs" color="cyan" uppercase on:click={addRule} disabled={busy}>
            {t('autocorrect-rule-add')}
        </Button>
    </div>
    {#each settings.custom_rules as rule, index (index)}
        <div class="ac-rule">
            <span class="ac-rule-pair">{rule.pattern} → {rule.replacement}</span>
            <span class="ac-rule-meta">{rule.auto_apply ? t('autocorrect-rule-auto') : t('autocorrect-rule-manual')}</span>
            <button class="ac-word-remove" on:click={() => removeRule(index)} disabled={busy}>×</button>
        </div>
    {/each}
</div>

<style lang="scss">
    .autocorrect-settings {
        display: flex;
        flex-direction: column;
        gap: 6px;
    }

    .ac-switches {
        display: flex;
        flex-direction: column;
        gap: 4px;
        margin-top: 4px;
    }

    .ac-field {
        display: flex;
        flex-direction: column;
        gap: 3px;
        font-size: 0.7rem;
        color: rgba(255, 255, 255, 0.8);

        small {
            color: rgba(255, 255, 255, 0.45);
            font-size: 0.62rem;
        }
    }

    .ac-grid {
        display: grid;
        grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
        gap: 8px;
    }

    .ac-input,
    .ac-select {
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

    .ac-select.short {
        max-width: 130px;
    }

    .ac-row {
        display: flex;
        align-items: center;
        gap: 6px;
        flex-wrap: wrap;
    }

    .ac-dict {
        display: flex;
        align-items: baseline;
        gap: 8px;
        flex-wrap: wrap;
        padding: 3px 0;
    }

    .ac-dict-name {
        font-size: 0.7rem;
        color: #ffffff;
        min-width: 70px;
    }

    .ac-dict-state {
        font-size: 0.62rem;
        letter-spacing: 0.04em;
        text-transform: uppercase;
        color: rgba(138, 200, 50, 0.9);
    }

    .ac-dict-reason {
        font-family: monospace;
        font-size: 0.6rem;
        color: rgba(255, 255, 255, 0.5);
        word-break: break-all;
    }

    .ac-word-list {
        display: flex;
        flex-wrap: wrap;
        gap: 4px;
        max-height: 160px;
        overflow: auto;
    }

    .ac-word {
        display: inline-flex;
        align-items: center;
        gap: 4px;
        background: rgba(82, 254, 254, 0.08);
        border: 1px solid rgba(82, 254, 254, 0.2);
        border-radius: 5px;
        font-size: 0.66rem;
        padding: 2px 5px;
    }

    .ac-word-meta {
        font-size: 0.56rem;
        color: rgba(255, 255, 255, 0.5);
    }

    .ac-word-remove {
        background: transparent;
        border: none;
        color: rgba(255, 107, 107, 0.8);
        cursor: pointer;
        font-size: 0.7rem;
        line-height: 1;
    }

    .ac-rule {
        display: flex;
        align-items: center;
        gap: 8px;
        font-size: 0.66rem;
    }

    .ac-rule-pair {
        font-family: monospace;
        color: rgba(255, 255, 255, 0.85);
    }

    .ac-rule-meta {
        font-size: 0.58rem;
        text-transform: uppercase;
        color: rgba(255, 255, 255, 0.45);
    }
</style>

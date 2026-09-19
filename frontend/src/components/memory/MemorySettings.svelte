<script lang="ts">
    /**
     * Every switch and number of `MemorySettings`.
     *
     * The settings are not secret, so this panel is reachable while the storage is
     * locked: that is exactly when a user decides whether memory should be on at all.
     * The form is validated locally with `validateSettings` before a save, and the
     * values are repaired with `normalizeSettings` so a half-typed number can never
     * become a value the core would refuse.
     */
    import { createEventDispatcher } from "svelte"
    import { Button, Switch, Text } from "@svelteuidev/core"

    import type { MemorySettings, SettingsIssue } from "@/lib/memory-model"
    import {
        MAX_MAX_RECENT_MESSAGES,
        MAX_MEMORY_TOKEN_BUDGET,
        MAX_SUMMARY_KEEP_RECENT,
        MAX_SUMMARY_TRIGGER_MESSAGES,
        MIN_MAX_RECENT_MESSAGES,
        MIN_MEMORY_TOKEN_BUDGET,
        MIN_SUMMARY_KEEP_RECENT,
        MIN_SUMMARY_TRIGGER_MESSAGES,
        normalizeSettings,
        validateSettings
    } from "@/lib/memory-model"
    import { translations, translate } from "@/stores"

    export let settings: MemorySettings
    export let busy = false
    export let notice = ""

    const dispatch = createEventDispatcher<{ save: MemorySettings; changed: void }>()

    $: t = (key: string) => translate($translations, key)

    /** The repaired form values; every edit passes through `normalizeSettings`. */
    let draft: MemorySettings = normalizeSettings(settings)

    // A settings document that arrives from the backend, or after a save, replaces the
    // form. A value the user is still typing is repaired instead of dropped.
    $: draft = normalizeSettings(settings)

    $: issues = validateSettings(draft)
    /** The `field` of every issue, so a control can mark itself. */
    $: issueFields = new Set(issues.map((issue: SettingsIssue) => issue.field))

    function patch(changes: Partial<MemorySettings>) {
        draft = normalizeSettings({ ...draft, ...changes })
        dispatch("changed")
    }

    /** A number input still being typed is repaired rather than trusted. */
    function onNumber(field: keyof MemorySettings, event: Event) {
        const raw = Number((event.currentTarget as HTMLInputElement).value)
        patch({ [field]: raw } as Partial<MemorySettings>)
    }

    function save() {
        if (issues.length > 0) return
        dispatch("save", draft)
    }
</script>

<div class="settings">
    <div class="settings-head">
        <span class="panel-title">{t('memory-settings-title')}</span>
        <Button size="xs" color="lime" uppercase disabled={busy || issues.length > 0} on:click={save}>
            {t('memory-settings-save')}
        </Button>
    </div>

    <div class="switches">
        <div class="switch-row" class:flagged={issueFields.has("enabled")}>
            <Switch
                label={t('memory-settings-enabled')}
                checked={draft.enabled}
                on:change={() => patch({ enabled: !draft.enabled })}
            />
            <Text size="xs" color="gray">{t('memory-settings-enabled-desc')}</Text>
            {#if issueFields.has("enabled")}
                <span class="issue">{t('memory-issue-suggestions-need-memory')}</span>
            {/if}
        </div>

        <div class="switch-row">
            <Switch
                label={t('memory-settings-history')}
                checked={draft.save_history}
                on:change={() => patch({ save_history: !draft.save_history })}
            />
            <Text size="xs" color="gray">{t('memory-settings-history-desc')}</Text>
        </div>

        <div class="switch-row">
            <Switch
                label={t('memory-settings-long-term')}
                checked={draft.use_long_term_memory}
                on:change={() => patch({ use_long_term_memory: !draft.use_long_term_memory })}
            />
            <Text size="xs" color="gray">{t('memory-settings-long-term-desc')}</Text>
        </div>

        <!-- The opt-in switch: it is off by default and is marked as such. -->
        <div class="switch-row opt-in" class:flagged={issueFields.has("suggest_facts")}>
            <Switch
                label={t('memory-settings-suggest')}
                checked={draft.suggest_facts}
                on:change={() => patch({ suggest_facts: !draft.suggest_facts })}
            />
            <Text size="xs" color="gray">{t('memory-settings-suggest-desc')}</Text>
            {#if issueFields.has("suggest_facts")}
                <span class="issue">{t('memory-issue-suggestions-need-memory')}</span>
            {/if}
        </div>

        <div class="switch-row" class:flagged={issueFields.has("summary_trigger_messages") && draft.auto_summaries}>
            <Switch
                label={t('memory-settings-summaries')}
                checked={draft.auto_summaries}
                on:change={() => patch({ auto_summaries: !draft.auto_summaries })}
            />
            <Text size="xs" color="gray">{t('memory-settings-summaries-desc')}</Text>
            {#if issueFields.has("summary_trigger_messages") && draft.auto_summaries}
                <span class="issue">{t('memory-issue-summary-order')}</span>
            {/if}
        </div>
    </div>

    <div class="numbers">
        <label class="number" class:flagged={issueFields.has("max_recent_messages")}>
            <Text size="xs" color="gray">{t('memory-settings-recent-messages')}</Text>
            <input
                type="number"
                min={String(MIN_MAX_RECENT_MESSAGES)}
                max={String(MAX_MAX_RECENT_MESSAGES)}
                value={String(draft.max_recent_messages)}
                on:input={(event) => onNumber("max_recent_messages", event)}
            />
            {#if issueFields.has("max_recent_messages")}
                <span class="issue">{t('memory-issue-recent-messages')}</span>
            {/if}
        </label>

        <label class="number" class:flagged={issueFields.has("memory_token_budget")}>
            <Text size="xs" color="gray">{t('memory-settings-token-budget')}</Text>
            <input
                type="number"
                min={String(MIN_MEMORY_TOKEN_BUDGET)}
                max={String(MAX_MEMORY_TOKEN_BUDGET)}
                value={String(draft.memory_token_budget)}
                on:input={(event) => onNumber("memory_token_budget", event)}
            />
            {#if issueFields.has("memory_token_budget")}
                <span class="issue">{t('memory-issue-token-budget')}</span>
            {/if}
        </label>

        <label class="number" class:flagged={issueFields.has("summary_trigger_messages")}>
            <Text size="xs" color="gray">{t('memory-settings-summary-trigger')}</Text>
            <input
                type="number"
                min={String(MIN_SUMMARY_TRIGGER_MESSAGES)}
                max={String(MAX_SUMMARY_TRIGGER_MESSAGES)}
                value={String(draft.summary_trigger_messages)}
                on:input={(event) => onNumber("summary_trigger_messages", event)}
            />
            {#if issueFields.has("summary_trigger_messages")}
                <span class="issue">{t('memory-issue-summary-trigger')}</span>
            {/if}
        </label>

        <label class="number" class:flagged={issueFields.has("summary_keep_recent")}>
            <Text size="xs" color="gray">{t('memory-settings-summary-keep')}</Text>
            <input
                type="number"
                min={String(MIN_SUMMARY_KEEP_RECENT)}
                max={String(MAX_SUMMARY_KEEP_RECENT)}
                value={String(draft.summary_keep_recent)}
                on:input={(event) => onNumber("summary_keep_recent", event)}
            />
            {#if issueFields.has("summary_keep_recent")}
                <span class="issue">{t('memory-issue-summary-keep')}</span>
            {/if}
        </label>
    </div>

    {#if !draft.enabled}
        <div class="off-note">
            <Text size="xs" color="gray">{t('memory-settings-off-note')}</Text>
        </div>
    {/if}

    {#if notice.length > 0}
        <Text size="xs" color="green">{notice}</Text>
    {/if}
</div>

<style lang="scss">
    .settings {
        background: rgba(20, 30, 35, 0.55);
        border: 1px solid rgba(255, 255, 255, 0.06);
        border-radius: 10px;
        padding: 10px;
        display: flex;
        flex-direction: column;
        gap: 8px;
    }

    .settings-head {
        display: flex;
        align-items: center;
        gap: 8px;
    }

    .panel-title {
        color: #ffffff;
        flex: 1;
        font-size: 0.7rem;
        letter-spacing: 0.08em;
        text-transform: uppercase;
    }

    .switches {
        display: grid;
        gap: 6px;
    }

    .switch-row {
        background: rgba(15, 22, 26, 0.55);
        border: 1px solid transparent;
        border-radius: 8px;
        padding: 6px 8px;

        &.flagged {
            border-color: rgba(255, 107, 107, 0.45);
        }

        &.opt-in {
            border-color: rgba(82, 254, 254, 0.25);
        }
    }

    .numbers {
        display: grid;
        grid-template-columns: repeat(auto-fit, minmax(190px, 1fr));
        gap: 6px;
    }

    .number {
        display: flex;
        flex-direction: column;
        gap: 3px;
        background: rgba(15, 22, 26, 0.55);
        border: 1px solid transparent;
        border-radius: 8px;
        padding: 6px 8px;

        &.flagged {
            border-color: rgba(255, 107, 107, 0.45);
        }

        input {
            width: 100%;
            background: rgba(10, 18, 22, 0.75);
            border: 1px solid rgba(255, 255, 255, 0.1);
            border-radius: 6px;
            color: #ffffff;
            font-size: 0.7rem;
            padding: 6px 7px;

            &:focus {
                outline: none;
                border-color: rgba(82, 254, 254, 0.5);
            }
        }
    }

    .issue {
        color: #ff9b9b;
        display: block;
        font-size: 0.62rem;
        margin-top: 2px;
    }

    .off-note {
        background: rgba(255, 179, 71, 0.1);
        border-radius: 6px;
        padding: 5px 8px;
    }
</style>

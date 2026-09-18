<script lang="ts">
    import { createEventDispatcher } from "svelte"
    import { Button, Text } from "@svelteuidev/core"
    import { EyeClosed, EyeOpen, LightningBolt, Clipboard, ClipboardCopy } from "radix-icons-svelte"

    import type { PasswordPolicy } from "@/lib/vault-model"
    import {
        GENERATOR_LENGTH_MAX,
        GENERATOR_LENGTH_MIN,
        clampGeneratorLength,
        defaultPolicy,
        estimateEntropyBits,
        selectedCategoryCount,
        validatePolicy
    } from "@/lib/vault-model"
    import { vaultApi } from "@/lib/vault"
    import { translations, translate } from "@/stores"

    export let busy = false
    /** Delay before the clipboard is cleared, configured on the page. */
    export let clipboardTimeout = 30

    const dispatch = createEventDispatcher<{
        /** The generated password goes into the editor field, unsaved. */
        use: string
        /** The password was generated and copied without being shown here. */
        copied: void
    }>()

    $: t = (key: string) => translate($translations, key)

    let policy: PasswordPolicy = defaultPolicy()
    let open = false
    let generated = ""
    let entropyBits = 0
    let errorKey = ""

    $: entropyPreview = estimateEntropyBits(policy)
    $: categories = selectedCategoryCount(policy)

    function toggle(value: boolean): boolean {
        return !value
    }

    function applyLength(event: Event) {
        policy = { ...policy, length: clampGeneratorLength(Number((event.currentTarget as HTMLInputElement).value)) }
    }

    async function generate(show: boolean) {
        const check = validatePolicy(policy)
        if (!check.ok) {
            errorKey = check.errorKey ?? "vault-generator-invalid"
            return
        }
        errorKey = ""
        busy = true
        try {
            const result = await vaultApi.generatePassword(policy)
            entropyBits = result.entropy_bits
            if (show) {
                generated = result.password
                dispatch("use", result.password)
            } else {
                // Copy without ever holding the value in this component.
                await vaultApi.generateAndCopy(policy, clipboardTimeout)
                generated = ""
                dispatch("copied")
            }
        } catch (error) {
            errorKey = typeof error === "string" ? error : String(error)
        } finally {
            busy = false
        }
    }
</script>

<div class="generator">
    <button class="generator-toggle" on:click={() => (open = !open)}>
        <LightningBolt size={13} />
        {t('vault-generator')}
        {#if open}
            <EyeClosed size={12} />
        {:else}
            <EyeOpen size={12} />
        {/if}
    </button>

    {#if open}
        <div class="generator-body">
            <label class="length">
                <Text size="xs" color="gray">{t('vault-generator-length')}: {policy.length}</Text>
                <input
                    type="range"
                    min={GENERATOR_LENGTH_MIN}
                    max={GENERATOR_LENGTH_MAX}
                    value={policy.length}
                    on:input={applyLength}
                />
            </label>

            <div class="toggles">
                <label><input type="checkbox" checked={policy.lowercase} on:change={() => (policy = { ...policy, lowercase: toggle(policy.lowercase) })} /> abc</label>
                <label><input type="checkbox" checked={policy.uppercase} on:change={() => (policy = { ...policy, uppercase: toggle(policy.uppercase) })} /> ABC</label>
                <label><input type="checkbox" checked={policy.digits} on:change={() => (policy = { ...policy, digits: toggle(policy.digits) })} /> 123</label>
                <label><input type="checkbox" checked={policy.symbols} on:change={() => (policy = { ...policy, symbols: toggle(policy.symbols) })} /> !@#</label>
                <label>
                    <input
                        type="checkbox"
                        checked={policy.exclude_similar}
                        on:change={() => (policy = { ...policy, exclude_similar: toggle(policy.exclude_similar) })}
                    />
                    {t('vault-generator-similar')}
                </label>
                <label>
                    <input
                        type="checkbox"
                        checked={policy.require_each_category}
                        on:change={() => (policy = { ...policy, require_each_category: toggle(policy.require_each_category) })}
                    />
                    {t('vault-generator-each')}
                </label>
            </div>

            <Text size="xs" color="gray">
                {t('vault-generator-categories')}: {categories} · {t('vault-generator-entropy')}:
                {entropyPreview.toFixed(0)}
            </Text>

            {#if errorKey}
                <Text size="xs" color="red">{t(errorKey)}</Text>
            {/if}

            {#if generated}
                <div class="generated">
                    <code>{generated}</code>
                    <Text size="xs" color="gray">
                        {t('vault-generator-not-saved')} · {entropyBits.toFixed(0)} bit
                    </Text>
                </div>
            {/if}

            <div class="generator-actions">
                <Button size="xs" color="cyan" uppercase on:click={() => generate(true)} disabled={busy}>
                    <LightningBolt size={12} />
                    {t('vault-generator-generate')}
                </Button>
                <Button size="xs" color="gray" uppercase on:click={() => generate(false)} disabled={busy}>
                    <Clipboard size={12} />
                    {t('vault-generator-copy')}
                </Button>
                {#if generated}
                    <Button
                        size="xs"
                        color="gray"
                        uppercase
                        on:click={() => {
                            generated = ""
                        }}
                    >
                        <ClipboardCopy size={12} />
                        {t('vault-hide')}
                    </Button>
                {/if}
            </div>
        </div>
    {/if}
</div>

<style lang="scss">
    .generator {
        border-top: 1px dashed rgba(255, 255, 255, 0.12);
        padding-top: 6px;
    }

    .generator-toggle {
        display: inline-flex;
        align-items: center;
        gap: 6px;
        background: transparent;
        border: none;
        color: rgba(255, 255, 255, 0.6);
        cursor: pointer;
        font-size: 0.62rem;
        letter-spacing: 0.04em;
        text-transform: uppercase;
        padding: 0;

        &:hover {
            color: #52fefe;
        }
    }

    .generator-body {
        display: flex;
        flex-direction: column;
        gap: 6px;
        margin-top: 6px;
        background: rgba(15, 22, 26, 0.6);
        border-radius: 6px;
        padding: 8px;
    }

    .length {
        display: flex;
        flex-direction: column;
        gap: 3px;
    }

    input[type="range"] {
        width: 100%;
    }

    .toggles {
        display: flex;
        flex-wrap: wrap;
        gap: 6px 10px;
        font-size: 0.62rem;
        color: rgba(255, 255, 255, 0.75);

        label {
            display: inline-flex;
            align-items: center;
            gap: 4px;
        }
    }

    .generated {
        display: flex;
        flex-direction: column;
        gap: 2px;

        code {
            background: rgba(10, 18, 22, 0.9);
            border-radius: 4px;
            color: #52fefe;
            font-size: 0.72rem;
            padding: 5px 7px;
            word-break: break-all;
        }
    }

    .generator-actions {
        display: flex;
        flex-wrap: wrap;
        gap: 5px;
    }
</style>

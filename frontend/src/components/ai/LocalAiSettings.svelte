<script lang="ts">
    /**
     * Model and resource settings for the local AI runtime.
     *
     * The form saves paths and numbers; the authority on whether the machine can
     * run the configuration is the core report, which is shown here and refused
     * again when the server is started. Nothing on this page is secret, and no
     * value leaves the machine.
     */
    import { onMount } from "svelte"

    import { translate, translations } from "@/stores"
    import { localAiApi } from "@/lib/local-ai"
    import {
        defaultSettings,
        describeModel,
        fieldLabelKey,
        formatBytes,
        normalizeSettings,
        reportLevelKey,
        validateDraft
    } from "@/lib/local-ai-model"
    import type { LocalAiReport, LocalAiSettings } from "@/lib/local-ai-model"

    import { Alert, Button, Text } from "@svelteuidev/core"

    $: t = (key: string) => translate($translations, key)

    let settings: LocalAiSettings = defaultSettings()
    let report: LocalAiReport | null = null
    let actionError = ""
    let saved = false
    let busy = false

    $: localIssues = validateDraft(settings)
    $: reportLevel = report ? report.level : "ok"

    onMount(async () => {
        await load()
    })

    async function load() {
        try {
            settings = normalizeSettings(await localAiApi.getConfig())
            await validate()
        } catch (error) {
            actionError = describeError(error)
        }
    }

    function describeError(error: unknown): string {
        if (typeof error === "string") return error
        if (error instanceof Error) return error.message
        return String(error)
    }

    /** Re-runs the core validation; it reads the files and the free memory. */
    async function validate() {
        try {
            report = await localAiApi.validate(settings)
            actionError = ""
        } catch (error) {
            actionError = describeError(error)
        }
    }

    async function browseServer() {
        const path = await localAiApi.selectServer()
        if (path) {
            updateServer("server_path", path)
            await validate()
        }
    }

    async function browseModel() {
        const path = await localAiApi.selectModel()
        if (path) {
            updateServer("model_path", path)
            await validate()
        }
    }

    async function save() {
        busy = true
        try {
            settings = normalizeSettings(settings)
            report = await localAiApi.saveConfig(settings)
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

    async function exportConfig() {
        try {
            await localAiApi.exportConfig(normalizeSettings(settings))
        } catch (error) {
            actionError = describeError(error)
        }
    }

    async function importConfig() {
        try {
            const imported = await localAiApi.importConfig()
            if (imported) {
                settings = normalizeSettings(imported)
                // An imported file is applied at once, so the form and the
                // running gateway never disagree.
                report = await localAiApi.saveConfig(settings)
            }
        } catch (error) {
            actionError = describeError(error)
        }
    }

    function updateServer(field: string, value: string | number) {
        settings = { ...settings, server: { ...settings.server, [field]: value } }
    }

    function onServerInput(field: string, event: Event) {
        updateServer(field, (event.currentTarget as HTMLInputElement).value)
    }

    function onServerNumber(field: string, event: Event) {
        const value = Number.parseInt((event.currentTarget as HTMLInputElement).value, 10)
        updateServer(field, Number.isFinite(value) ? value : 0)
    }

    function onNumber(field: string, event: Event) {
        const value = Number.parseFloat((event.currentTarget as HTMLInputElement).value)
        settings = { ...settings, [field]: Number.isFinite(value) ? value : 0 }
    }
</script>

<div class="ai-settings">
    <Text weight={600}>{t('ai-settings-title')}</Text>
    <p class="ai-desc">{t('ai-settings-desc')}</p>

    <label class="ai-field">
        <span>{t('ai-settings-server-path')}</span>
        <div class="ai-row">
            <input
                type="text"
                class="ai-input"
                placeholder="llama-server.exe"
                autocomplete="off"
                value={settings.server.server_path}
                on:input={(event) => onServerInput("server_path", event)}
            />
            <Button size="xs" color="gray" uppercase on:click={browseServer}>
                {t('ai-settings-browse')}
            </Button>
        </div>
    </label>

    <label class="ai-field">
        <span>{t('ai-settings-model-path')}</span>
        <div class="ai-row">
            <input
                type="text"
                class="ai-input"
                placeholder="Qwen3-8B-Q4_K_M.gguf"
                autocomplete="off"
                value={settings.server.model_path}
                on:input={(event) => onServerInput("model_path", event)}
            />
            <Button size="xs" color="gray" uppercase on:click={browseModel}>
                {t('ai-settings-browse')}
            </Button>
        </div>
    </label>

    <div class="ai-grid">
        <label class="ai-field">
            <span>{t('ai-settings-host')}</span>
            <input
                type="text"
                class="ai-input"
                autocomplete="off"
                value={settings.server.host}
                on:input={(event) => onServerInput("host", event)}
            />
            <small>{t('ai-settings-host-desc')}</small>
        </label>

        <label class="ai-field">
            <span>{t('ai-settings-port')}</span>
            <input
                type="number"
                class="ai-input"
                min="1024"
                max="65535"
                value={String(settings.server.port)}
                on:input={(event) => onServerNumber("port", event)}
            />
        </label>

        <label class="ai-field">
            <span>{t('ai-settings-context')}</span>
            <input
                type="number"
                class="ai-input"
                min="512"
                max="262144"
                value={String(settings.server.context_size)}
                on:input={(event) => onServerNumber("context_size", event)}
            />
            <small>{t('ai-settings-context-desc')}</small>
        </label>

        <label class="ai-field">
            <span>{t('ai-settings-threads')}</span>
            <input
                type="number"
                class="ai-input"
                min="0"
                max="256"
                value={String(settings.server.cpu_threads)}
                on:input={(event) => onServerNumber("cpu_threads", event)}
            />
            <small>{t('ai-settings-threads-desc')}</small>
        </label>

        <label class="ai-field">
            <span>{t('ai-settings-gpu-layers')}</span>
            <input
                type="number"
                class="ai-input"
                min="0"
                max="1000"
                value={String(settings.server.gpu_layers)}
                on:input={(event) => onServerNumber("gpu_layers", event)}
            />
            <small>{t('ai-settings-gpu-layers-desc')}</small>
        </label>

        <label class="ai-field">
            <span>{t('ai-settings-timeout')}</span>
            <input
                type="number"
                class="ai-input"
                min="5"
                max="900"
                value={String(settings.server.startup_timeout_seconds)}
                on:input={(event) => onServerNumber("startup_timeout_seconds", event)}
            />
        </label>

        <label class="ai-field">
            <span>{t('ai-settings-temperature')}</span>
            <input
                type="number"
                class="ai-input"
                min="0"
                max="2"
                step="0.05"
                value={String(settings.temperature)}
                on:input={(event) => onNumber("temperature", event)}
            />
        </label>

        <label class="ai-field">
            <span>{t('ai-settings-top-p')}</span>
            <input
                type="number"
                class="ai-input"
                min="0.05"
                max="1"
                step="0.05"
                value={String(settings.top_p)}
                on:input={(event) => onNumber("top_p", event)}
            />
        </label>

        <label class="ai-field">
            <span>{t('ai-settings-max-tokens')}</span>
            <input
                type="number"
                class="ai-input"
                min="1"
                max="32768"
                value={String(settings.max_tokens)}
                on:input={(event) => onNumber("max_tokens", event)}
            />
        </label>
    </div>

    <p class="ai-desc">{t('ai-settings-loopback-note')}</p>

    <div class="ai-report">
        <Text size="sm" weight={600}>{t('ai-settings-report-title')}</Text>
        <p class:ok={reportLevel === "ok"} class:warn={reportLevel === "warning"} class:bad={reportLevel === "blocked"}>
            {t(reportLevelKey(reportLevel))}
        </p>

        {#if report?.model}
            <p class="ai-model">{describeModel(report.model)}</p>
            <p class="ai-declared">
                {t('ai-settings-architecture')}: {report.model.gguf.architecture ?? "—"} ·
                {t('ai-settings-quantisation')}: {report.model.gguf.quantisation ?? "—"}
            </p>
            <p class="ai-declared">{t('ai-settings-declared-note')}</p>
        {/if}

        {#if report}
            <p class="ai-declared">
                {t('ai-settings-memory')}: {formatBytes(report.resources.estimated_need_bytes)} ·
                {t('ai-settings-memory-available')}: {formatBytes(report.resources.available_ram_bytes)}
            </p>
        {/if}

        <ul class="ai-issues">
            {#each localIssues as issue}
                <li class:warn={issue.level === "warning"} class:bad={issue.level === "blocked"}>
                    <strong>{t(fieldLabelKey(issue.field))}:</strong> {t(issue.key)}
                </li>
            {/each}
            {#each report?.issues ?? [] as issue}
                <li class:warn={issue.level === "warning"} class:bad={issue.level === "blocked"}>
                    <strong>{t(fieldLabelKey(issue.field))}:</strong> {issue.message}
                </li>
            {/each}
        </ul>
    </div>

    {#if actionError}
        <Alert color="orange" variant="outline" title={t('settings-attention')}>
            <Text size="sm">{actionError}</Text>
        </Alert>
    {/if}

    {#if saved}
        <Text size="sm" color="teal">{t('ai-settings-saved')}</Text>
    {/if}

    <div class="ai-actions">
        <Button color="lime" size="sm" uppercase on:click={save} disabled={busy}>
            {t('ai-settings-save')}
        </Button>
        <Button color="gray" size="sm" uppercase variant="outline" on:click={exportConfig}>
            {t('ai-settings-export')}
        </Button>
        <Button color="gray" size="sm" uppercase variant="outline" on:click={importConfig}>
            {t('ai-settings-import')}
        </Button>
    </div>
</div>

<style lang="scss">
.ai-settings {
    display: flex;
    flex-direction: column;
    gap: 0.7rem;
}

.ai-desc {
    margin: 0;
    font-size: 0.75rem;
    color: rgba(255, 255, 255, 0.5);
    white-space: pre-line;
}

.ai-field {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;

    span {
        font-size: 0.75rem;
        color: rgba(255, 255, 255, 0.75);
    }

    small {
        font-size: 0.65rem;
        color: rgba(255, 255, 255, 0.35);
    }
}

.ai-input {
    width: 100%;
    background: rgba(30, 40, 45, 0.9);
    color: #fff;
    border: 1px solid rgba(255, 255, 255, 0.12);
    border-radius: 6px;
    padding: 0.35rem 0.5rem;
    font-family: inherit;
    font-size: 0.8rem;

    &:focus {
        outline: none;
        border-color: rgba(82, 254, 254, 0.4);
    }
}

.ai-row {
    display: flex;
    gap: 0.5rem;
    align-items: center;

    .ai-input {
        flex: 1;
    }
}

.ai-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(190px, 1fr));
    gap: 0.6rem;
}

.ai-report {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    padding: 0.6rem;
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: 8px;
    background: rgba(0, 0, 0, 0.22);

    p {
        margin: 0;
        font-size: 0.75rem;
    }

    .ok {
        color: #52fefa;
    }

    .warn {
        color: #ffb84d;
    }

    .bad {
        color: #ff6b6b;
    }
}

.ai-model {
    font-family: monospace;
    color: rgba(255, 255, 255, 0.75);
}

.ai-declared {
    color: rgba(255, 255, 255, 0.45);
}

.ai-issues {
    margin: 0.2rem 0 0;
    padding-left: 1.1rem;
    font-size: 0.73rem;
    color: rgba(255, 255, 255, 0.6);

    li.warn {
        color: #ffb84d;
    }

    li.bad {
        color: #ff6b6b;
    }
}

.ai-actions {
    display: flex;
    gap: 0.5rem;
    flex-wrap: wrap;
}
</style>

import { writable } from "svelte/store"
import { invoke } from "@tauri-apps/api/core"

// ### RE-EXPORT IPC STORES
export {
    jarvisState,
    ipcConnected,
    lastRecognizedText,
    lastExecutedCommand,
    lastError,
    connectIpc,
    enableIpc,
    disableIpc,
    disconnectIpc,
    sendAction,
    sendIpcMessage,
    sendTextCommand,
    stopJarvisApp,
    reloadCommands
} from "./lib/ipc"

// re-export i18n
export {
    translations,
    currentLanguage,
    translate,
    loadTranslations,
    setLanguage,
    loadLanguage,
    getSupportedLanguages
} from "./lib/i18n"

// ### RUNNING STATE
export const isJarvisRunning = writable(false)
export const jarvisRamUsage = writable(0)
export const jarvisCpuUsage = writable(0)

// ### ASSISTANT VOICE
export const assistantVoice = writable("")

/**
 * What the command field on the home page holds.
 *
 * It is a store and not local state because two places fill it: the person
 * typing, and the dictation panel's "insert into the command field" button. The
 * text is never sent from here — it is a draft until the person presses Enter.
 */
export const commandDraft = writable("")

/**
 * The last stages a spoken phrase passed, newest first.
 *
 * One entry per safe diagnostic the voice host sends: the stage name, the length
 * of the text the matcher saw, a reason code, a command id and an outcome. There
 * is no transcript in an entry and nothing here is persisted — it is a short
 * in-memory ring, so "why was my command not accepted" has an answer that does not
 * involve reading a log.
 */
export interface CommandStageEntry {
    stage: string
    length: number
    code: string | null
    command_id: string | null
    success: boolean | null
}

export const commandStages = writable<CommandStageEntry[]>([])

/** Keeps the last stages, newest first, and never grows without a bound. */
export function noteCommandStage(entry: CommandStageEntry) {
    commandStages.update((entries) => [entry, ...entries].slice(0, 20))
}

// ### APP INFO
export const appInfo = writable({
    tgOfficialLink: "",
    feedbackLink: "",
    repositoryLink: "",
    boostySupportLink: "",
    patreonSupportLink: "",
    logFilePath: ""
})

// ### INIT FUNCTIONS (call these from a component)
export async function loadVoiceSetting() {
    try {
        const voice = await invoke<string>("db_read", { key: "assistant_voice" })
        assistantVoice.set(voice)
    } catch (err) {
        console.error("failed to load voice setting:", err)
    }
}

export async function loadAppInfo() {
    try {
        const [tg, feedback, repo, boosty, patreon, logPath] = await Promise.all([
            invoke<string>("get_tg_official_link"),
            invoke<string>("get_feedback_link"),
            invoke<string>("get_repository_link"),
            invoke<string>("get_boosty_link"),
            invoke<string>("get_patreon_link"),
            invoke<string>("get_log_file_path")
        ])

        appInfo.set({
            tgOfficialLink: tg,
            feedbackLink: feedback,
            repositoryLink: repo,
            boostySupportLink: boosty,
            patreonSupportLink: patreon,
            logFilePath: logPath
        })
    } catch (err) {
        console.error("failed to load app info:", err)
    }
}

export async function updateJarvisStats() {
    try {
        const stats = await invoke<{running: boolean, ram_mb: number, cpu_usage: number}>("get_jarvis_app_stats")
        isJarvisRunning.set(stats.running)
        jarvisRamUsage.set(stats.ram_mb)
        jarvisCpuUsage.set(stats.cpu_usage)
    } catch (err) {
        console.error("failed to get jarvis stats:", err)
    }
}

// polling manager
let statsInterval: ReturnType<typeof setInterval> | null = null

export function startStatsPolling(intervalMs = 5000) {
    if (statsInterval) return // already running
    
    updateJarvisStats()
    statsInterval = setInterval(updateJarvisStats, intervalMs)
}

export function stopStatsPolling() {
    if (statsInterval) {
        clearInterval(statsInterval)
        statsInterval = null
    }
}
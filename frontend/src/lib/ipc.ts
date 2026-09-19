import { writable, get } from "svelte/store"
import { invoke } from "@tauri-apps/api/core"
import { getCurrentWindow } from "@tauri-apps/api/window"

// ### IPC STORES ###

export type JarvisState = "disconnected" | "idle" | "listening" | "processing"

export const jarvisState = writable<JarvisState>("disconnected")
export const ipcConnected = writable(false)
export const lastRecognizedText = writable("")
export const lastExecutedCommand = writable("")
export const lastError = writable("")

// ### CONNECTION ###

const IPC_URL = "ws://127.0.0.1:9712"
const RECONNECT_DELAY = 5000

let ws: WebSocket | null = null
let reconnectTimer: ReturnType<typeof setTimeout> | null = null
let manualDisconnect = false
let enabled = false  // only connect when enabled
/** One global dictation at a time, whatever the trigger. */
let globalDictationRunning = false

/**
 * Runs one global dictation, and gives the microphone back to the listener.
 *
 * The listener stopped and told us so over this channel; when the dictation has
 * finished, the same channel is told that listening may resume. Nothing about the
 * transcript travels here: the result goes to the clipboard in the core, and the
 * window only learns that it happened.
 */
async function runGlobalDictation() {
    if (globalDictationRunning) return
    globalDictationRunning = true
    try {
        await invoke("voice_input_start")
    } catch (error) {
        console.warn("voice input: the request failed", error)
    } finally {
        globalDictationRunning = false
        // The listener may listen again: the microphone is free.
        sendAction("set_muted", { muted: false })
    }
}

export function enableIpc() {
    enabled = true
    connectIpc()
}

export function disableIpc() {
    enabled = false
    disconnectIpc()
}

export function connectIpc(port: number = 9712) {
    if (ws?.readyState === WebSocket.OPEN) return

    ws = new WebSocket(`ws://127.0.0.1:${port}`)

    ws.onopen = () => {
        ipcConnected.set(true)
        jarvisState.set("idle")
        console.log("[IPC] connected")
    }

    ws.onclose = () => {
        void invoke("voice_host_note_ipc_closed")
        ipcConnected.set(false)
        console.log("[IPC] disconnected")
    }

    ws.onerror = (err) => {
        console.error("[IPC] error:", err)
    }

    ws.onmessage = (event) => {
        try {
            const msg = JSON.parse(event.data)
            handleEvent(msg)
        } catch (e) {
            console.error("[IPC] failed to parse message:", e)
        }
    }
}

function scheduleReconnect() {
    if (reconnectTimer || manualDisconnect || !enabled) return

    console.log(`IPC: Will retry in ${RECONNECT_DELAY / 1000}s...`)
    reconnectTimer = setTimeout(() => {
        reconnectTimer = null
        connectIpc()
    }, RECONNECT_DELAY)
}

export function disconnectIpc() {
    manualDisconnect = true

    if (reconnectTimer) {
        clearTimeout(reconnectTimer)
        reconnectTimer = null
    }

    if (ws) {
        ws.close()
        ws = null
    }

    ipcConnected.set(false)
    jarvisState.set("disconnected")
}

// ### EVENT HANDLING ###

function handleEvent(data: any) {
    console.log("IPC: Event", data.event, data)

    switch (data.event) {
        case "wake_word_detected":
        case "listening":
            jarvisState.set("listening")
            break

        // The handshake: only a host that introduced itself with the version
        // this build speaks counts as a listener. Anything else is reported as
        // its own state, never as "running".
        case "hello":
            void invoke("voice_host_note_handshake", { protocolVersion: data.protocol_version })
            break

        case "speech_recognized":
            lastRecognizedText.set(data.text || "")
            jarvisState.set("processing")
            break

        // The listener recognised the global voice input phrase and has given the
        // microphone up. The event carries no text: the dictation runs in this
        // process, with its own session, and its result goes to the clipboard.
        case "global_dictation_requested":
            void runGlobalDictation()
            break

        case "command_executed":
            lastExecutedCommand.set(data.id || "")
            break

        case "idle":
            jarvisState.set("idle")
            break

        case "error":
            lastError.set(data.message || "Unknown error")
            break

        case "started":
            jarvisState.set("idle")
            break

        case "stopping":
            jarvisState.set("disconnected")
            break

        case "pong":
            // connection verified
            break

        case "reveal_window":
            // bring window to foreground
            revealWindow()
            break
    }
}

// ### ACTIONS ###

export function sendAction(action: string, payload: Record<string, any> = {}) {
    if (ws?.readyState !== WebSocket.OPEN) {
        return false
    }

    ws.send(JSON.stringify({ action, ...payload }))
    return true
}

export function stopJarvisApp() {
    return sendAction("stop")
}

export function reloadCommands() {
    return sendAction("reload_commands")
}

export function sendIpcMessage(message: object): Promise<void> {
    return new Promise((resolve, reject) => {
        if (!ws || ws.readyState !== WebSocket.OPEN) {
            reject(new Error("IPC not connected"))
            return
        }

        try {
            ws.send(JSON.stringify(message))
            resolve()
        } catch (err) {
            reject(err)
        }
    })
}

export function sendTextCommand(text: string): boolean {
    return sendAction("text_command", { text })
}

async function revealWindow() {
    try {
        const window = getCurrentWindow()
        await window.show()
        await window.unminimize()
        await window.setFocus()
    } catch (e) {
        console.error("[IPC] Failed to reveal window:", e)
    }
}

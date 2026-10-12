import { writable, get } from "svelte/store"
import { invoke } from "@tauri-apps/api/core"
import {recordRequest} from './requestHistory'

// ### IPC STORES ###

export type JarvisState = "disconnected" | "idle" | "listening" | "processing"

export const jarvisState = writable<JarvisState>("disconnected")
export const ipcConnected = writable(false)
export const microphoneMuted = writable(false)
export const dialogueActive = writable(false)
export const microphoneMuteKnown = writable(false)
export const lastRecognizedText = writable("")
export const lastExecutedCommand = writable("")
export const lastError = writable("")
export type JarvisNotification = { title: string, primary: string, detail?: string, id: number }
export const activeNotification = writable<JarvisNotification | null>(null)

// ### CONNECTION ###

const RECONNECT_DELAY = 5000

let ws: WebSocket | null = null
let reconnectTimer: ReturnType<typeof setTimeout> | null = null
let manualDisconnect = false
let enabled = false  // only connect when enabled
let connecting=false

export function enableIpc() {
    enabled = true
    connectIpc()
}

export function disableIpc() {
    enabled = false
    disconnectIpc()
}

export async function connectIpc(port: number = 9713) {
    if (connecting || ws?.readyState === WebSocket.OPEN || ws?.readyState === WebSocket.CONNECTING) return

    manualDisconnect = false
    enabled = true

    connecting=true
    let token:string
    try{token=await invoke<string>('ipc_credentials')}catch{connecting=false;lastError.set('Защищённое голосовое ядро недоступно. Перезапустите JARVIS после обновления.');scheduleReconnect();return}
    connecting=false
    if(!enabled||manualDisconnect)return
    ws = new WebSocket(`ws://127.0.0.1:${port}`)
    const connection=ws
    const authTimeout=setTimeout(()=>{if(ws===connection&&!get(ipcConnected)){lastError.set('Голосовое ядро не подтвердило защищённое подключение. Обновите оба процесса JARVIS.');connection.close()}},8000)

    ws.onopen = () => {
        if(ws!==connection)return
        ws?.send(JSON.stringify({action:'authenticate',token}))
    }

    ws.onclose = () => {
        clearTimeout(authTimeout);if(ws!==connection)return
        dialogueActive.set(false)
        ipcConnected.set(false)
        microphoneMuteKnown.set(false)
        jarvisState.set("disconnected")
        console.log("[IPC] disconnected")
        scheduleReconnect()
    }

    ws.onerror = (err) => {
        if(ws!==connection)return
        console.error("[IPC] error:", err)
    }

    ws.onmessage = (event) => {
        if(ws!==connection)return
        try {
            const msg = JSON.parse(event.data)
            if(msg.event==='authenticated'){clearTimeout(authTimeout);ipcConnected.set(true);jarvisState.set('idle');ws?.send(JSON.stringify({action:'get_muted'}));return}
            if(!get(ipcConnected))return
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
    dialogueActive.set(false)
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
    microphoneMuteKnown.set(false)
    jarvisState.set("disconnected")
}

// ### EVENT HANDLING ###

function handleEvent(data: any) {
    // Personal event payloads must not be copied to browser logs.

    switch (data.event) {
        case "dialogue_mode":
            dialogueActive.set(Boolean(data.active))
            break
        case "center_command":
            window.dispatchEvent(new CustomEvent("jarvis-center-command", { detail: data.text || "" }));
            break;
        case "wake_word_detected":
        case "listening":
            jarvisState.set("listening")
            break

        case "speech_recognized":
            recordRequest(data.text || '', 'voice')
            lastRecognizedText.set(data.text || "")
            jarvisState.set("processing")
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

        case "notification":
            activeNotification.set({
                title: data.title || "JARVIS",
                primary: data.primary || "",
                detail: data.detail || undefined,
                id: Date.now()
            })
            break

        case "microphone_muted":
            microphoneMuted.set(Boolean(data.muted))
            microphoneMuteKnown.set(true)
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
    if(!get(ipcConnected))return false
    if (ws?.readyState !== WebSocket.OPEN) {
        return false
    }

    ws.send(JSON.stringify({ action, ...payload }))
    return true
}

export function stopJarvisApp() {
    return sendAction("stop")
}

export function setMicrophoneMuted(muted: boolean): boolean {
    return sendAction("set_muted", { muted })
}

export function reloadCommands() {
    return sendAction("reload_commands")
}

export function sendIpcMessage(message: object): Promise<void> {
    return new Promise((resolve, reject) => {
        if (!get(ipcConnected) || !ws || ws.readyState !== WebSocket.OPEN) {
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
        await invoke("animate_window_in", { reducedMotion: window.matchMedia("(prefers-reduced-motion: reduce)").matches })
    } catch (e) {
        console.error("[IPC] Failed to reveal window:", e)
    }
}

<script lang="ts">
    import {resolvedAppearance} from "./lib/appearance"
    import { onMount, onDestroy } from "svelte"
    import { invoke } from "@tauri-apps/api/core"
    import { Router } from "@roxi/routify"
    import routes from "../.routify/routes.default.js"
    import { SvelteUIProvider } from "@svelteuidev/core"
    import Events from "./Events.svelte"
    import JarvisNotification from "./components/JarvisNotification.svelte"
    import CenterReminderWatcher from "./components/CenterReminderWatcher.svelte"
    import CenterTimerWatcher from "./components/CenterTimerWatcher.svelte"
    import ScheduleWebsiteDialog from './components/ScheduleWebsiteDialog.svelte'
    import { initListeningShortcut } from "./lib/listeningShortcut"
    let shortcutCleanup: (() => void) | undefined
    let destroyed = false
    let remoteTimer: ReturnType<typeof setInterval>
    let remoteBusy=false,remoteNotice=""
    import {get} from "svelte/store"
    import {ipcConnected,sendIpcMessage} from "./lib/ipc"
    async function remoteCommands(){if(remoteBusy||!get(ipcConnected))return;remoteBusy=true;try{const requests=await invoke<{id:string,command_id:string,expires_at:number}[]>("lan_sync_take_commands");for(const r of requests){remoteNotice="Команда с телефона · "+r.command_id;try{await sendIpcMessage({action:"remote_command",id:r.command_id,request_id:r.id,expires_at:r.expires_at})}catch{await invoke("lan_sync_command_result",{id:r.id,status:"uncertain"})}}}catch{/* Keep requests on disk; never replay a claimed request. */}finally{remoteBusy=false}}

    import {
        loadVoiceSetting,
        loadAppInfo,
        startStatsPolling,
        stopStatsPolling,
        connectIpc,
        disconnectIpc,
        loadTranslations
    } from "@/stores"

    onMount(() => {
        initListeningShortcut().then(cleanup => { if (destroyed) cleanup(); else shortcutCleanup = cleanup }).catch(console.error)
        invoke("animate_window_in", { reducedMotion: window.matchMedia("(prefers-reduced-motion: reduce)").matches }).catch(error => console.error("Не удалось показать окно JARVIS", error))
        // load static data
        loadVoiceSetting()
        loadAppInfo()

        // start process monitoring
        startStatsPolling(5000)

        // connect to IPC
        connectIpc()

        remoteTimer=setInterval(remoteCommands,1000)
        // load language
        loadTranslations()
    })

    onDestroy(() => {
        destroyed = true
        clearInterval(remoteTimer)
        shortcutCleanup?.()
        stopStatsPolling()
        disconnectIpc()
    })
</script>

<SvelteUIProvider themeObserver={$resolvedAppearance} withNormalizeCSS withGlobalStyles>
    <Router {routes} />
</SvelteUIProvider>

<Events />
<JarvisNotification />
<CenterReminderWatcher />
<CenterTimerWatcher />
<ScheduleWebsiteDialog />

{#if remoteNotice}<div role="status" class="remote-notice">{remoteNotice}<button aria-label="Закрыть сообщение" on:click={()=>remoteNotice=""}>×</button></div>{/if}
<style>.remote-notice{position:fixed;bottom:20px;right:20px;z-index:10000;max-width:90vw;background:#123136;color:#cffff4;border:1px solid #3a6869;padding:12px 16px;border-radius:14px;font-size:14px}.remote-notice button{margin-left:16px;background:none;border:0;color:inherit;font-size:22px;cursor:pointer}</style>

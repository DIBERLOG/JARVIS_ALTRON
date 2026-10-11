<script lang="ts">
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

        // load language
        loadTranslations()
    })

    onDestroy(() => {
        destroyed = true
        shortcutCleanup?.()
        stopStatsPolling()
        disconnectIpc()
    })
</script>

<SvelteUIProvider themeObserver="dark" withNormalizeCSS withGlobalStyles>
    <Router {routes} />
</SvelteUIProvider>

<Events />
<JarvisNotification />
<CenterReminderWatcher />
<CenterTimerWatcher />
<ScheduleWebsiteDialog />

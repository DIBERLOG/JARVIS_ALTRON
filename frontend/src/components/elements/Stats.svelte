<script lang="ts">
    import { invoke } from "@tauri-apps/api/core"
    import { onMount } from "svelte"
    import { ipcConnected, jarvisRamUsage } from "@/stores"

    type Health = {
        microphone_ready: boolean
        microphone_name: string
        neural_ready: boolean
        tts_mode: string
        tts_ready: boolean
        voicemod_running: boolean
        internet_available: boolean
    }

    let health: Health = {
        microphone_ready: false, microphone_name: "Проверка…",
        neural_ready: false, tts_mode: "—", tts_ready: false,
        voicemod_running: false, internet_available: false
    }
    let checked = false

    async function refresh() {
        try {
            health = await invoke<Health>("get_health_status")
            checked = true
        } catch (error) {
            console.error("Failed to check Jarvis status:", error)
            checked = false
        }
    }

    onMount(() => {
        refresh()
        const timer = window.setInterval(refresh, 10000)
        return () => window.clearInterval(timer)
    })

    function short(value: string): string {
        return value.length > 25 ? value.slice(0, 24) + "…" : value
    }

    // RAM bar scale: the assistant normally stays well under 1 GB.
    const RAM_SCALE_MB = 1024
    $: ramPercent = Math.min(100, Math.round(($jarvisRamUsage || 0) / RAM_SCALE_MB * 100))

    // Stroke icons (24×24), drawn with currentColor.
    const icons = {
        mic: "M12 3a3 3 0 0 0-3 3v6a3 3 0 0 0 6 0V6a3 3 0 0 0-3-3ZM5 11a7 7 0 0 0 14 0M12 18v3M8.5 21h7",
        neural: "M12 4a3 3 0 0 0-3 3v.2A3 3 0 0 0 6.5 12 3 3 0 0 0 9 16.8v.2a3 3 0 0 0 6 0v-.2A3 3 0 0 0 17.5 12 3 3 0 0 0 15 7.2V7a3 3 0 0 0-3-3ZM12 4v16",
        voice: "M4 10v4M8 7v10M12 4v16M16 8v8M20 11v2",
        voicemod: "M4 14v-2a8 8 0 0 1 16 0v2M4 14a2 2 0 0 1 2-2h1v6H6a2 2 0 0 1-2-2v-2ZM20 14a2 2 0 0 0-2-2h-1v6h1a2 2 0 0 0 2-2v-2Z",
        globe: "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18ZM3 12h18M12 3c2.5 2.6 3.8 5.6 3.8 9s-1.3 6.4-3.8 9c-2.5-2.6-3.8-5.6-3.8-9S9.5 5.6 12 3Z",
        chip: "M8 8h8v8H8zM10 3v3M14 3v3M10 18v3M14 18v3M3 10h3M3 14h3M18 10h3M18 14h3",
    }

    $: items = [
        { icon: icons.mic, title: "Микрофон", text: short(health.microphone_name), hint: health.microphone_name, on: checked && health.microphone_ready },
        { icon: icons.neural, title: "Нейросеть", text: checked && health.neural_ready && $ipcConnected ? "Связь с Jarvis" : "Нет связи", on: checked && health.neural_ready && $ipcConnected },
        { icon: icons.voice, title: "Озвучка", text: `${health.tts_mode === "xtts" ? "XTTS" : "Silero"} · ${checked && health.tts_ready ? "готова" : "недоступна"}`, on: checked && health.tts_ready },
        { icon: icons.voicemod, title: "Voicemod", text: checked && health.voicemod_running ? "Запущен" : "Не запущен", on: checked && health.voicemod_running },
        { icon: icons.globe, title: "Интернет", text: checked && health.internet_available ? "Доступен" : "Нет доступа", on: checked && health.internet_available },
        { icon: icons.chip, title: "Ресурсы", text: `RAM ${$jarvisRamUsage || "—"} МБ`, on: $jarvisRamUsage > 0, ram: true },
    ]
</script>

<div class="stats-bar" aria-label="Состояние Jarvis">
    {#each items as item, index (item.title)}
        <div class="stat-card" class:on={item.on} style="--i:{index}" title={item.hint ?? item.text}>
            <span class="stat-icon" aria-hidden="true">
                <svg viewBox="0 0 24 24" width="18" height="18"><path d={item.icon} /></svg>
            </span>
            <div class="stat-body">
                <b>{item.title}</b>
                <small>{item.text}</small>
                {#if item.ram}<span class="ram-bar" aria-hidden="true"><i style="width:{ramPercent}%"></i></span>{/if}
            </div>
            <span class="stat-dot" role="img" aria-label={item.on ? "в порядке" : "недоступно"}></span>
        </div>
    {/each}
</div>

<style lang="scss">
    .stats-bar {
        display: grid;
        grid-template-columns: repeat(3, minmax(0, 1fr));
        gap: .75rem;
        width: min(100%, 900px);
        margin: 0 auto;
        padding: 1.25rem 1.5rem;
    }

    .stat-card {
        --accent: #2adbb8;
        position: relative;
        display: flex;
        align-items: center;
        gap: .8rem;
        min-width: 0;
        padding: .8rem .95rem;
        overflow: hidden;
        border: 1px solid #1c3439;
        border-radius: 14px;
        background: linear-gradient(140deg, rgba(20, 44, 49, .55), rgba(9, 18, 22, .75));
        box-shadow: inset 0 1px rgba(160, 255, 244, .05);
        transition: border-color .3s, transform .35s cubic-bezier(.22, 1, .36, 1), box-shadow .35s;
        // Staggered rise from below.
        animation: card-rise .7s cubic-bezier(.22, 1, .36, 1) both;
        animation-delay: calc(var(--i) * 70ms + 120ms);

        // Soft light that follows the top edge on hover.
        &::before {
            content: "";
            position: absolute;
            inset: 0;
            background: radial-gradient(120% 80% at 0% 0%, rgba(42, 219, 184, .12), transparent 60%);
            opacity: 0;
            transition: opacity .35s;
            pointer-events: none;
        }

        &:hover {
            border-color: #2c5a5f;
            transform: translateY(-3px);
            box-shadow: 0 12px 28px rgba(0, 0, 0, .35), inset 0 1px rgba(160, 255, 244, .08);
            &::before { opacity: 1; }
        }

        &:not(.on) { --accent: #6b7a80; }
    }

    @keyframes card-rise {
        from { opacity: 0; transform: translateY(18px) scale(.97); filter: blur(4px); }
        60% { filter: blur(0); }
        to { opacity: 1; transform: none; filter: none; }
    }

    .stat-icon {
        display: grid;
        flex: none;
        place-items: center;
        width: 36px;
        height: 36px;
        border: 1px solid color-mix(in srgb, var(--accent) 30%, transparent);
        border-radius: 10px;
        background: color-mix(in srgb, var(--accent) 10%, transparent);
        color: var(--accent);
        transition: color .3s, background .3s, border-color .3s;

        svg { fill: none; stroke: currentColor; stroke-width: 1.7; stroke-linecap: round; stroke-linejoin: round; }
    }

    .stat-body {
        display: flex;
        flex: 1;
        min-width: 0;
        flex-direction: column;
        gap: .2rem;

        b { color: #edfafa; font-size: .7rem; font-weight: 700; letter-spacing: .12em; text-transform: uppercase; }
        small { overflow: hidden; color: rgba(220, 245, 247, .58); font-size: .72rem; text-overflow: ellipsis; white-space: nowrap; }
    }

    .ram-bar {
        display: block;
        height: 3px;
        margin-top: .2rem;
        overflow: hidden;
        border-radius: 3px;
        background: rgba(255, 255, 255, .06);

        i {
            display: block;
            height: 100%;
            border-radius: inherit;
            background: linear-gradient(90deg, #1fa7a0, #52fefe);
            box-shadow: 0 0 8px rgba(82, 254, 254, .5);
            transition: width .8s cubic-bezier(.22, 1, .36, 1);
        }
    }

    .stat-dot {
        position: relative;
        flex: none;
        width: 8px;
        height: 8px;
        border-radius: 50%;
        background: #4f5b60;
        transition: background .3s, box-shadow .3s;
    }

    .on .stat-dot {
        background: var(--accent);
        box-shadow: 0 0 10px var(--accent);

        // Calm "alive" ring.
        &::after {
            content: "";
            position: absolute;
            inset: -4px;
            border: 1px solid var(--accent);
            border-radius: 50%;
            animation: dot-pulse 2.4s ease-out infinite;
            animation-delay: calc(var(--i) * 300ms);
        }
    }

    @keyframes dot-pulse {
        from { opacity: .7; transform: scale(.6); }
        to { opacity: 0; transform: scale(1.8); }
    }

    @media (max-width: 760px) { .stats-bar { grid-template-columns: repeat(2, minmax(0, 1fr)); } }
    @media (max-width: 460px) { .stats-bar { grid-template-columns: minmax(0, 1fr); } }

    @media (prefers-reduced-motion: reduce) {
        .stat-card, .on .stat-dot::after { animation: none; }
        .stat-card:hover { transform: none; }
    }
</style>

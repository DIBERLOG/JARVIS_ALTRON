<script lang="ts">
    import { goto, activeRoute } from "@roxi/routify"
    import { invoke } from "@tauri-apps/api/core"
    import { onMount, tick } from "svelte"
    import { fly } from "svelte/transition"
    import { cubicOut } from "svelte/easing"
    import { currentLanguage, setLanguage, translations, translate } from "@/stores"
    import { ipcConnected, microphoneMuted, microphoneMuteKnown, setMicrophoneMuted } from "@/lib/ipc"

    let appVersion = ""
    let commandsCount = 0

    let langDropdownOpen = false
    let navOpen = false
    let reducedMotion = false

    // Primary sections. They used to hide behind a «Разделы» dropdown;
    // now they are always visible and collapse into the menu only on narrow windows.
    type NavItem = { path: string, label: string, icon: string, badge?: "commands", tag?: string, title?: string }
    $: t = (key: string) => translate($translations, key)
    $: items = [
        { path: "/", label: "Ассистент", icon: "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18Zm0 4.5a4.5 4.5 0 1 1 0 9 4.5 4.5 0 0 1 0-9Z" },
        { path: "/center", label: "Центр", icon: "M4 4h7v7H4zM13 4h7v4h-7zM13 10h7v10h-7zM4 13h7v7H4z" },
        { path: "/chat", label: "Чат", icon: "M5 5h14a1 1 0 0 1 1 1v9a1 1 0 0 1-1 1H10l-4 3.5V16H5a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1Z" },
        { path: "/commands", label: titleCase(t("header-commands")), icon: "M5 7l4 5-4 5M12 17h7", badge: "commands" },
        { path: "/agents", label: "Агенты", icon: "M12 4l2.4 4.9 5.4.8-3.9 3.8.9 5.4L12 16.4l-4.8 2.5.9-5.4-3.9-3.8 5.4-.8Z", tag: "LAB", title: "Тестовые офисные помощники" },
        { path: "/settings", label: titleCase(t("header-settings")), icon: "M12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6Zm7.4 3a7.4 7.4 0 0 0-.1-1.2l2-1.6-2-3.4-2.4 1a7.6 7.6 0 0 0-2-1.2L14.5 3h-5l-.4 2.6a7.6 7.6 0 0 0-2 1.2l-2.4-1-2 3.4 2 1.6a7.4 7.4 0 0 0 0 2.4l-2 1.6 2 3.4 2.4-1a7.6 7.6 0 0 0 2 1.2l.4 2.6h5l.4-2.6a7.6 7.6 0 0 0 2-1.2l2.4 1 2-3.4-2-1.6c.1-.4.1-.8.1-1.2Z" },
    ] as NavItem[]

    function titleCase(value: string) {
        if (!value || value.startsWith("header-")) return value === "header-commands" ? "Команды" : value === "header-settings" ? "Настройки" : value
        return value.charAt(0).toUpperCase() + value.slice(1).toLowerCase()
    }

    $: currentPath = (($activeRoute as { url?: string } | undefined)?.url ?? location.pathname).split(/[?#]/)[0] || "/"
    function isCurrent(path: string, current: string) {
        return path === "/" ? current === "/" || current === "/index" : current === path || current.startsWith(path + "/")
    }

    function navigate(path: string) { navOpen = false; $goto(path) }

    // Sliding pill under the active tab.
    let navEl: HTMLElement
    let indicator = { x: 0, w: 0, visible: false }
    let indicatorReady = false
    async function moveIndicator() {
        await tick()
        const active = navEl?.querySelector<HTMLElement>(".nav-tab.active")
        if (!active) { indicator = { ...indicator, visible: false }; return }
        indicator = { x: active.offsetLeft, w: active.offsetWidth, visible: true }
        // First placement must not slide in from the left edge.
        if (!indicatorReady) requestAnimationFrame(() => indicatorReady = true)
    }
    $: currentPath, items, moveIndicator()

    async function refreshCommandsCount() {
        try {
            commandsCount = await invoke<number>("get_commands_count")
        } catch (error) {
            console.error("Не удалось обновить количество команд", error)
        }
    }

    onMount(() => {
        reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches
        ;(async () => {
            try {
                appVersion = await invoke<string>("get_app_version")
                await refreshCommandsCount()
            } catch (error) { console.error("Не удалось загрузить заголовок", error) }
        })()
        const observer = new ResizeObserver(() => moveIndicator())
        if (navEl) observer.observe(navEl)
        return () => observer.disconnect()
    })

    async function selectLanguage(code: string) {
        await setLanguage(code)
        langDropdownOpen = false
    }

    function closeMenus(e: MouseEvent) {
        const target = e.target as HTMLElement
        if (!target.closest(".lang-selector")) langDropdownOpen = false
        if (!target.closest(".navigation-menu")) navOpen = false
    }

    const languages = [
        { code: "ru", label: "RU", flag: "🇷🇺", name: "Русский" },
        { code: "en", label: "EN", flag: "🇬🇧", name: "English" },
        { code: "ua", label: "UA", flag: "🇺🇦", name: "Українська" },
    ]
    $: currentLang = languages.find(l => l.code === $currentLanguage) || languages[0]
    $: listening = $ipcConnected && $microphoneMuteKnown && !$microphoneMuted
</script>

<svelte:window on:click={closeMenus} on:focus={refreshCommandsCount} on:keydown={e => { if (e.key === "Escape") { navOpen = false; langDropdownOpen = false } }} />

<header id="header" class="header">
    <div class="header-left">
        <a class="logo" href="/" title="JARVIS" on:click|preventDefault={() => navigate("/")}>
            <img src="/media/128x128.png" alt="" />
            <span class="logo-text">
                <span class="logo-title"><span id="jarvis-logo">&nbsp;</span></span>
                <span class="logo-version"><small>v</small>{appVersion.split("-")[0]} <span class="v-badge">ALTRON</span></span>
            </span>
        </a>
    </div>

    <nav class="primary-nav" aria-label="Разделы приложения" bind:this={navEl}>
        <span class="nav-indicator" class:ready={indicatorReady && !reducedMotion} class:visible={indicator.visible} style="transform:translateX({indicator.x}px);width:{indicator.w}px" aria-hidden="true"></span>
        {#each items as item (item.path)}
            {@const active = isCurrent(item.path, currentPath)}
            <button class="nav-tab" class:active aria-current={active ? "page" : undefined} title={item.title} on:click={() => navigate(item.path)}>
                <svg class="nav-icon" aria-hidden="true" viewBox="0 0 24 24" width="16" height="16"><path d={item.icon} /></svg>
                <span>{item.label}</span>
                {#if item.badge === "commands" && commandsCount}<span class="btn-badge purple">{commandsCount}+</span>{/if}
                {#if item.tag}<span class="nav-tag">{item.tag}</span>{/if}
            </button>
        {/each}
    </nav>

    <div class="header-right">
        <button class="mic-toggle" class:muted={$microphoneMuted} class:listening on:click={() => setMicrophoneMuted(!$microphoneMuted)} disabled={!$ipcConnected || !$microphoneMuteKnown} aria-label={$microphoneMuted ? "Включить глобальное прослушивание микрофона" : "Остановить глобальное прослушивание микрофона"} aria-pressed={$microphoneMuted} title={$microphoneMuted ? "Микрофон на паузе — включить" : "JARVIS слушает — поставить на паузу"}>
            <span class="mic-indicator" aria-hidden="true"></span>
            <span class="mic-label">{!$ipcConnected ? "Нет связи" : $microphoneMuted ? "Пауза" : "Слушаю"}</span>
        </button>

        <div class="lang-selector">
            <button class="lang-btn" aria-label="Выбрать язык" aria-expanded={langDropdownOpen} on:click|stopPropagation={() => langDropdownOpen = !langDropdownOpen}>
                <span class="lang-flag"><img src="/media/flags/{currentLang.label}.png" width="22" alt={currentLang.flag}></span>
            </button>
            {#if langDropdownOpen}
                <div class="lang-dropdown" transition:fly={{ y: -8, duration: reducedMotion ? 0 : 220, easing: cubicOut }}>
                    {#each languages as lang}
                        <button class="lang-option" class:active={lang.code === $currentLanguage} on:click|stopPropagation={() => selectLanguage(lang.code)}>
                            <span class="lang-flag"><img src="/media/flags/{lang.label}.png" width="20" alt={lang.flag}></span>
                            <span class="lang-name">{lang.name}</span>
                        </button>
                    {/each}
                </div>
            {/if}
        </div>

        <!-- Narrow windows only: the tab bar folds into this menu. -->
        <div class="navigation-menu">
            <button class="menu-toggle" class:expanded={navOpen} aria-expanded={navOpen} aria-controls="header-navigation" aria-label="Разделы" on:click={() => navOpen = !navOpen}><span class="menu-icon" aria-hidden="true"><i></i><i></i><i></i></span></button>
            {#if navOpen}
                <nav id="header-navigation" class="compact-nav" aria-label="Разделы приложения (меню)" transition:fly={{ y: -12, duration: reducedMotion ? 0 : 320, easing: cubicOut }}>
                    {#each items as item (item.path)}
                        {@const active = isCurrent(item.path, currentPath)}
                        <button class="menu-item" class:active aria-current={active ? "page" : undefined} on:click={() => navigate(item.path)}>
                            <svg class="nav-icon" aria-hidden="true" viewBox="0 0 24 24" width="16" height="16"><path d={item.icon} /></svg>
                            <span>{item.label}</span>
                            {#if item.badge === "commands" && commandsCount}<span class="btn-badge purple">{commandsCount}+</span>{/if}
                            {#if item.tag}<span class="nav-tag">{item.tag}</span>{/if}
                        </button>
                    {/each}
                </nav>
            {/if}
        </div>
    </div>
</header>

<style lang="scss">
    $ease: cubic-bezier(.22, 1, .36, 1);

    .header { gap: 18px; align-items: center; }
    .header-left { flex-shrink: 0; }
    .logo { display: flex; align-items: center; gap: .85rem; color: inherit; text-decoration: none; }
    .logo:hover { text-decoration: none; }
    .logo img { width: 52px; height: 52px; border-radius: 14px; transition: transform .5s $ease, filter .5s; }
    .logo:hover img { transform: rotate(-6deg) scale(1.04); filter: drop-shadow(0 0 10px #52fefe55); }
    .logo-version { white-space: nowrap; }
    .v-badge { white-space: nowrap; }

    /* ---------- primary tabs ---------- */
    .primary-nav {
        position: relative;
        display: flex;
        align-items: center;
        gap: 2px;
        padding: 5px;
        margin: 0 auto;
        border: 1px solid #2a454b;
        border-radius: 16px;
        background: linear-gradient(180deg, #121e22, #0d171b);
        box-shadow: inset 0 1px 0 #ffffff08;
    }
    .nav-indicator {
        position: absolute;
        top: 5px;
        bottom: 5px;
        left: 0;
        border-radius: 11px;
        background: linear-gradient(135deg, #17575c, #12353b);
        border: 1px solid #4fd6cf66;
        box-shadow: 0 0 22px #35d6cc26, inset 0 1px 0 #ffffff12;
        opacity: 0;
        pointer-events: none;
        &.visible { opacity: 1; }
        &.ready { transition: transform .45s $ease, width .45s $ease, opacity .2s; }
    }
    .nav-tab {
        position: relative;
        z-index: 1;
        display: flex;
        align-items: center;
        gap: 7px;
        height: 38px;
        padding: 0 14px;
        border: 0;
        border-radius: 11px;
        background: transparent;
        color: #9fb9bd;
        font: 650 13px/1 "Manrope Variable", "Segoe UI", sans-serif;
        white-space: nowrap;
        cursor: pointer;
        transition: color .25s, background .25s, transform .15s;
        &:hover { color: #e9fbfb; background: #ffffff08; }
        &:active { transform: scale(.97); }
        &.active { color: #fff; background: transparent; }
        &:focus-visible { outline: 2px solid #72efe6; outline-offset: 2px; }
    }
    .nav-icon { flex: none; fill: none; stroke: currentColor; stroke-width: 1.7; stroke-linecap: round; stroke-linejoin: round; opacity: .8; transition: opacity .25s, transform .35s $ease; }
    .nav-tab:hover .nav-icon { transform: translateY(-1px); }
    .nav-tab.active .nav-icon { opacity: 1; color: #72efe6; }
    .nav-tag { padding: 2px 5px; border: 1px solid #356466; border-radius: 5px; color: #91e8df; font-size: 9px; font-weight: 800; letter-spacing: .08em; }
    .btn-badge { padding: 2px 5px !important; border-radius: 5px !important; font-size: 10px !important; }

    /* ---------- right cluster ---------- */
    .header-right { flex-shrink: 0; flex-wrap: nowrap; justify-content: flex-end; }
    .mic-toggle {
        position: relative;
        display: flex;
        align-items: center;
        gap: 9px;
        height: 38px;
        padding: 0 14px 0 12px;
        border: 1px solid #2f5d60;
        border-radius: 12px;
        background: #11292d;
        color: #c8ffff;
        font: 700 12px/1 "Manrope Variable", "Segoe UI", sans-serif;
        letter-spacing: .02em;
        white-space: nowrap;
        cursor: pointer;
        transition: background .3s, border-color .3s, color .3s, transform .15s;
        &:hover:not(:disabled) { background: #17393e; border-color: #52fefe88; }
        &:active:not(:disabled) { transform: scale(.97); }
        &.muted { border-color: #7a3c40; background: #2a1518; color: #ffd8d8; }
        &:disabled { opacity: .5; cursor: not-allowed; }
        &:focus-visible { outline: 2px solid #fff; outline-offset: 2px; }
    }
    .mic-indicator {
        position: relative;
        width: 9px;
        height: 9px;
        border-radius: 50%;
        background: #52fefe;
        box-shadow: 0 0 8px #52fefe;
        transition: background .3s, box-shadow .3s;
    }
    .mic-indicator::before, .mic-indicator::after {
        content: "";
        position: absolute;
        inset: 0;
        border-radius: 50%;
        border: 1px solid #52fefe;
        opacity: 0;
    }
    .listening .mic-indicator::before { animation: mic-ring 2s $ease infinite; }
    .listening .mic-indicator::after { animation: mic-ring 2s $ease 1s infinite; }
    .muted .mic-indicator { background: #ff8686; box-shadow: 0 0 6px #ff868688; }
    @keyframes mic-ring { 0% { transform: scale(1); opacity: .8; } 100% { transform: scale(3.2); opacity: 0; } }

    .lang-selector { position: relative; }
    .lang-btn {
        display: grid;
        place-items: center;
        width: 38px;
        height: 38px;
        padding: 0;
        background: #18262b;
        border: 1px solid #38555b60;
        border-radius: 12px;
        cursor: pointer;
        transition: background .25s, border-color .25s, transform .15s;
        &:hover { background: #22343a; border-color: #52fefe55; }
        &:active { transform: scale(.95); }
    }
    .lang-flag { line-height: 0; }
    .lang-dropdown {
        position: absolute;
        top: calc(100% + 8px);
        right: 0;
        z-index: 100;
        min-width: 160px;
        padding: 6px;
        overflow: hidden;
        background: #12201f;
        border: 1px solid #ffffff1a;
        border-radius: 14px;
        box-shadow: 0 16px 40px #0008;
    }
    .lang-option {
        display: flex;
        align-items: center;
        gap: .6rem;
        width: 100%;
        padding: .6rem .7rem;
        background: transparent;
        border: none;
        border-radius: 9px;
        color: #ffffffc0;
        font-size: .8rem;
        text-align: left;
        cursor: pointer;
        transition: background .15s, color .15s;
        &:hover { background: #52fefe1a; color: #fff; }
        &.active { background: #52fefe26; color: #52fefe; }
    }

    /* ---------- compact menu (narrow windows) ---------- */
    .navigation-menu { position: relative; display: none; }
    .menu-toggle {
        display: grid;
        place-items: center;
        width: 42px;
        height: 38px;
        border: 1px solid #38555b80;
        border-radius: 12px;
        background: linear-gradient(145deg, #203238, #142227);
        color: #9be4db;
        cursor: pointer;
        transition: background .3s, border-color .3s;
        &:hover, &.expanded { border-color: #68c9bf75; background: linear-gradient(145deg, #28464b, #183237); }
        &:focus-visible { outline: 2px solid #72efe6; outline-offset: 3px; }
    }
    .menu-icon { display: flex; flex-direction: column; gap: 4px; }
    .menu-icon i { display: block; width: 15px; height: 1.5px; background: currentColor; border-radius: 2px; transition: transform .36s $ease, opacity .25s; }
    .expanded .menu-icon i:first-child { transform: translateY(5.5px) rotate(45deg); }
    .expanded .menu-icon i:nth-child(2) { opacity: 0; }
    .expanded .menu-icon i:last-child { transform: translateY(-5.5px) rotate(-45deg); }
    .compact-nav {
        position: absolute;
        top: calc(100% + 10px);
        right: 0;
        z-index: 100;
        width: 240px;
        display: flex;
        flex-direction: column;
        gap: 4px;
        padding: 8px;
        border: 1px solid #3b585b80;
        border-radius: 16px;
        background: linear-gradient(155deg, #17282d, #101b20);
        box-shadow: 0 20px 60px #0008;
    }
    .menu-item {
        display: flex;
        align-items: center;
        gap: 10px;
        width: 100%;
        padding: 12px;
        border: 1px solid transparent;
        border-radius: 10px;
        background: transparent;
        color: #cfe3e5;
        font: 650 13px "Manrope Variable", sans-serif;
        text-align: left;
        cursor: pointer;
        animation: menu-arrive .4s $ease both;
        &:hover { background: #ffffff0a; }
        &.active { border-color: #4fd6cf55; background: #153b3f; color: #fff; }
        .btn-badge, .nav-tag { margin-left: auto; }
    }
    @for $i from 1 through 6 { .menu-item:nth-child(#{$i}) { animation-delay: #{($i - 1) * 30}ms; } }
    @keyframes menu-arrive { from { opacity: 0; transform: translateY(-6px); } to { opacity: 1; transform: none; } }

    @media (max-width: 1080px) {
        .nav-tab { padding: 0 10px; }
        .nav-tab .nav-icon { display: none; }
    }
    @media (max-width: 900px) {
        .primary-nav { display: none; }
        .navigation-menu { display: block; }
        .header-right { margin-left: auto; }
    }
    @media (max-width: 520px) {
        .mic-label { display: none; }
        .mic-toggle { padding: 0 13px; }
        .logo-version { display: none; }
    }

    @media (prefers-reduced-motion: reduce) {
        .menu-item { animation: none; }
        .listening .mic-indicator::before, .listening .mic-indicator::after { animation: none; }
        .menu-icon i, .nav-tab, .logo img, .nav-icon { transition: none; }
    }

    :global(html[data-theme=light]) {
        .primary-nav { background: #fff; border-color: var(--ui-line); }
        .nav-tab { color: var(--ui-muted); background: transparent !important; }
        .nav-tab.active { color: #124b41 !important; }
        .nav-indicator { background: var(--ui-selected); border-color: #428777; box-shadow: none; }
        .mic-toggle.listening { background: var(--ui-selected) !important; }
    }
</style>

<script lang="ts">
    import { goto } from "@roxi/routify"
    import { invoke } from "@tauri-apps/api/core"
    import { onMount } from "svelte"
    import { fly } from "svelte/transition"
    import { cubicOut } from "svelte/easing"
    import { currentLanguage, setLanguage, translations, translate } from "@/stores"
    import { ipcConnected, microphoneMuted, microphoneMuteKnown, setMicrophoneMuted } from "@/lib/ipc"
    
    let appVersion = ""
    let commandsCount = 0

    let selectedLang = "?"
    let langDropdownOpen = false
    let navOpen = false
    let reducedMotion = false
    function navigate(path: string) { navOpen = false; $goto(path) }

    const languages = [
        { code: "ru", label: "RU", flag: "🇷🇺", name: "Русский" },
        { code: "en", label: "EN", flag: "🇬🇧", name: "English" },
        { code: "ua", label: "UA", flag: "🇺🇦", name: "Українська" },
    ]

    async function refreshCommandsCount() {
        try {
            commandsCount = await invoke<number>("get_commands_count")
        } catch (error) {
            console.error("Не удалось обновить количество команд", error)
        }
    }

    onMount(async () => {
        reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches
        try {
            appVersion = await invoke<string>("get_app_version")
            await refreshCommandsCount()

            // load saved language
            const savedLang = await invoke<string>("db_read", { key: "language" })
            if (savedLang) {
                selectedLang = savedLang
            }
        } catch (error) { console.error("Не удалось загрузить заголовок", error) }
    })

    async function selectLanguage(code: string) {
        await setLanguage(code)
        langDropdownOpen = false
    }

    function toggleLangDropdown() {
        langDropdownOpen = !langDropdownOpen
    }

    function closeLangDropdown(e: MouseEvent) {
        const target = e.target as HTMLElement
        if (!target.closest('.lang-selector')) {
            langDropdownOpen = false
        }
        if (!target.closest('.navigation-menu')) navOpen = false
    }

    $: currentLang = languages.find(l => l.code === $currentLanguage) || languages[0]
    $: t = (key: string) => translate($translations, key)
</script>

<svelte:window on:click={closeLangDropdown} on:focus={refreshCommandsCount} on:keydown={e => { if (e.key === 'Escape') { navOpen = false; langDropdownOpen = false } }} />

<header id="header" class="header">
    <div class="header-left">
        <div class="logo">
            <a href="/" title="JARVIS">
                <img src="/media/128x128.png" alt="Jarvis Logo" />
            </a>
            <div class="logo-text">
                <span class="logo-title"><a href="/" id="jarvis-logo">&nbsp;</a></span>
                <span class="logo-version"><small>v</small>{appVersion.split('-')[0]} <span class="v-badge" title="Тестовая сборка синхронизации устройств">Sync · LAB</span></span>
            </div>
        </div>
    </div>
    
    <div class="header-right">
        <button class="mic-toggle" class:muted={$microphoneMuted} on:click={() => setMicrophoneMuted(!$microphoneMuted)} disabled={!$ipcConnected || !$microphoneMuteKnown} aria-label={$microphoneMuted ? "Включить глобальное прослушивание микрофона" : "Остановить глобальное прослушивание микрофона"} aria-pressed={$microphoneMuted} title={$microphoneMuted ? "Микрофон на паузе — включить" : "JARVIS слушает — поставить на паузу"}>
            <span class="mic-indicator" aria-hidden="true"></span>{$microphoneMuted ? "НЕ СЛУШАТЬ" : "СЛУШАТЬ"}
        </button>
        <div class="navigation-menu">
        <button class="menu-toggle" class:expanded={navOpen} aria-expanded={navOpen} aria-controls="header-navigation" on:click={() => navOpen = !navOpen}><span class="menu-icon" aria-hidden="true"><i></i><i></i><i></i></span><span>Разделы</span><svg class="menu-chevron" aria-hidden="true" width="14" height="14" viewBox="0 0 24 24" fill="none"><path d="m7 10 5 5 5-5" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/></svg></button>
        {#if navOpen}
        <nav id="header-navigation" aria-label="Разделы приложения" transition:fly={{ y: -12, duration: reducedMotion ? 0 : 360, easing: cubicOut }}>
        <button class="header-btn center-nav" on:click={() => navigate('/center')}>ЦЕНТР</button>
        <button class="header-btn" on:click={() => navigate('/commands')}>
            <span class="btn-text">{t('header-commands')}</span>
            <span class="btn-badge purple">{commandsCount}+</span>
        </button>

        <button class="header-btn" on:click={() => navigate('/chat')}>
            <span class="btn-text">ЧАТ</span>
        </button>
        <button class="header-btn agents-nav" on:click={() => navigate('/agents')} title="Тестовые офисные помощники">АГЕНТЫ · LAB</button>
        
        <button class="header-btn" on:click={() => navigate('/settings')}>
            <span class="btn-text">{t('header-settings')}</span>
        </button>
        </nav>
        {/if}
        </div>

        <div class="lang-selector">
            <button class="lang-btn" aria-label="Выбрать язык" aria-expanded={langDropdownOpen} on:click|stopPropagation={toggleLangDropdown}>
                <span class="lang-flag"><img src="/media/flags/{currentLang.label}.png" width="23px" alt="{currentLang.flag}"></span>
            </button>
            
            {#if langDropdownOpen}
                <div class="lang-dropdown">
                    {#each languages as lang}
                        <button 
                            class="lang-option" 
                            class:active={lang.code === $currentLanguage}
                            on:click|stopPropagation={() => selectLanguage(lang.code)}
                        >
                            <span class="lang-flag"><img src="/media/flags/{lang.label}.png" width="20px" alt="{lang.flag}"></span>
                            <span class="lang-name">{lang.name}</span>
                        </button>
                    {/each}
                </div>
            {/if}
        </div>
    </div>
</header>

<style lang="scss">
    .navigation-menu { position: relative; }
    .menu-toggle { display:flex; align-items:center; gap:12px; height:42px; padding:0 16px 0 10px; border:1px solid #38555b80; border-radius:14px; background:linear-gradient(145deg,#203238,#142227); box-shadow:inset 0 1px 0 #ffffff0a,0 4px 16px #0003; color:#e5f3f2; font-family:'Manrope Variable',sans-serif; font-size:13px; font-weight:650; letter-spacing:.01em; cursor:pointer; transition:background .3s,border-color .3s,box-shadow .3s; }
    .menu-toggle:hover,.menu-toggle.expanded { background:linear-gradient(145deg,#28464b,#183237); border-color:#68c9bf75; box-shadow:inset 0 1px 0 #ffffff0d,0 5px 20px #0004; }
    .menu-toggle:focus-visible { outline:2px solid #72efe6; outline-offset:3px; }
    .menu-icon { display:flex; flex-direction:column; align-items:center; justify-content:center; gap:4px; width:28px; height:28px; border-radius:9px; background:#79dacf12; color:#9be4db; }
    .menu-icon i { display:block; width:14px; height:1.5px; background:currentColor; border-radius:2px; transition:transform .36s cubic-bezier(.22,1,.36,1),opacity .25s; }
    .expanded .menu-icon i:first-child { transform:translateY(5.5px) rotate(45deg); }
    .expanded .menu-icon i:nth-child(2) { opacity:0; }
    .expanded .menu-icon i:last-child { transform:translateY(-5.5px) rotate(-45deg); }
    .menu-chevron { color:#8daeb1; margin-left:2px; transition:transform .36s cubic-bezier(.22,1,.36,1); }
    .expanded .menu-chevron { transform:rotate(180deg); }
    nav { position:absolute; top:calc(100% + 12px); right:0; z-index:100; width:260px; display:flex; flex-direction:column; gap:6px; padding:12px; border:1px solid #3b585b80; border-radius:18px; background:linear-gradient(155deg,#17282d,#101b20); box-shadow:inset 0 1px 0 #ffffff08,0 20px 60px #0008; }
    nav .header-btn { justify-content:flex-start; width:100%; margin:0; padding:15px; border-radius:10px; animation:menu-arrive .44s cubic-bezier(.22,1,.36,1) both; }
    @for $i from 1 through 5 { nav .header-btn:nth-child(#{$i}) { animation-delay:#{($i - 1) * 35}ms; } }
    @keyframes menu-arrive { from { opacity:0; transform:translateY(-8px); } to { opacity:1; transform:translateY(0); } }
    @media (prefers-reduced-motion:reduce) { nav .header-btn { animation:none; }.menu-icon i,.menu-chevron,.menu-toggle { transition:none; } }
    .header-left { flex-shrink: 0; }
    .logo-version { white-space: nowrap; }
    .v-badge { white-space: nowrap; }
    .header-right { flex-wrap: wrap; justify-content: flex-end; }
    .agents-nav { color: #91e8df; border-color: #356466; }
    .mic-toggle {display:flex;align-items:center;gap:.4rem;padding:.4rem .52rem;border:1px solid rgba(82,254,254,.45);border-radius:6px;background:rgba(22,74,78,.28);color:#c8ffff;font-size:.65rem;font-weight:700;letter-spacing:.04em;white-space:nowrap;cursor:pointer;transition:background .2s,border-color .2s}
    .mic-toggle:hover:not(:disabled) {background:rgba(27,105,110,.48)}
    .mic-toggle.muted {border-color:#e48787;background:rgba(116,37,43,.35);color:#ffd8d8}
    .mic-toggle:disabled {opacity:.45;cursor:not-allowed}
    .mic-toggle:focus-visible {outline:2px solid #fff;outline-offset:2px}
    .mic-indicator {width:.42rem;height:.42rem;border-radius:50%;background:#52fefe;box-shadow:0 0 7px #52fefe}
    .mic-toggle.muted .mic-indicator {background:#ff8686;box-shadow:0 0 7px #ff8686}
    .center-nav {color:#72efe6;border:1px solid #326d70;background:#123237}
    .center-nav:hover {background:#185057}
    .lang-selector {
        position: relative;
    }

    .lang-btn {
        display: flex;
        align-items: center;
        gap: 0.35rem;
        justify-content: center;
        width: 38px;
        height: 38px;
        padding: 0;
        background: #18262b;
        border: 1px solid #38555b60;
        border-radius: 12px;
        color: #ffffff;
        font-size: 0.7rem;
        cursor: pointer;
        transition: background .25s, border-color .25s;

        &:hover {
            background: rgba(35, 50, 55, 0.7);
        }
    }

    .lang-flag {
        font-size: 0.9rem;
        line-height: 1;
    }

    .lang-code {
        font-weight: 600;
        letter-spacing: 0.5px;
    }

    .lang-arrow {
        font-size: 0.8rem;
        opacity: 0.6;
        transition: transform 0.2s ease;

        &.open {
            transform: rotate(180deg);
        }
    }

    .lang-dropdown {
        position: absolute;
        top: calc(100% + 0.35rem);
        right: 0;
        background: rgba(20, 30, 35, 0.98);
        border: 1px solid rgba(255, 255, 255, 0.1);
        border-radius: 6px;
        overflow: hidden;
        z-index: 100;
        min-width: 130px;
        box-shadow: 0 4px 20px rgba(0, 0, 0, 0.4);
    }

    .lang-option {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        width: 100%;
        padding: 0.6rem 0.85rem;
        background: transparent;
        border: none;
        color: rgba(255, 255, 255, 0.75);
        font-size: 0.75rem;
        cursor: pointer;
        transition: all 0.15s ease;
        text-align: left;

        &:hover {
            background: rgba(82, 254, 254, 0.1);
            color: #ffffff;
        }

        &.active {
            background: rgba(82, 254, 254, 0.15);
            color: #52fefe;
        }
    }

    .lang-name {
        font-weight: 500;
    }
</style>

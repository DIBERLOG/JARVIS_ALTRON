<script lang="ts">
    import { goto } from "@roxi/routify"
    import { invoke } from "@tauri-apps/api/core"
    import { onMount } from "svelte"
    import { currentLanguage, setLanguage, translations, translate } from "@/stores"
    import { ipcConnected, microphoneMuted, microphoneMuteKnown, setMicrophoneMuted } from "@/lib/ipc"
    
    let appVersion = ""
    let commandsCount = 0

    let selectedLang = "?"
    let langDropdownOpen = false

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
    }

    $: currentLang = languages.find(l => l.code === $currentLanguage) || languages[0]
    $: t = (key: string) => translate($translations, key)
</script>

<svelte:window on:click={closeLangDropdown} on:focus={refreshCommandsCount} />

<header id="header" class="header">
    <div class="header-left">
        <div class="logo">
            <a href="/" title="JARVIS">
                <img src="/media/128x128.png" alt="Jarvis Logo" />
            </a>
            <div class="logo-text">
                <span class="logo-title"><a href="/" id="jarvis-logo">&nbsp;</a></span>
                <span class="logo-version"><small>v</small>{appVersion} <span class="v-badge" title="Релиз">Release</span></span>
            </div>
        </div>
    </div>
    
    <div class="header-right">
        <button class="mic-toggle" class:muted={$microphoneMuted} on:click={() => setMicrophoneMuted(!$microphoneMuted)} disabled={!$ipcConnected || !$microphoneMuteKnown} aria-label={$microphoneMuted ? "Включить глобальное прослушивание микрофона" : "Остановить глобальное прослушивание микрофона"} aria-pressed={$microphoneMuted} title={$microphoneMuted ? "Микрофон на паузе — включить" : "JARVIS слушает — поставить на паузу"}>
            <span class="mic-indicator" aria-hidden="true"></span>{$microphoneMuted ? "НЕ СЛУШАТЬ" : "СЛУШАТЬ"}
        </button>
        <button class="header-btn center-nav" on:click={() => $goto('/center')}>ЦЕНТР</button>
        <button class="header-btn" on:click={() => $goto('/commands')}>
            <span class="btn-text">{t('header-commands')}</span>
            <span class="btn-badge purple">{commandsCount}+</span>
        </button>

        <button class="header-btn" on:click={() => $goto('/chat')}>
            <span class="btn-text">ЧАТ</span>
        </button>
        <button class="header-btn agents-nav" on:click={() => $goto('/agents')} title="Тестовые офисные помощники">АГЕНТЫ · LAB</button>
        
        <button class="header-btn" on:click={() => $goto('/settings')}>
            <span class="btn-text">{t('header-settings')}</span>
        </button>

        <div class="lang-selector">
            <button class="lang-btn" on:click|stopPropagation={toggleLangDropdown}>
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
        padding: 0.5rem 0.65rem;
        background: transparent;
        border: none;
        border-radius: 6px;
        color: #ffffff;
        font-size: 0.7rem;
        cursor: pointer;

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

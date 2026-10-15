<script lang="ts">
    import {appearance} from "@/lib/appearance"
    import { onMount, onDestroy } from "svelte"
    import { invoke } from "@tauri-apps/api/core"
    import { setTimeout } from "worker-timers"

    import { showInExplorer } from "@/functions"
    import { appInfo, assistantVoice, translations, translate } from "@/stores"
    import { ipcConnected, microphoneMuted, microphoneMuteKnown, setMicrophoneMuted } from "@/lib/ipc"

    import Footer from "@/components/Footer.svelte"
    import ListeningShortcut from "@/components/ListeningShortcut.svelte"
    import CenterDataTransfer from '@/components/CenterDataTransfer.svelte'
    import LanSync from '@/components/LanSync.svelte'
    import ChatProviderSettings from '@/components/ChatProviderSettings.svelte'
    import ScheduleWebsite from '@/components/ScheduleWebsite.svelte'
    import {recordingShortcut} from '@/lib/listeningShortcut'

    import {
        Notification,
        Text,

        Space,
        Alert,
        Input,
        InputWrapper,
        NativeSelect,
        Switch
    } from "@svelteuidev/core"

    import {
        Link2, Archive, Globe, MixerHorizontal, InfoCircled,
        Check,
        Mix,
        Cube,
        Code,
        Gear,
        CrossCircled
    } from "radix-icons-svelte"

    $: t = (key: string) => translate($translations, key)

    interface VoiceMeta {
        id: string
        name: string
        author: string
        languages: string[]
    }

    interface VoiceConfig {
        voice: VoiceMeta
    }
    
    let availableVoices: VoiceMeta[] = []

    async function selectVoice(voiceId: string) {
        voiceVal = voiceId
        
        // play preview sound
        try {
            await invoke("preview_voice", { voiceId })
        } catch (err) {
            console.error("Failed to preview voice:", err)
        }
    }

    // ### STATE
    interface MicrophoneOption {
        label: string
        value: string
    }

    let availableMicrophones: MicrophoneOption[] = []
    let availableVoskModels: { label: string; value: string }[] = []
    let availableGlinerModels: { label: string; value: string }[] = []
    type Section = 'general'|'voice'|'models'|'devices'|'data'|'automation'|'advanced'|'about'
    const sections: {id:Section,title:string,description:string,icon:any,group:string}[] = [
        {id:'general',title:'Основные',description:'Оформление, микрофон и управление слушанием.',icon:Gear,group:'Ассистент'},
        {id:'voice',title:'Голос и звук',description:'Голос, характер ответов и вывод звука.',icon:Mix,group:'Ассистент'},
        {id:'models',title:'Нейросеть',description:'Модель для общения и ключи доступа.',icon:Cube,group:'Ассистент'},
        {id:'devices',title:'Устройства',description:'Подключение телефона и обмен данными.',icon:Link2,group:'Ваши данные'},
        {id:'data',title:'Данные и копии',description:'Перенос записей и резервное копирование.',icon:Archive,group:'Ваши данные'},
        {id:'automation',title:'Сайты и команды',description:'Ваши ссылки для голосовых команд.',icon:Globe,group:'Ещё'},
        {id:'advanced',title:'Дополнительно',description:'Распознавание речи и обработка микрофона.',icon:MixerHorizontal,group:'Ещё'},
        {id:'about',title:'О приложении',description:'Версия JARVIS, поддержка и диагностика.',icon:InfoCircled,group:'Ещё'}
    ]
    let active:Section='general', loaded=false, loadError='', savedSnapshot='', appVersion=''
    function chooseSection(section:Section){recordingShortcut.set(false);active=section}
    $: selectedSection=sections.find(section=>section.id===active)!
    $: settingsValues={voiceVal,selectedMicrophone,selectedWakeWordEngine,selectedIntentRecognitionEngine,selectedSlotExtractionEngine,selectedGlinerModel,selectedVoskModel,selectedNoiseSuppression,selectedVad,gainNormalizerEnabled,voiceDialoguePersonality,ttsMode,audioOutputMode,monitorSelf,apiKeyPicovoice}
    $: dirty=loaded&&JSON.stringify(settingsValues)!==savedSnapshot
    function discard(){if(!loaded||saveButtonDisabled)return;({voiceVal,selectedMicrophone,selectedWakeWordEngine,selectedIntentRecognitionEngine,selectedSlotExtractionEngine,selectedGlinerModel,selectedVoskModel,selectedNoiseSuppression,selectedVad,gainNormalizerEnabled,voiceDialoguePersonality,ttsMode,audioOutputMode,monitorSelf,apiKeyPicovoice}=JSON.parse(savedSnapshot));saveError='';settingsSaved=false}
    let settingsSaved = false
    let saveError = ""
    let saveButtonDisabled = false

    // form values (state vars)
    let voiceVal = ""
    let selectedMicrophone = ""
    let selectedWakeWordEngine = ""
    let selectedIntentRecognitionEngine = ""
    let selectedSlotExtractionEngine = ""
    let selectedGlinerModel = ""
    let selectedVoskModel = ""
    let selectedNoiseSuppression = ""
    let selectedVad = ""
    let gainNormalizerEnabled = false
    let voiceDialoguePersonality = "jarvis"
    let ttsMode = "xtts"
    let audioOutputMode = "direct"
    let monitorSelf = false
    let apiKeyPicovoice = ""

    // subscribe to stores
    const stopVoiceSubscription=assistantVoice.subscribe(value => {
        voiceVal = value
    })

    let feedbackLink = ""
    let logFilePath = ""
    const stopInfoSubscription=appInfo.subscribe(info => {
        feedbackLink = info.feedbackLink
        logFilePath = info.logFilePath
    })
    onDestroy(()=>{stopVoiceSubscription();stopInfoSubscription()})

    // ### FUNCTIONS
    async function saveSettings() {
        if(!loaded||saveButtonDisabled||!dirty)return
        const submitted=JSON.stringify(settingsValues)
        saveButtonDisabled = true
        settingsSaved = false
        saveError = ""

        try {
            await invoke("db_write_many", { entries: [
                { key: "assistant_voice", val: voiceVal },
                { key: "selected_microphone", val: selectedMicrophone },
                { key: "selected_wake_word_engine", val: selectedWakeWordEngine },
                { key: "intent_backend", val: selectedIntentRecognitionEngine },
                { key: "slots_backend", val: selectedSlotExtractionEngine },
                { key: "selected_gliner_model", val: selectedGlinerModel },
                { key: "selected_vosk_model", val: selectedVoskModel },
                { key: "noise_suppression", val: selectedNoiseSuppression },
                { key: "vad_backend", val: selectedVad },
                { key: "gain_normalizer", val: gainNormalizerEnabled.toString() },
                { key: "voice_dialogue_personality", val: voiceDialoguePersonality },
                { key: "api_key__picovoice", val: apiKeyPicovoice },
                { key: "tts_mode", val: ttsMode },
                { key: "audio_output_mode", val: audioOutputMode },
                { key: "monitor_self", val: String(monitorSelf) }
            ] })

            // update shared store
            assistantVoice.set(voiceVal)
            savedSnapshot = submitted
            settingsSaved = true

            // hide alert after 5 seconds
            setTimeout(() => {
                settingsSaved = false
            }, 5000)

            // restart listening with new settings
            // stopListening(() => startListening())
        } catch (err) {
            console.error("failed to save settings:", err)
            saveError = `Не удалось сохранить настройки: ${String(err)}`
        } finally {
            saveButtonDisabled = false
        }
    }

    // ### INIT
    onMount(async () => {
        try{appVersion=await invoke<string>("get_app_version")}catch{}
        // load voices
        try {
            const voices = await invoke<VoiceConfig[]>("list_voices")
            availableVoices = voices.map(v => v.voice)
        } catch (err) {
            console.error("Failed to load voices:", err)
            availableVoices = []
        }

        try {
            // load microphones
            const mics = await invoke<string[]>("pv_get_audio_devices")
            availableMicrophones = [
                { label: t('settings-mic-default'), value: "-1" },  // system default
                ...mics.map((name, idx) => ({
                    label: name,
                    value: String(idx)
                }))
            ]

            // load vosk models
            const languageNames: Record<string, string> = {
                us: 'English',
                ru: 'Русский',
                uk: 'Українська',
                de: 'German',
                fr: 'French',
                es: 'Spanish',
                // ..
            };
            const voskModels = await invoke<{ name: string; language: string; size: string }[]>("list_vosk_models")
            availableVoskModels = voskModels.map(m => ({
                label: `${m.name} (${languageNames[m.language] ?? m.language}, ${m.size})`,
                value: m.name
            }))

            // load gliner models
            const glinerModels = await invoke<{ display_name: string; value: string }[]>("list_gliner_models")
            availableGlinerModels = glinerModels.map(m => ({
                label: m.display_name,
                value: m.value,
            }))

            // load settings from db
            const [mic, wakeWord, intentReco, slotEngine, glinerModel, voskModel,
                   noiseSuppression, vad, gainNormalizer, dialoguePersonality, savedTtsMode, savedMonitorSelf,
                   pico] = await Promise.all([
                invoke<string>("db_read", { key: "selected_microphone" }),
                invoke<string>("db_read", { key: "selected_wake_word_engine" }),
                invoke<string>("db_read", { key: "intent_backend" }),
                invoke<string>("db_read", { key: "slots_backend" }),
                invoke<string>("db_read", { key: "selected_gliner_model" }),
                invoke<string>("db_read", { key: "selected_vosk_model" }),

                invoke<string>("db_read", { key: "noise_suppression" }),
                invoke<string>("db_read", { key: "vad_backend" }),
                invoke<string>("db_read", { key: "gain_normalizer" }),
                invoke<string>("db_read", { key: "voice_dialogue_personality" }),
                invoke<string>("db_read", { key: "tts_mode" }),
                invoke<string>("db_read", { key: "monitor_self" }),

                invoke<string>("db_read", { key: "api_key__picovoice" })
            ])

            selectedMicrophone = mic
            selectedWakeWordEngine = wakeWord
            selectedIntentRecognitionEngine = intentReco
            selectedSlotExtractionEngine = slotEngine
            selectedVoskModel = voskModel
            selectedGlinerModel = glinerModel
            selectedNoiseSuppression = noiseSuppression
            selectedVad = vad
            gainNormalizerEnabled = gainNormalizer === "true"
            voiceDialoguePersonality = dialoguePersonality === "altron" ? "altron" : "jarvis"
            ttsMode = ["s2", "xtts", "silero"].includes(savedTtsMode) ? savedTtsMode : "xtts"
            monitorSelf = savedMonitorSelf === "true"
            audioOutputMode = await invoke<string>("db_read", { key: "audio_output_mode" }) === "voicemod" ? "voicemod" : "direct"
            apiKeyPicovoice = pico
            voiceVal = await invoke<string>("db_read", {key:"assistant_voice"})
            savedSnapshot = JSON.stringify({voiceVal,selectedMicrophone,selectedWakeWordEngine,selectedIntentRecognitionEngine,selectedSlotExtractionEngine,selectedGlinerModel,selectedVoskModel,selectedNoiseSuppression,selectedVad,gainNormalizerEnabled,voiceDialoguePersonality,ttsMode,audioOutputMode,monitorSelf,apiKeyPicovoice})
            loaded = true
        } catch (err) {
            loadError = "Не удалось загрузить настройки. Повторите открытие страницы."
            console.error("failed to load settings:", err)
        }
    })
</script>

<div class="settings-workspace">
    <header class="settings-heading"><div><p class="kicker">JARVIS / ПРЕДПОЧТЕНИЯ</p><h1>Настройки</h1><p class="intro">Ваш ассистент. Ваш привычный ритм.</p></div><span class="connection" class:connected={$ipcConnected}><i></i>{$ipcConnected?'Голосовой модуль подключён':'Голосовой модуль не подключён'}</span></header>
    <div class="settings-layout">
        <aside class="settings-sidebar"><nav aria-label="Разделы настроек">{#each ['Ассистент','Ваши данные','Ещё'] as group}<p class="nav-group">{group}</p>{#each sections.filter(section=>section.group===group) as section}<button type="button" class:chosen={active===section.id} aria-current={active===section.id?'page':undefined} aria-controls={'settings-'+section.id} on:click={()=>chooseSection(section.id)}><span class="nav-icon"><svelte:component this={section.icon} width="18" height="18"/></span><span>{section.title}</span>{#if dirty&&['general','voice','advanced'].includes(section.id)}<span class="unsaved-dot" aria-hidden="true" title="Несохранённые изменения"></span>{/if}</button>{/each}{/each}</nav><div class="sidebar-note"><span>JARVIS</span><small>Всё важное — на своём месте.</small></div></aside>
        <div class="settings-content">
            <header class="section-heading"><div class="section-icon"><svelte:component this={selectedSection.icon} width="24" height="24"/></div><div><h2>{selectedSection.title}</h2><p>{selectedSection.description}</p></div></header>
            {#if !loaded&&!loadError}<p class="load-state" role="status">Загружаем настройки…</p>{/if}
            {#if loadError}<div class="status-message error" role="alert">{loadError}<button type="button" on:click={()=>window.location.reload()}>Повторить</button></div>{/if}
            <section id="settings-general" hidden={active!=='general'} aria-label="Основные">
                <article class="settings-card"><div class="card-heading"><h3>Оформление</h3><span class="autosave">Сохраняется сразу</span></div><label class="field-label" for="app-appearance">Тема приложения</label><select id="app-appearance" bind:value={$appearance}><option value="dark">Тёмная</option><option value="light">Светлая</option><option value="system">Как в системе</option></select><p class="hint">Одна тема для всех разделов JARVIS.</p></article>
                <article class="settings-card"><div class="card-heading"><h3>Слушание и микрофон</h3><span class="autosave">Слушание — сразу</span></div>
        <Space h="sm" />
        <section class="global-listening" class:muted={$microphoneMuted} aria-label="Глобальное прослушивание микрофона">
            <div><p class="module-label">МИКРОФОН JARVIS</p><h3>{!$ipcConnected ? "Голосовой модуль не подключён" : !$microphoneMuteKnown ? "Проверяем состояние…" : $microphoneMuted ? "Прослушивание на паузе" : "JARVIS слушает"}</h3><p>Выключает распознавание слова «Джарвис» и голосовых команд. Текстовый чат продолжает работать.</p></div>
            <button type="button" on:click={() => setMicrophoneMuted(!$microphoneMuted)} disabled={!$ipcConnected || !$microphoneMuteKnown} aria-pressed={$microphoneMuted} aria-label={$microphoneMuted ? "Включить глобальное прослушивание микрофона" : "Остановить глобальное прослушивание микрофона"}>{$microphoneMuted ? "Включить" : "Приостановить"}</button>
        </section>
        <Space h="xl" />
        <ListeningShortcut />
        <Space h="xl" />
<fieldset disabled={!loaded||saveButtonDisabled}><div class="field-block">
        <Space h="sm" />
        <NativeSelect
            data={availableMicrophones}
            label={t('settings-microphone')}
            description={t('settings-microphone-desc')}
            variant="filled"
            bind:value={selectedMicrophone}
        />
    </div></fieldset></article>
            </section>
            <section id="settings-voice" hidden={active!=='voice'} aria-label="Голос и звук"><fieldset disabled={!loaded||saveButtonDisabled}><article class="settings-card">
        <section class="tts-mode" aria-labelledby="tts-mode-title">
            <p class="module-label">ОЗВУЧКА ОТВЕТОВ</p>
            <h3 id="tts-mode-title">Как будет говорить ассистент</h3>
            <p>Выбор для чата, диалога и других ответов, созданных на лету. Готовые звуки команд не меняются.</p>
            <div class="tts-mode-options">
                <button type="button" class:active={ttsMode === "s2"} on:click={() => ttsMode = "s2"} aria-pressed={ttsMode === "s2"}>
                    <strong>Fish Audio S2 Pro</strong><span>Выбранный реалистичный голос. Постоянный локальный сервер и кэш ускоряют повторные ответы.</span>
                </button>
                <button type="button" class:active={ttsMode === "xtts"} on:click={() => ttsMode = "xtts"} aria-pressed={ttsMode === "xtts"}>
                    <strong>Обученный голос XTTS</strong><span>Предыдущий сохранённый голос. Можно вернуться к нему без повторного обучения.</span>
                </button>
                <button type="button" class:active={ttsMode === "silero"} on:click={() => ttsMode = "silero"} aria-pressed={ttsMode === "silero"}>
                    <strong>Silero</strong><span>Локальная озвучка. Работает напрямую или через Voicemod — выбери вывод ниже.</span>
                </button>
            </div>
            <h3>Куда выводить голос JARVIS</h3>
            <div class="tts-mode-options">
                <button type="button" class:active={audioOutputMode === "direct"} aria-pressed={audioOutputMode === "direct"} on:click={()=>audioOutputMode="direct"}><strong>Напрямую в наушники</strong><span>Без Voicemod и виртуальных кабелей. Используются физические наушники или динамики.</span></button>
                <button type="button" class:active={audioOutputMode === "voicemod"} aria-pressed={audioOutputMode === "voicemod"} on:click={()=>audioOutputMode="voicemod"}><strong>Через Voicemod</strong><span>Вывод в CABLE Input. Требуются запущенный Voicemod и настроенный виртуальный кабель.</span></button>
            </div>
            <p>Выбор применяется после сохранения настроек к озвучке и готовым ответам команд. Настройки звука Windows не меняются.</p>
            <label class="monitor-self-option">
                <input type="checkbox" bind:checked={monitorSelf} />
                <span><strong>Слышать себя в наушниках</strong><small>Отдельное прослушивание обычного микрофона. Не влияет на звук Jarvis через VB-CABLE и Voicemod. Выключено по умолчанию.</small></span>
            </label>
        </section>
        <Space h="xl" />
        <div class="voice-select">
            <p class="voice-heading">{t('settings-voice')}</p>
            <p class="description">{t('settings-voice-desc')}</p>
            
            <div class="voice-options">
                {#each availableVoices as voice}
                    <button 
                        type="button"
                        class="voice-option"
                        class:selected={voiceVal === voice.id}
                        on:click={() => selectVoice(voice.id)}
                    >
                        <div class="voice-info">
                            <span class="voice-name">{voice.name}</span>
                            {#if voice.author}
                                <span class="voice-author">by {voice.author}</span>
                            {/if}
                        </div>
                        <div class="voice-languages">
                            {#each voice.languages as lang}
                                <img 
                                    src="/media/flags/{lang.toUpperCase()}.png" 
                                    alt={lang} 
                                    width="20" 
                                    title={lang}
                                />
                            {/each}
                        </div>
                    </button>
                {/each}
                
                {#if availableVoices.length === 0}
                    <p class="no-voices">{t('settings-no-voices')}</p>
                {/if}
            </div>
        </div>
    </article><article class="settings-card">        <section class="dialogue-personality" class:altron={voiceDialoguePersonality === "altron"} aria-labelledby="dialogue-personality-title">
            <p class="module-label">ГОЛОСОВОЙ ДИАЛОГ</p>
            <h3 id="dialogue-personality-title">Характер ответов</h3>
            <p>Используется только после команды «Джарвис, давай пообщаемся». Текстовый чат настраивается отдельно.</p>
            <div class="personality-options">
                <button type="button" class:active={voiceDialoguePersonality === "jarvis"} on:click={() => voiceDialoguePersonality = "jarvis"}>
                    <strong>JARVIS</strong><span>Дружелюбный оптимист-реалист: честно оценивает риски и помогает действовать.</span>
                </button>
                <button type="button" class:active={voiceDialoguePersonality === "altron"} on:click={() => voiceDialoguePersonality = "altron"}>
                    <strong>ALTRON</strong><span>Презирает слабости человечества, но холодно и реалистично помогает тебе.</span>
                </button>
            </div>
        </section>
        <Space h="xl" />
    </article></fieldset></section>
            <section id="settings-models" hidden={active!=='models'} aria-label="Нейросеть"><ChatProviderSettings /></section>
            <section id="settings-devices" hidden={active!=='devices'} aria-label="Устройства"><LanSync /></section>
            <section id="settings-data" hidden={active!=='data'} aria-label="Данные и копии"><CenterDataTransfer /><p class="hint">Беседы можно сохранить в разделе «Чат» через меню экспорта.</p></section>
            <section id="settings-automation" hidden={active!=='automation'} aria-label="Сайты и команды"><ScheduleWebsite /></section>
            <section id="settings-advanced" hidden={active!=='advanced'} aria-label="Дополнительно"><article class="settings-card"><h3>Распознавание и обработка речи</h3><p class="hint">Если ассистент слышит вас правильно, эти параметры можно оставить без изменений.</p><fieldset disabled={!loaded||saveButtonDisabled}>
        <Space h="sm" />
        <NativeSelect
            data={[
                { label: "Rustpotter", value: "Rustpotter" },
                { label: "Vosk", value: "Vosk" },
                { label: "Picovoice Porcupine", value: "Picovoice" }
            ]}
            label={t('settings-wake-word-engine')}
            description={t('settings-wake-word-desc')}
            variant="filled"
            bind:value={selectedWakeWordEngine}
        />

        {#if selectedWakeWordEngine === "Picovoice"}
            <Space h="sm" />
            <Alert title={t('settings-attention')} color="#868E96" variant="outline">
                <Notification
                    title={t('settings-picovoice-warning')}
                    icon={CrossCircled}
                    color="orange"
                    withCloseButton={false}
                >
                    {t('settings-picovoice-waiting')}
                </Notification>
                <Space h="sm" />
                <Text size="sm" color="gray">
                    {t('settings-picovoice-key-desc')}
                    <a href="https://console.picovoice.ai/" target="_blank">Picovoice Console</a>.
                </Text>
                <Space h="sm" />
                <Input
                    type="password" icon={Code}
                    placeholder={t('settings-picovoice-key')}
                    variant="filled"
                    autocomplete="off"
                    bind:value={apiKeyPicovoice}
                />
            </Alert>
        {/if}

        <Space h="xl" />
        {#key availableVoskModels}
        <NativeSelect
            data={[
                { label: t('settings-auto-detect'), value: "" },
                ...availableVoskModels
            ]}
            label={t('settings-vosk-model')}
            description={t('settings-vosk-model-desc')}
            variant="filled"
            bind:value={selectedVoskModel}
        />
        {/key}

        {#if availableVoskModels.length === 0}
            <Space h="sm" />
            <Alert title={t('settings-models-not-found')} color="orange" variant="outline">
                <Text size="sm" color="gray">
                    {t('settings-models-hint')}
                </Text>
            </Alert>
        {/if}

        <Space h="xl" />
        <NativeSelect
            data={[
                { label: "Intent Classifier", value: "IntentClassifier" },
                { label: "Embedding Classifier", value: "EmbeddingClassifier" }
            ]}
            label={t('settings-intent-engine')}
            description={t('settings-intent-engine-desc')}
            variant="filled"
            bind:value={selectedIntentRecognitionEngine}
        />

        <Space h="xl" />
        <NativeSelect
            data={[
                { label: t('settings-disabled'), value: "None" },
                { label: "GLiNER (NER)", value: "GLiNER" }
            ]}
            label={t('settings-slot-engine')}
            description={t('settings-slot-engine-desc')}
            variant="filled"
            bind:value={selectedSlotExtractionEngine}
        />

        {#if selectedSlotExtractionEngine === "GLiNER"}
            <Space h="sm" />
            {#key availableGlinerModels}
            <NativeSelect
                data={[
                    { label: t('settings-auto-detect'), value: "" },
                    ...availableGlinerModels
                ]}
                label={t('settings-gliner-model')}
                description={t('settings-gliner-model-desc')}
                variant="filled"
                bind:value={selectedGlinerModel}
            />
            {/key}

            {#if availableGlinerModels.length === 0}
                <Space h="sm" />
                <Alert title={t('settings-models-not-found')} color="orange" variant="outline">
                    <Text size="sm" color="gray">
                        {t('settings-gliner-models-hint')}
                    </Text>
                </Alert>
            {/if}
        {/if}

        <Space h="xl" />
        <NativeSelect
            data={[
                { label: t('settings-disabled'), value: "None" },
                { label: "Nnnoiseless", value: "Nnnoiseless" }
            ]}
            label={t('settings-noise-suppression')}
            description={t('settings-noise-suppression-desc')}
            variant="filled"
            bind:value={selectedNoiseSuppression}
        />

        <Space h="md" />

        <NativeSelect
            data={[
                { label: t('settings-disabled'), value: "None" },
                { label: "Energy", value: "Energy" },
                { label: "Nnnoiseless", value: "Nnnoiseless" }
            ]}
            label={t('settings-vad')}
            description={t('settings-vad-desc')}
            variant="filled"
            bind:value={selectedVad}
        />

        <Space h="md" />

        <InputWrapper label={t('settings-gain-normalizer')}>
            <Text size="sm" color="gray">
                {t('settings-gain-normalizer-desc')}
            </Text>
            <Space h="xs" />
            <Switch
                label={gainNormalizerEnabled ? t('settings-enabled') : t('settings-disabled')}
                bind:checked={gainNormalizerEnabled}
            />
        </InputWrapper>

        <Space h="xl" />

    </fieldset></article></section>
            <section id="settings-about" hidden={active!=='about'} aria-label="О приложении"><article class="settings-card about-card"><p class="kicker">ВАШ ЛИЧНЫЙ АССИСТЕНТ</p><h3>JARVIS</h3>{#if appVersion}<p class="hint">Версия {appVersion.split('-')[0]}</p>{/if}<p class="hint">Голосовое управление, общение и личные данные в одном месте.</p><div class="support-actions"><button type="button" on:click={()=>showInExplorer(logFilePath)} disabled={!logFilePath}>Открыть журнал приложения</button>{#if feedbackLink}<a href={feedbackLink} target="_blank" rel="noreferrer">Связаться с поддержкой ↗</a>{/if}</div><p class="hint">Журнал поможет разобраться, если что-то работает не так.</p></article><Footer showOriginal={true}/></section>
            {#if ['general','voice','advanced'].includes(active)}<div class="save-bar"><div aria-live="polite">{#if !loaded}<strong>{loadError?'Настройки не загружены':'Загружаем настройки…'}</strong>{:else if saveError}<span class="save-error" role="alert">{saveError}</span>{:else if settingsSaved&&!dirty}<strong class="saved">✓ Изменения сохранены</strong>{:else if dirty}<strong>Есть несохранённые изменения</strong><small>Примените их, когда всё настроите.</small>{:else}<strong>Все изменения сохранены</strong><small>Настройки готовы к работе.</small>{/if}</div><div class="save-actions"><button type="button" class="secondary" on:click={discard} disabled={!dirty||saveButtonDisabled}>Отменить</button><button type="button" class="primary" on:click={saveSettings} disabled={!loaded||!dirty||saveButtonDisabled}>{saveButtonDisabled?'Сохраняем…':'Сохранить изменения'}</button></div></div>{/if}
        </div>
    </div>
</div>

<style>
    .settings-workspace{--accent:var(--ui-accent,#a0e4ce);--line:var(--ui-line,#34474e);--muted:var(--ui-muted,#b4c2c7);--panel:var(--ui-panel,#172126);--input:var(--ui-input,#101a1f);font-family:'Manrope Variable',sans-serif;color:var(--ui-text,#edf5f5);max-width:1240px;margin:32px auto 64px;padding:0 28px;box-sizing:border-box}
    .settings-heading{display:flex;justify-content:space-between;align-items:center;gap:24px;margin-bottom:32px}.kicker{font-size:10px;font-weight:800;letter-spacing:.18em;color:var(--accent);margin:0 0 10px}h1{font-size:36px;font-weight:750;letter-spacing:-.045em;margin:0}.intro{font-size:14px;color:var(--muted);margin:10px 0 0}.connection{display:flex;align-items:center;gap:8px;border:1px solid var(--line);padding:10px 14px;border-radius:30px;font-size:11px;color:var(--muted);white-space:nowrap}.connection i{width:6px;height:6px;border-radius:50%;background:var(--muted)}.connection.connected i{background:var(--accent);box-shadow:0 0 10px color-mix(in srgb,var(--accent) 35%,transparent)}
    .settings-layout{display:grid;grid-template-columns:232px minmax(0,1fr);gap:36px;align-items:start}.settings-sidebar{position:sticky;top:24px;min-width:0}.nav-group{color:var(--muted);font-size:10px;font-weight:700;text-transform:uppercase;letter-spacing:.1em;margin:24px 12px 10px}.nav-group:first-child{margin-top:0}.settings-sidebar nav button{width:100%;display:flex;align-items:center;gap:11px;text-align:left;font-size:12px;font-weight:600;padding:12px;border:1px solid transparent;background:transparent;color:var(--muted);border-radius:11px;margin:4px 0;cursor:pointer;transition:background .16s,border-color .16s}.settings-sidebar nav button:hover{background:var(--input);color:var(--ui-text)}.settings-sidebar nav button.chosen{color:var(--accent);background:var(--ui-selected,#163b35);border-color:color-mix(in srgb,var(--accent) 26%,transparent)}.nav-icon{display:flex;flex:none}.unsaved-dot{margin-left:auto;width:5px;height:5px;flex:none;background:var(--accent);border-radius:50%}.sidebar-note{display:grid;gap:6px;margin:28px 12px;padding-top:22px;border-top:1px solid var(--line)}.sidebar-note>span{font-size:12px;font-weight:800;letter-spacing:.12em}.sidebar-note small{font-size:10px;color:var(--muted)}
    .settings-content{min-width:0}.section-heading{display:flex;align-items:center;gap:16px;padding:0 0 24px;margin-bottom:24px;border-bottom:1px solid var(--line)}.section-heading h2{font-size:23px;margin:0 0 5px;font-weight:700;letter-spacing:-.03em}.section-heading p{font-size:12px;color:var(--muted);margin:0;line-height:1.6}.section-icon{display:flex;align-items:center;justify-content:center;width:46px;height:46px;border-radius:14px;background:var(--ui-selected,#163b35);color:var(--accent);flex:none}.settings-content>section[hidden]{display:none!important}
    .settings-card{padding:24px;border:1px solid var(--line);border-radius:18px;background:var(--panel);margin-bottom:18px}.settings-card h3{font:700 15px/1.5 'Manrope Variable',sans-serif;margin:0 0 12px}.card-heading{display:flex;align-items:center;justify-content:space-between;gap:12px;flex-wrap:wrap;margin-bottom:18px}.card-heading h3{margin:0}.autosave{font-size:10px;color:var(--muted);border:1px solid var(--line);padding:5px 8px;border-radius:6px}.field-label{display:block;font-size:12px;margin-bottom:9px}.hint{color:var(--muted);font-size:12px;line-height:1.7;margin:12px 0 0}.field-block{border-top:1px solid var(--line);padding-top:20px}.settings-content fieldset{border:0;padding:0;margin:0;min-width:0}.settings-content fieldset:disabled{opacity:.55}
    .settings-workspace button,.settings-workspace select{font-family:inherit}.settings-workspace select{box-sizing:border-box;max-width:100%;min-height:44px;padding:10px 14px;background:var(--input);color:var(--ui-text);border:1px solid var(--line);border-radius:10px;font-size:13px;min-width:220px}.settings-workspace button:focus-visible,.settings-workspace select:focus-visible{outline:2px solid var(--accent);outline-offset:4px}.settings-workspace button:disabled{opacity:.45;cursor:default}.load-state{font-size:13px;color:var(--muted)}
    .settings-content :global(.global-listening),.settings-content :global(.shortcut-card){display:flex;justify-content:space-between;align-items:center;gap:20px}.settings-content :global(.global-listening),.settings-content :global(.shortcut-card),.settings-content :global(.tts-mode),.settings-content :global(.dialogue-personality){padding:0;border:0;background:none;border-radius:0;box-shadow:none}.settings-content :global(.global-listening h3),.settings-content :global(.tts-mode h3),.settings-content :global(.dialogue-personality h3){font:700 15px/1.5 'Manrope Variable',sans-serif;margin:0 0 10px;color:var(--ui-text)}.settings-content :global(.module-label),.settings-content :global(.eyebrow){display:none}.settings-content :global(p),.settings-content :global(small){line-height:1.65}.settings-content :global(.global-listening p),.settings-content :global(.shortcut-card p),.settings-content :global(.tts-mode p),.settings-content :global(.dialogue-personality p){font-size:12px;color:var(--muted)}
    .settings-content :global(.personality-options),.settings-content :global(.tts-mode-options){display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:12px;margin:16px 0 24px}.settings-content :global(.personality-options button),.settings-content :global(.tts-mode-options button){position:relative;padding:18px;text-align:left;min-height:106px;background:var(--input);border:1px solid var(--line);border-radius:12px;color:var(--ui-text);cursor:pointer}.settings-content :global(.personality-options button.active),.settings-content :global(.tts-mode-options button.active){border-color:var(--accent);background:var(--ui-selected,#163b35);box-shadow:inset 0 0 0 1px color-mix(in srgb,var(--accent) 15%,transparent)}.settings-content :global(.personality-options strong),.settings-content :global(.tts-mode-options strong){display:block;font-size:13px;font-weight:700}.settings-content :global(.personality-options span),.settings-content :global(.tts-mode-options span){display:block;font-size:11px;margin-top:8px;line-height:1.6;color:var(--muted)}.settings-content :global(.monitor-self-option){display:flex;align-items:flex-start;gap:12px;border:1px solid var(--line);border-radius:12px;padding:16px;margin:20px 0;background:var(--input);font-size:12px}.settings-content :global(.monitor-self-option small){display:block;color:var(--muted);font-size:11px;margin-top:5px}.settings-content :global(input[type=checkbox]){accent-color:var(--accent);width:16px;height:16px;flex:none}
    .settings-content :global(.voice-options){display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:12px;margin-top:16px;max-height:300px;overflow:auto}.settings-content :global(.voice-option){display:flex;align-items:center;justify-content:space-between;gap:12px;background:var(--input);color:var(--ui-text);border:1px solid var(--line);border-radius:12px;padding:16px;cursor:pointer;text-align:left}.settings-content :global(.voice-option.selected){border-color:var(--accent);background:var(--ui-selected)}.settings-content :global(.voice-info){display:grid;gap:4px}.settings-content :global(.voice-author){font-size:10px;color:var(--muted)}.settings-content :global(.voice-languages){display:flex;gap:4px}.settings-content :global(.voice-heading){font-size:14px;font-weight:700}.settings-content :global(.description),.settings-content :global(.no-voices){font-size:12px;color:var(--muted)}
    .settings-content :global(.provider-panel),.settings-content :global(.sync),.settings-content :global(.transfer),.settings-content :global(.website){--accent:var(--ui-accent);--border:var(--ui-line);padding:24px;background:var(--panel);border:1px solid var(--line);border-radius:18px;font-family:inherit;color:var(--ui-text)}.settings-content :global(.sync button),.settings-content :global(.transfer button),.settings-content :global(.provider-panel button),.settings-content :global(.website button),.settings-content :global(.global-listening button),.settings-content :global(.shortcut-card button){min-height:42px;padding:11px 16px;font:600 12px 'Manrope Variable',sans-serif;border-radius:10px;border:1px solid var(--line);background:var(--input);color:var(--ui-text);cursor:pointer}.settings-content :global(.sync input),.settings-content :global(.sync select),.settings-content :global(.sync textarea),.settings-content :global(.provider-panel input),.settings-content :global(.provider-panel select),.settings-content :global(.website input){border:1px solid var(--line);background:var(--input);color:var(--ui-text);border-radius:10px;min-height:42px}.settings-content :global(.sync .primary),.settings-content :global(.provider-panel button),.settings-content :global(.website button){background:var(--accent);color:var(--ui-bg);font-weight:750}.settings-content :global(.sync .guide){background:var(--input);border-color:var(--line)}.settings-content :global(.sync h2),.settings-content :global(.provider-panel h2),.settings-content :global(.transfer h2),.settings-content :global(.website h2){font-size:18px;letter-spacing:-.02em;margin:0 0 12px}.settings-content :global(.sync p),.settings-content :global(.provider-panel small){color:var(--muted)}
    .save-bar{position:sticky;bottom:12px;z-index:5;display:flex;align-items:center;justify-content:space-between;gap:20px;background:var(--panel);border:1px solid var(--line);box-shadow:0 8px 32px #0002;border-radius:14px;padding:16px 20px;margin-top:24px}.save-bar strong{display:block;font-size:11px;font-weight:650}.save-bar small{display:block;font-size:10px;color:var(--muted);margin-top:5px}.save-bar .saved{color:var(--accent)}.save-actions{display:flex;gap:10px;flex:none}.save-actions button,.support-actions button,.support-actions a,.status-message button{padding:12px 16px;border-radius:10px;font:650 11px 'Manrope Variable',sans-serif;cursor:pointer;min-height:42px;text-decoration:none}.primary{background:var(--accent);border:1px solid var(--accent);color:var(--ui-bg)}.secondary,.support-actions button,.support-actions a{background:var(--input);border:1px solid var(--line);color:var(--ui-text)}.support-actions{display:flex;gap:12px;flex-wrap:wrap;margin:24px 0}.about-card h3{font-size:26px;letter-spacing:-.04em}.status-message,.save-error{color:var(--ui-danger);font-size:12px}.status-message{padding:16px;border:1px solid var(--ui-danger);border-radius:12px;margin:16px 0}.status-message button{margin-left:12px}.settings-content :global(.svelteui-InputWrapper-root){margin-top:20px}
    .settings-content :global(.shortcut-card){display:grid;grid-template-columns:minmax(0,1fr) auto}.settings-content :global(.shortcut-card small){grid-column:1/-1}.settings-content :global(.shortcut-card .controls){margin:0;justify-content:flex-end}.settings-content :global(.shortcut-card .capture){min-width:140px}
    @media(max-width:960px){.settings-workspace{padding:0 20px}.settings-layout{grid-template-columns:190px minmax(0,1fr);gap:22px}.connection{font-size:10px}.settings-card{padding:20px}.save-bar{flex-wrap:wrap}.save-actions{margin-left:auto}.settings-content :global(.tts-mode-options){grid-template-columns:1fr}}
    @media(max-width:700px){.settings-workspace{margin-top:20px;padding:0 14px}.settings-heading{align-items:flex-start;flex-direction:column;gap:16px;margin-bottom:24px}h1{font-size:30px}.settings-layout{grid-template-columns:minmax(0,1fr);gap:24px}.settings-sidebar{position:static}.settings-sidebar nav{display:flex;overflow:auto;gap:8px;padding-bottom:8px}.nav-group,.sidebar-note{display:none}.settings-sidebar nav button{width:auto;flex:none;margin:0;padding:11px 13px}.settings-content :global(.personality-options),.settings-content :global(.voice-options){grid-template-columns:1fr}.settings-content :global(.global-listening),.settings-content :global(.shortcut-card){align-items:flex-start;flex-direction:column}.save-bar{bottom:8px;padding:14px}.settings-content :global(.provider-panel),.settings-content :global(.sync),.settings-content :global(.transfer),.settings-content :global(.website){padding:18px}}
    @media(prefers-reduced-motion:reduce){.settings-sidebar nav button{transition:none}}
</style>

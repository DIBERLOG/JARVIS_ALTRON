<script lang="ts">
    import { onMount } from "svelte"
    import { invoke } from "@tauri-apps/api/core"
    import { goto } from "@roxi/routify"
    import { setTimeout } from "worker-timers"

    import { showInExplorer } from "@/functions"
    import { appInfo, assistantVoice, translations, translate } from "@/stores"
    import { ipcConnected, microphoneMuted, microphoneMuteKnown, setMicrophoneMuted } from "@/lib/ipc"

    import HDivider from "@/components/elements/HDivider.svelte"
    import Footer from "@/components/Footer.svelte"
    import ListeningShortcut from "@/components/ListeningShortcut.svelte"
    import CenterDataTransfer from '@/components/CenterDataTransfer.svelte'
    import LanSync from '@/components/LanSync.svelte'
    import ChatProviderSettings from '@/components/ChatProviderSettings.svelte'
    import ScheduleWebsite from '@/components/ScheduleWebsite.svelte'

    import {
        Notification,
        Button,
        Text,
        Tabs,
        Space,
        Alert,
        Input,
        InputWrapper,
        NativeSelect,
        Switch
    } from "@svelteuidev/core"

    import {
        Check,
        Mix,
        Cube,
        Code,
        Gear,
        QuestionMarkCircled,
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
    assistantVoice.subscribe(value => {
        voiceVal = value
    })

    let feedbackLink = ""
    let logFilePath = ""
    appInfo.subscribe(info => {
        feedbackLink = info.feedbackLink
        logFilePath = info.logFilePath
    })

    // ### FUNCTIONS
    async function saveSettings() {
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
        } catch (err) {
            console.error("failed to load settings:", err)
        }
    })
</script>

<Space h="xl" />

<header class="settings-heading"><p>КОНТРОЛЬНЫЙ ЦЕНТР</p><h1>Настройки</h1><span>Слушание, голос, модели и перенос данных — каждый в своём разделе.</span></header>
<Notification
    title={t('settings-beta-title')}
    icon={QuestionMarkCircled}
    color="blue"
    withCloseButton={false}
>
    {t('settings-beta-desc')}<br />
    {t('settings-beta-feedback')} <a href={feedbackLink} target="_blank">{t('settings-beta-bot')}</a>.
    <Space h="sm" />
    <Button
        color="gray"
        radius="md"
        size="xs"
        uppercase
        on:click={() => showInExplorer(logFilePath)}
    >
        {t('settings-open-logs')}
    </Button>
</Notification>

<Space h="xl" />

{#if settingsSaved}
    <Notification
        title={t('notification-saved')}
        icon={Check}
        color="teal"
        on:close={() => { settingsSaved = false }}
    />
    <Space h="xl" />
{/if}
{#if saveError}
    <Notification title="Ошибка сохранения" color="red" on:close={() => { saveError = "" }}>
        {saveError}
    </Notification>
    <Space h="xl" />
{/if}

<Tabs class="form" color="#52fefe" position="left">
    <Tabs.Tab label="Слушание и диалог" icon={Gear}>
        <Space h="sm" />
        <section class="global-listening" class:muted={$microphoneMuted} aria-label="Глобальное прослушивание микрофона">
            <div><p class="module-label">МИКРОФОН JARVIS</p><h3>{$microphoneMuted ? "Прослушивание на паузе" : "JARVIS слушает"}</h3><p>Выключает распознавание слова «Джарвис» и голосовых команд. Текстовый чат продолжает работать.</p></div>
            <button type="button" on:click={() => setMicrophoneMuted(!$microphoneMuted)} disabled={!$ipcConnected || !$microphoneMuteKnown} aria-pressed={$microphoneMuted} aria-label={$microphoneMuted ? "Включить глобальное прослушивание микрофона" : "Остановить глобальное прослушивание микрофона"}>{$microphoneMuted ? "Не слушать" : "Слушать"}</button>
        </section>
        <Space h="xl" />
        <ListeningShortcut />
        <Space h="xl" />
        <section class="dialogue-personality" class:altron={voiceDialoguePersonality === "altron"} aria-labelledby="dialogue-personality-title">
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
    </Tabs.Tab>
    <Tabs.Tab label="Голос и звук" icon={Mix}>
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
    </Tabs.Tab>

    <Tabs.Tab label="Микрофон" icon={Mix}>
        <Space h="sm" />
        <NativeSelect
            data={availableMicrophones}
            label={t('settings-microphone')}
            description={t('settings-microphone-desc')}
            variant="filled"
            bind:value={selectedMicrophone}
        />
    </Tabs.Tab>

    <Tabs.Tab label="Распознавание речи" icon={Cube}>
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

        {#if selectedWakeWordEngine === "picovoice"}
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
                    icon={Code}
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

    </Tabs.Tab>
    <Tabs.Tab label="Модели и ответы" icon={Cube}>
        <ChatProviderSettings />
    </Tabs.Tab>
    <Tabs.Tab label="Данные и резервные копии" icon={Gear}>
        <CenterDataTransfer />
        <p class="settings-hint">История бесед экспортируется отдельно: «Чат → Экспорт текущего чата / Экспорт всех чатов».</p>
    </Tabs.Tab>
    <Tabs.Tab label="Синхронизация устройств" icon={Gear}>
        <LanSync />
    </Tabs.Tab>
    <Tabs.Tab label="Сайты и команды" icon={Gear}>
        <ScheduleWebsite />
    </Tabs.Tab>
</Tabs>

<Space h="xl" />

<p class="settings-hint">Кнопка ниже сохраняет слушание, голос и распознавание. В разделе «Модели и ответы» и при переносе данных используются отдельные кнопки.</p>
<Button
    color="lime"
    radius="md"
    size="sm"
    uppercase
    ripple
    fullSize
    on:click={saveSettings}
    disabled={saveButtonDisabled}
>
    {t('settings-save')}
</Button>

<Space h="sm" />

<Button
    color="gray"
    radius="md"
    size="sm"
    uppercase
    fullSize
    on:click={() => $goto("/")}
>
    {t('settings-back')}
</Button>

<HDivider />
<Footer showOriginal={true} />

<style lang="scss">
.settings-heading{font-family:'Manrope Variable',sans-serif;margin-bottom:1.4rem;color:#eaf8fa}.settings-heading p{font-size:.7rem;letter-spacing:.16em;color:#52fefe}.settings-heading h1{margin:.3rem 0;font-size:1.8rem}.settings-heading span,.settings-hint{color:#96b5bc;line-height:1.6;font-size:.85rem}
.dialogue-personality {
    --persona-line: rgba(82, 254, 254, .38);
    --persona-glow: rgba(82, 254, 254, .12);
    padding: 1rem;
    border: 1px solid var(--persona-line);
    border-left: 3px solid #52fefe;
    background: linear-gradient(110deg, var(--persona-glow), rgba(7, 12, 14, .36) 65%);

    &.altron { --persona-line: rgba(255, 105, 82, .45); --persona-glow: rgba(145, 38, 28, .17); border-left-color: #ff6952; }
    h3 { color: #edfafa; margin: .2rem 0; font: 700 1.1rem "Roboto Condensed", sans-serif; letter-spacing: .05em; }
    > p:not(.module-label) { color: rgba(222, 241, 243, .65); font-size: .78rem; margin: 0 0 .85rem; }
}

.module-label { margin: 0; color: #52fefe; font: 700 .65rem "Roboto Condensed", sans-serif; letter-spacing: .15em; }
.altron .module-label { color: #ff917f; }
.personality-options { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: .55rem; }
.personality-options button { text-align: left; padding: .75rem; background: rgba(8, 28, 31, .72); border: 1px solid rgba(126, 183, 188, .26); color: #c4d6d9; cursor: pointer; transition: border-color .18s ease, transform .18s ease, background .18s ease; }
.personality-options button:hover { transform: translateY(-1px); border-color: rgba(82, 254, 254, .55); }
.personality-options button.active { color: #fff; background: linear-gradient(135deg, rgba(6, 102, 110, .8), rgba(15, 38, 43, .85)); border-color: #52fefe; box-shadow: 0 0 18px rgba(82, 254, 254, .12); }
.dialogue-personality.altron .personality-options button.active:last-child { background: linear-gradient(135deg, rgba(126, 35, 25, .85), rgba(42, 14, 17, .88)); border-color: #ff6952; box-shadow: 0 0 18px rgba(255, 92, 72, .12); }
.tts-mode { padding: 1rem; border: 1px solid rgba(82, 254, 254, .34); background: linear-gradient(120deg, rgba(8, 65, 72, .26), rgba(7, 12, 14, .36)); }
.tts-mode h3 { color: #edfafa; margin: .2rem 0; font: 700 1.1rem "Roboto Condensed", sans-serif; letter-spacing: .05em; }
.tts-mode > p:not(.module-label) { color: rgba(222, 241, 243, .65); font-size: .78rem; margin: 0 0 .85rem; }
.tts-mode-options { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: .55rem; }
.tts-mode-options button { text-align: left; padding: .75rem; background: rgba(8, 28, 31, .72); border: 1px solid rgba(126, 183, 188, .26); color: #c4d6d9; cursor: pointer; transition: border-color .18s ease, background .18s ease; }
.tts-mode-options button:hover { border-color: rgba(82, 254, 254, .55); }
.tts-mode-options button.active { color: #fff; background: linear-gradient(135deg, rgba(6, 102, 110, .8), rgba(15, 38, 43, .85)); border-color: #52fefe; box-shadow: 0 0 18px rgba(82, 254, 254, .12); }
.tts-mode-options strong, .tts-mode-options span { display: block; }
.tts-mode-options strong { font: 700 .83rem "Roboto Condensed", sans-serif; letter-spacing: .04em; }
.tts-mode-options span { margin-top: .28rem; font-size: .7rem; line-height: 1.35; opacity: .75; }
.monitor-self-option { display: flex; align-items: flex-start; gap: .7rem; margin-top: .8rem; padding: .7rem; border: 1px solid rgba(126, 183, 188, .26); border-radius: 7px; background: rgba(8, 28, 31, .72); cursor: pointer; }
.monitor-self-option input { margin-top: .1rem; accent-color: #52fefe; width: 17px; height: 17px; flex: none; }
.monitor-self-option strong, .monitor-self-option small { display: block; }
.monitor-self-option small { color: rgba(222, 241, 243, .65); font-size: .7rem; line-height: 1.35; margin-top: .25rem; }
@media (max-width: 520px) { .tts-mode-options { grid-template-columns: 1fr; } }
.personality-options strong, .personality-options span { display: block; }
.personality-options strong { font: 700 .83rem "Roboto Condensed", sans-serif; letter-spacing: .08em; }
.personality-options span { margin-top: .28rem; font-size: .7rem; line-height: 1.35; opacity: .75; }
@media (max-width: 520px) { .personality-options { grid-template-columns: 1fr; } }

.voice-select {
    margin-bottom: 1rem;
    
    .voice-heading {
        font-weight: 600;
        font-size: 0.9rem;
        color: #fff;
        display: block;
        margin-bottom: 0.25rem;
    }
    
    .description {
        font-size: 0.75rem;
        color: rgba(255,255,255,0.5);
        margin: 0 0 0.75rem;
        white-space: pre-line;
    }
}

$voice-item-height: 70px;
$voice-item-gap: 0.5rem;
$voice-max-visible: 3;

.voice-options {
    display: flex;
    flex-direction: column;
    gap: $voice-item-gap;
    max-height: $voice-item-height * $voice-max-visible;
    overflow-y: auto;
    
    &::-webkit-scrollbar {
        width: 6px;
    }
    
    &::-webkit-scrollbar-track {
        background: rgba(255, 255, 255, 0.05);
        border-radius: 3px;
    }
    
    &::-webkit-scrollbar-thumb {
        background: rgba(255, 255, 255, 0.2);
        border-radius: 3px;
        
        &:hover {
            background: rgba(255, 255, 255, 0.3);
        }
    }
}

.voice-option {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 0.75rem 1rem;
    background: rgba(30, 40, 45, 0.8);
    border: 1px solid rgba(255,255,255,0.1);
    border-radius: 8px;
    cursor: pointer;
    transition: all 0.2s ease;
    text-align: left;
    width: 100%;
    
    &:hover {
        background: rgba(40, 55, 60, 0.9);
        border-color: rgba(255,255,255,0.2);
    }
    
    &.selected {
        background: rgba(82, 254, 254, 0.1);
        border-color: rgba(82, 254, 254, 0.4);
    }
}

.voice-info {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 0.15rem;
}

.voice-name {
    font-size: 0.85rem;
    color: #fff;
    font-weight: 500;
}

.voice-author {
    font-size: 0.7rem;
    color: rgba(255,255,255,0.4);
}

.voice-languages {
    display: flex;
    gap: 0.35rem;
    
    img {
        opacity: 0.8;
        border-radius: 2px;
    }
}

.no-voices {
    font-size: 0.8rem;
    color: rgba(255,255,255,0.4);
    font-style: italic;
}
.global-listening {display:flex;align-items:center;justify-content:space-between;gap:1rem;padding:1rem;border:1px solid rgba(82,254,254,.3);border-radius:9px;background:linear-gradient(110deg,#10252a,#0c1519)}
.global-listening.muted {border-color:rgba(255,134,134,.5);background:linear-gradient(110deg,#2b1b20,#101519)}
.global-listening h3 {margin:.2rem 0;color:#e8feff;font-size:1rem}
.global-listening p:not(.module-label) {margin:.2rem 0 0;color:#a8bdc2;font-size:.8rem;line-height:1.4}
.global-listening button {flex:none;padding:.5rem .7rem;border:1px solid #52fefe;border-radius:6px;background:#113b40;color:#d9ffff;font-size:.75rem;font-weight:700;cursor:pointer}
.global-listening.muted button {border-color:#ff9696;background:#532329;color:#ffe1e1}
.global-listening button:disabled {opacity:.5;cursor:not-allowed}
.global-listening button:focus-visible {outline:2px solid #fff;outline-offset:2px}
</style>

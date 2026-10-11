<script lang="ts">
    import { onMount, createEventDispatcher } from 'svelte'
    import { invoke } from '@tauri-apps/api/core'
    export let config = {provider:'local',local_model:'qwen3:8b',deepseek_model:'deepseek-flash',deepseek_configured:false,openai_model:'gpt-4.1-mini',openai_configured:false,speak_responses:true,personality:'jarvis'}
    const dispatch=createEventDispatcher()
    let keys={openai:'',deepseek:''},busy=false,ready=false,error='',notice=''
    onMount(async()=>{try{config=await invoke('chat_get_config');ready=true}catch(e){error=String(e)}})
    async function save(){
        if(busy||!ready)return
        busy=true;error='';notice=''
        try{
            const entries: [string,string][]=[['chat_provider',config.provider],['local_chat_model',config.local_model],['deepseek_chat_model',config.deepseek_model],['openai_chat_model',config.openai_model],['chat_speak_responses',String(config.speak_responses)],['assistant_personality',config.personality]]
            for(const provider of ['openai','deepseek'] as const)if(keys[provider].trim())entries.push([`api_key__${provider}`,keys[provider].trim()])
            await invoke('db_write_many',{entries:entries.map(([key,val])=>({key,val}))})
            config=await invoke('chat_get_config');keys={openai:'',deepseek:''};notice='Настройки моделей сохранены';dispatch('saved',config)
        }catch(e){error=String(e)}finally{busy=false}
    }
</script>

<section class="provider-panel" aria-label="Модели и ответы" aria-busy={busy}>
    <p class="eyebrow">ИНТЕЛЛЕКТ АССИСТЕНТА</p><h2>Модели и ответы</h2>
    <p class="muted">Источник общий для текстовых ответов и голосового разговора. Распознавание речи и озвучка настраиваются отдельно.</p>
    <fieldset disabled={busy||!ready}>
        <label>Источник ответов<select bind:value={config.provider}><option value="local">Ollama · локально</option><option value="deepseek">DeepSeek · API</option><option value="openai">OpenAI · API</option></select></label>
        {#if config.provider==='local'}
            <label>Модель Ollama<input bind:value={config.local_model} maxlength="200"/></label><small>Модель должна быть установлена в Ollama. API-ключ не нужен.</small>
        {:else if config.provider==='openai'}
            <label>Модель OpenAI<input bind:value={config.openai_model} maxlength="200" placeholder="gpt-4.1-mini"/></label>
            <label>API-ключ OpenAI<input type="password" autocomplete="off" bind:value={keys.openai} placeholder={config.openai_configured?'Ключ сохранён · пустое поле оставит его прежним':'Введите свой API-ключ'}/></label>
            <small>Запрос и контекст выбранного чата отправляются OpenAI. Нужны доступ к API и его баланс; подписка ChatGPT не заменяет ключ API. Ключ хранится в локальных настройках и не входит в экспорт чатов.</small>
        {:else}
            <label>Модель DeepSeek<input bind:value={config.deepseek_model} maxlength="200"/></label>
            <label>API-ключ DeepSeek<input type="password" autocomplete="off" bind:value={keys.deepseek} placeholder={config.deepseek_configured?'Ключ сохранён · пустое поле оставит его прежним':'Введите свой API-ключ'}/></label>
            <small>Запрос и контекст выбранного чата отправляются DeepSeek. Ключ не включается в экспорт.</small>
        {/if}
        <label>Личность текстового чата<select bind:value={config.personality}><option value="jarvis">JARVIS · спокойный</option><option value="altron">ALTRON · прямой</option></select></label>
        <label class="check"><input type="checkbox" bind:checked={config.speak_responses}/>Озвучивать ответы текстового чата</label>
        <button type="button" on:click={save}>{busy?'Сохранение…':'Сохранить настройки моделей'}</button>
    </fieldset>
    {#if error}<p role="alert" class="error">{error}</p>{/if}{#if notice}<p role="status">{notice}</p>{/if}
</section>
<style>
    .provider-panel{--accent:#52fefe;--border:#28474e;padding:1.2rem;border:1px solid var(--border);border-radius:12px;background:#0c191d;color:#eaf8fa;font-family:'Manrope Variable',sans-serif}.eyebrow{font-size:.67rem;letter-spacing:.15em;color:var(--accent)}h2{font-size:1.2rem;margin:.3rem 0}.muted,small{color:#96b5bc;line-height:1.6}fieldset{border:0;padding:0;display:grid;gap:.8rem;margin-top:1rem;min-width:0}label{display:grid;gap:.4rem;font-size:.85rem}input,select{padding:.7rem;border:1px solid var(--border);border-radius:7px;background:#071215;color:#eaf8fa;font:inherit;min-width:0}.check{display:flex;align-items:center}.check input{accent-color:var(--accent)}button{padding:.7rem 1rem;border:0;border-radius:7px;background:var(--accent);color:#062024;cursor:pointer;font-weight:700;width:fit-content}button:disabled,fieldset:disabled{opacity:.6}.error{color:#ffabab}small{font-size:.75rem}input:focus-visible,select:focus-visible,button:focus-visible{outline:2px solid var(--accent);outline-offset:2px}
</style>

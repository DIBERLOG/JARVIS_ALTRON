<script lang="ts">
    import { onMount } from "svelte"
    import { invoke } from "@tauri-apps/api/core"
    import { open } from "@tauri-apps/plugin-shell"
    import LoadingSpinner from "@/components/LoadingSpinner.svelte"
    import {recordRequest} from '@/lib/requestHistory'
    import ChatProviderSettings from '@/components/ChatProviderSettings.svelte'
    import {loadChatHistory,saveChatHistory,chatExport,type ChatSession,type ChatMessage} from '@/lib/chatHistory'
    import {save as saveFile} from '@tauri-apps/plugin-dialog'
    import {writeTextFile} from '@tauri-apps/plugin-fs'
    type Message = ChatMessage
    type NewsItem = { source: string, title: string, url: string, published_at: string }
    let config = { provider: "local", local_model: "qwen3:8b", deepseek_model: "deepseek-flash", deepseek_configured: false, openai_model:'gpt-4.1-mini',openai_configured:false,speak_responses: true, personality: "jarvis" }
    let prompt = "", loading = false, error = "", webSearch = false
    let chats:ChatSession[]=[],activeId='',historyReady=false,historyError='',exporting=false,exportNotice='',chatSearch=''
    $: filteredChats=chats.filter(c=>c.title.toLowerCase().includes(chatSearch.toLowerCase())).sort((a,b)=>b.updatedAt.localeCompare(a.updatedAt))
    function newChat(){if(loading)return;activeId='';messages=[];prompt='';error=''}
    function selectChat(chat:ChatSession){if(loading)return;activeId=chat.id;messages=[...chat.messages];prompt='';error=''}
    async function persist(){
        if(!historyReady)return
        if(!activeId)activeId=crypto.randomUUID()
        const existing=chats.find(c=>c.id===activeId)
        const entry:ChatSession={id:activeId,title:existing?.title||messages.find(m=>m.role==='user')?.content.slice(0,80)||'Новый чат',updatedAt:new Date().toISOString(),messages:[...messages]}
        chats=[entry,...chats.filter(c=>c.id!==activeId)]
        try{await saveChatHistory(chats);historyError=''}catch(e){historyError=String(e)}
    }
    async function exportChats(all:boolean){
        if(exporting||!historyReady)return
        exporting=true;exportNotice=''
        try{
            const selected=all?chats:chats.filter(c=>c.id===activeId)
            const file=await saveFile({defaultPath:`jarvis-chats-${new Date().toISOString().slice(0,10)}.json`,filters:[{name:'Чаты JARVIS',extensions:['json']}]})
            if(file){await writeTextFile(file,chatExport(selected));exportNotice='Чаты экспортированы. Файл содержит переписку — храните его безопасно.'}
        }catch(e){historyError=String(e)}finally{exporting=false}
    }
    let speaking = false, stoppingSpeech = false
    let messages: Message[] = []
    let news: NewsItem[] = [], newsLoading = false, newsError = "", newsUpdated = ""
    async function loadNews() {
        if (newsLoading) return
        newsLoading = true; newsError = ""
        try {
            news = await invoke<NewsItem[]>("chat_get_news")
            newsUpdated = new Date().toLocaleTimeString("ru-RU", { hour: "2-digit", minute: "2-digit" })
        } catch (e) { newsError = String(e) }
        finally { newsLoading = false }
    }
    function newsTime(value: string) {
        return new Date(value).toLocaleString("ru-RU", { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" })
    }
    async function openNews(url: string) {
        try { await open(url) } catch (e) { newsError = `Не удалось открыть ссылку: ${String(e)}` }
    }
    function sourceUrl(line: string) {
        return line.startsWith("Источник: ") ? line.slice(10).trim() : line.includes(" — https://") ? line.slice(line.indexOf("https://")).trim() : ""
    }
    $: visible = messages
    onMount(() => {
        invoke<typeof config>("chat_get_config").then(value => config = value).catch(e => error = String(e))
        loadChatHistory().then(value=>{chats=value;historyReady=true;const latest=[...chats].sort((a,b)=>b.updatedAt.localeCompare(a.updatedAt))[0];if(latest)selectChat(latest)}).catch(e=>historyError=`История не загружена: ${String(e)}. Отправка отключена, чтобы не затереть данные.`)
        loadNews()
        const refreshSpeech = () => invoke<boolean>("chat_is_speaking")
            .then(value => speaking = value).catch(() => speaking = false)
        refreshSpeech()
        const timer = window.setInterval(refreshSpeech, 350)
        return () => window.clearInterval(timer)
    })
    async function stopSpeech() {
        if (stoppingSpeech) return
        stoppingSpeech = true
        try {
            await invoke("chat_stop_speech")
            speaking = false
        } catch (e) { error = `Не удалось остановить озвучку: ${String(e)}` }
        finally { stoppingSpeech = false }
    }
    async function send() {
        const text = prompt.trim(); if (!text || loading || !historyReady) return
        if(text.length>12000){error='Сообщение слишком длинное: максимум 12 000 символов';return}
        recordRequest(text,'chat')
        error = ""; prompt = ""; messages = [...messages, { role: "user", content: text }]; loading = true
        try {
            await persist()
            const reply = await invoke<{content:string}>("chat_send", { clientMessages: messages.slice(-20), useWebSearch: webSearch }); messages = [...messages, { role: "assistant", content: reply.content }]
            await persist()
            speaking = await invoke<boolean>("chat_is_speaking")
        }
        catch (e) { error = String(e) } finally { loading = false }
    }
</script>

<section class="chat-shell">
    <header><p>НЕЙРОННЫЙ МОДУЛЬ</p><h1>Чат с Jarvis</h1><span>Ollama локально · DeepSeek · OpenAI. История сохраняется на этом компьютере.</span></header>
    <div class="chat-workspace">
    <aside class="chat-history" aria-label="История чатов">
        <p class="history-kicker">ВАШИ БЕСЕДЫ</p><button type="button" disabled={loading||!historyReady} on:click={newChat}>＋ Новый чат</button>
        <label>Поиск чатов<input bind:value={chatSearch} placeholder="Найти беседу…"/></label>
        <nav aria-label="Сохранённые чаты">{#each filteredChats as chat}<button type="button" class:chosen={activeId===chat.id} aria-current={activeId===chat.id?'true':undefined} disabled={loading} on:click={()=>selectChat(chat)}><strong>{chat.title}</strong><small>{new Date(chat.updatedAt).toLocaleDateString('ru-RU')} · {chat.messages.length} сообщений</small></button>{/each}</nav>
        {#if historyReady&&!chats.length}<small>Здесь появятся новые беседы.</small>{/if}
        <div class="history-export"><button type="button" on:click={()=>exportChats(false)} disabled={exporting||loading||!activeId}>↓ Экспорт текущего чата</button><button type="button" on:click={()=>exportChats(true)} disabled={exporting||loading||!chats.length}>↓ Экспорт всех чатов</button><small>JSON-файл без API-ключей. Личные сведения из сообщений остаются в переписке.</small></div>
    </aside>
    <div class="chat-main">
    {#if historyError}<p class="error" role="alert">{historyError}</p>{/if}{#if exportNotice}<p role="status">{exportNotice}</p>{/if}
    <section class="news-panel" aria-label="Лента новостей" aria-busy={newsLoading}>
        <div class="news-heading"><div><p class="news-kicker"><span class="live-dot"></span> WEB INTEL · ЖИВАЯ ЛЕНТА</p><h2>Актуальные новости</h2><small>{newsUpdated ? `Обновлено в ${newsUpdated}` : "Интерфакс · Лента.ру · BBC News · DW"}</small></div><button class="news-refresh" type="button" on:click={loadNews} disabled={newsLoading}>{#if newsLoading}<LoadingSpinner /><span>Обновление…</span>{:else}↻ Обновить{/if}</button></div>
        {#if newsError}<p class="news-error" role="alert">{newsError}</p>{/if}
        {#if news.length}<div class="news-grid">{#each news as item}<button class="news-card" type="button" on:click={() => openNews(item.url)} title="Открыть оригинал: {item.source}"><span class="news-meta"><b>{item.source}</b><time datetime={item.published_at}>{newsTime(item.published_at)}</time></span><strong>{item.title}</strong><span class="news-link">Читать источник ↗</span></button>{/each}</div>{:else if newsLoading}<div class="news-loading" role="status"><LoadingSpinner size={30}/><div><strong>Загружаю свежие новости</strong><span>Проверяю четыре источника…</span></div></div>{:else if !newsError}<p class="news-empty">Свежих публикаций пока нет.</p>{/if}
    </section>
    <details class="model-settings"><summary>Модель и настройки ответов · {config.provider==='openai'?'OpenAI':config.provider==='deepseek'?'DeepSeek':'Ollama'}</summary><ChatProviderSettings bind:config /></details>
    <div class="dialog">
        {#if speaking}<div class="speech-controls"><button class="stop-speech" type="button" on:click={stopSpeech} disabled={stoppingSpeech} aria-label="Остановить озвучку ответа"><span aria-hidden="true">■</span> Остановить звук</button></div>{/if}
        {#each visible as message}<article class:me={message.role === "user"}><b>{message.role === "user" ? "ТЫ" : "JARVIS"}</b>{#each message.content.split('\n') as line}{#if message.role === "assistant" && sourceUrl(line)}<button class="source-link" type="button" on:click={() => openNews(sourceUrl(line))}>{line}</button>{:else}<p>{line || '\u00a0'}</p>{/if}{/each}</article>{/each}{#if loading}<article><b>JARVIS</b><p>Думаю…</p></article>{/if}{#if error}<p class="error">{error}</p>{/if}
    </div>
    <form on:submit|preventDefault={send}><textarea aria-label="Сообщение ассистенту" maxlength="12000" bind:value={prompt} placeholder="Напиши вопрос ассистенту…" disabled={loading||!historyReady}></textarea><label class="web-search"><input type="checkbox" bind:checked={webSearch} /><span class="pulse"></span><span><b>WEB INTEL</b><small>Свежие новости и актуальные вопросы ищутся автоматически; включи для любого запроса</small></span></label><small>В ответах учитываются последние 20 сообщений выбранной беседы. Полная переписка остаётся в истории.</small><button disabled={loading||!historyReady}>Отправить</button></form>
    </div></div>
</section>

<style lang="scss">
.chat-shell{max-width:1250px!important;font-family:'Manrope Variable',sans-serif}.chat-workspace{display:grid;grid-template-columns:250px minmax(0,1fr);gap:1.2rem;margin-top:1.4rem}.chat-main{min-width:0}.chat-history{--history-accent:#52fefe;align-self:start;position:sticky;top:1rem;display:grid;gap:.9rem;padding:1rem;background:#0a191d;border:1px solid #25474e;border-radius:12px}.history-kicker{font-size:.66rem;letter-spacing:.14em;color:var(--history-accent);margin:0}.chat-history>button{width:100%}.chat-history label{display:grid;gap:.4rem;font-size:.75rem;color:#96b5bc}.chat-history input{width:100%;box-sizing:border-box}.chat-history nav{display:grid;gap:.4rem;max-height:440px;overflow:auto}.chat-history nav button{width:100%;background:#102126;border:1px solid transparent;color:#eaf8fa;text-align:left;font:inherit;padding:.65rem}.chat-history nav button.chosen{border-color:var(--history-accent);background:#15353b}.chat-history nav strong{display:block;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-size:.8rem}.chat-history nav small{display:block;margin-top:.3rem;font-size:.65rem}.history-export{border-top:1px solid #25474e;padding-top:.8rem;display:grid;gap:.5rem}.history-export button{font-size:.72rem;background:#17353b;color:#b6e7e8;width:100%}.history-export small{font-size:.67rem;line-height:1.5}.model-settings{margin-top:1rem}.model-settings summary{padding:.85rem 1rem;border:1px solid #28474e;border-radius:9px;background:#102126;cursor:pointer}.model-settings[open] summary{margin-bottom:.7rem}button:disabled{opacity:.5;cursor:not-allowed}@media(max-width:800px){.chat-workspace{grid-template-columns:1fr}.chat-history{position:static}.chat-history nav{max-height:180px}}
.chat-shell{max-width:940px;margin:2.5rem auto;color:#eaf8fa;padding:0 1.4rem}.chat-shell header p{color:#52fefe;letter-spacing:.16em;font-size:.75rem}.chat-shell h1{margin:.25rem 0}.chat-shell header span,small{color:#90a7ad}.settings,.dialog,form{background:#0d1417;border:1px solid #1c353a;border-radius:10px;padding:1rem;margin-top:1rem}.settings{display:grid;gap:.7rem}.settings label{display:grid;gap:.3rem;color:#b9d2d8}.persona-switch{display:grid;grid-template-columns:1fr 1fr;gap:.55rem}.persona-switch button{width:100%;text-align:left;background:#102126;border:1px solid #27454c;color:#b8d4d9}.persona-switch button span{display:block;font-size:.72rem;font-weight:400;opacity:.7;margin-top:.2rem}.persona-switch button.active{background:linear-gradient(135deg,#0a6e78,#132c37);border-color:#52fefe;color:#fff}.persona-switch button:last-child.active{background:linear-gradient(135deg,#7b2318,#2a1113);border-color:#ff735a}input,select,textarea{background:#071012;border:1px solid #315057;border-radius:6px;color:#ecffff;padding:.65rem;font:inherit}button{background:#16a8ae;color:#041011;border:0;border-radius:6px;padding:.65rem 1rem;font-weight:700;cursor:pointer;width:max-content}.dialog{min-height:260px;max-height:430px;overflow:auto}article{padding:.65rem .8rem;margin:.6rem 0;background:#101d20;border-left:3px solid #52fefe;border-radius:4px}article.me{border-left-color:#9b63ff}article p{white-space:pre-wrap;margin:.3rem 0 0}form{display:grid;gap:.7rem}textarea{min-height:90px;resize:vertical}.web-search{display:flex!important;align-items:center;gap:.65rem;padding:.65rem .8rem;border:1px solid #27535a;border-radius:7px;background:linear-gradient(90deg,#0b2025,#0d1417);cursor:pointer}.web-search input{accent-color:#52fefe}.web-search b{color:#52fefe;letter-spacing:.1em;font-size:.73rem}.web-search small{display:block}.pulse{height:.5rem;width:.5rem;border-radius:99px;background:#52fefe;box-shadow:0 0 12px #52fefe}.error{color:#ff8e8e}code{color:#52fefe}

.settings .voice-toggle{display:flex;align-items:center;gap:.75rem;padding:.7rem .85rem;border:1px solid #27535a;border-radius:7px;background:linear-gradient(90deg,#0b2025,#0d1417);cursor:pointer}
.voice-toggle input{position:absolute;width:1px;height:1px;opacity:0;padding:0}
.voice-check{display:grid;place-items:center;flex:none;width:20px;height:20px;border:1px solid #4e8088;border-radius:5px;background:#071418;color:#051517;font-size:14px;font-weight:800;line-height:1;transition:background .2s,border-color .2s,box-shadow .2s}
.voice-toggle input:checked + .voice-check{background:#52fefe;border-color:#52fefe;box-shadow:0 0 12px rgba(82,254,254,.3)}
.voice-toggle input:checked + .voice-check::after{content:'✓'}
.voice-toggle input:focus-visible + .voice-check{outline:2px solid #fff;outline-offset:3px}
.voice-copy{display:flex;flex-direction:column;gap:.15rem;text-align:left}
.voice-copy b{color:#52fefe;font-size:.73rem;letter-spacing:.1em}
.voice-copy small{font-size:.78rem}
.speech-controls{position:sticky;top:0;z-index:2;display:flex;justify-content:flex-end;padding:.15rem 0;background:linear-gradient(90deg,transparent,#0d1417 30%)}
.stop-speech{display:inline-flex;align-items:center;gap:.4rem;padding:.35rem .55rem;border:1px solid #b95d61;border-radius:6px;background:#30191c;color:#ffd8d8;font-size:.72rem;line-height:1.2;box-shadow:0 3px 12px #0005}
.stop-speech:hover{background:#492126;border-color:#ff8c91}
.stop-speech:focus-visible{outline:2px solid #fff;outline-offset:2px}
.stop-speech:disabled{opacity:.55;cursor:wait}
.news-panel{--news-cyan:#52fefe;--news-border:#23444a;margin-top:1.2rem;padding:1.05rem;border:1px solid var(--news-border);border-radius:12px;background:radial-gradient(ellipse at 90% 0%,#123239 0%,transparent 48%),#0b1519;box-shadow:0 15px 35px #0002}
.news-heading{display:flex;align-items:center;justify-content:space-between;gap:1rem;margin-bottom:.85rem}.news-heading h2{margin:.18rem 0;font-size:1.15rem;letter-spacing:.02em}.news-heading small{font-size:.73rem}.news-kicker{display:flex;align-items:center;gap:.5rem;margin:0;color:var(--news-cyan);font-size:.67rem;font-weight:800;letter-spacing:.13em}.live-dot{width:.45rem;height:.45rem;border-radius:50%;background:var(--news-cyan);box-shadow:0 0 10px var(--news-cyan)}.news-refresh{flex:none;padding:.45rem .7rem;border:1px solid #41787e;background:#143139;color:#dcffff;font-size:.72rem}.news-refresh:hover:not(:disabled){background:#1c484f}.news-refresh:disabled{opacity:.55;cursor:wait}
.news-grid{display:grid;grid-auto-flow:column;grid-auto-columns:minmax(220px,34%);gap:.65rem;overflow-x:auto;padding:.15rem .1rem .55rem;scroll-snap-type:x mandatory;scrollbar-color:#32676c transparent}.news-card{display:flex;flex-direction:column;align-items:stretch;min-height:150px;width:auto;padding:.8rem;border:1px solid #25444a;border-radius:9px;background:linear-gradient(150deg,#14252a,#0d1b1f);color:#edfbfd;text-align:left;scroll-snap-align:start;transition:transform .18s,border-color .18s,background .18s}.news-card:hover{transform:translateY(-3px);border-color:var(--news-cyan);background:#173037}.news-card:focus-visible,.news-refresh:focus-visible{outline:2px solid #fff;outline-offset:2px}.news-meta{display:flex;align-items:center;justify-content:space-between;gap:.4rem;margin-bottom:.65rem;font-size:.66rem}.news-meta b{color:var(--news-cyan);text-transform:uppercase;letter-spacing:.08em}.news-meta time{color:#a0b8bd;white-space:nowrap}.news-card strong{display:-webkit-box;overflow:hidden;-webkit-line-clamp:3;-webkit-box-orient:vertical;font-size:.84rem;line-height:1.35;font-weight:600}.news-link{margin-top:auto;padding-top:.75rem;color:#8dcdd0;font-size:.68rem}.news-error{margin:.5rem 0;color:#ffaaaa;font-size:.78rem}.news-empty{margin:.6rem 0;color:#a6bfc3;font-size:.8rem}@media(max-width:640px){.news-grid{grid-auto-columns:minmax(230px,80%)}.news-heading h2{font-size:1rem}}
.source-link{display:block;width:100%;margin:.25rem 0;padding:.3rem .4rem;border:1px solid #285057;background:#10262b;color:#8ee5e8;text-align:left;overflow-wrap:anywhere;font-size:.75rem;font-weight:500}.source-link:hover{border-color:#52fefe;background:#173840}.source-link:focus-visible{outline:2px solid #fff;outline-offset:2px}
    .news-refresh{display:inline-flex;align-items:center;justify-content:center;gap:.5rem;min-width:112px}
    .news-loading{display:flex;align-items:center;justify-content:center;gap:1rem;min-height:150px;border:1px solid #23444a;border-radius:9px;background:linear-gradient(135deg,#13313955,#0d1b1f);color:#d6f8f6}
    .news-loading>div{display:flex;flex-direction:column;gap:.35rem}.news-loading strong{font-size:.8rem;font-weight:600}.news-loading span{color:#89adb3;font-size:.7rem}
</style>

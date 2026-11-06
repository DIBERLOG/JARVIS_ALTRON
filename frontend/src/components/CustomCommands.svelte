<script lang="ts">
    import {onMount} from 'svelte'
    import {invoke} from '@tauri-apps/api/core'

    type Kind = 'app' | 'site'
    type Command = {id:string, phrases:string[], kind:Kind, target:string, enabled:boolean}

    let commands:Command[] = []
    let ready = false, busy = false, error = '', notice = ''
    let editing:string|null = null
    let phrases = '', kind:Kind = 'site', target = ''

    onMount(async()=>{
        try{
            const raw = await invoke<string>('db_read',{key:'custom_commands_v1'})
            commands = raw ? JSON.parse(raw) : []
            ready = true
        }catch(e){error = 'Не удалось загрузить свои команды: '+String(e)}
    })

    const splitPhrases = (text:string) => text.split(/[,;\n]/).map(p=>p.trim()).filter(Boolean)

    async function persist(next:Command[], message:string){
        busy = true; error = ''; notice = ''
        try{
            await invoke('db_write_many',{entries:[{key:'custom_commands_v1',val:JSON.stringify(next)}]})
            commands = next; notice = message
            return true
        }catch(e){error = String(e); return false}
        finally{busy = false}
    }

    function validate():string|null{
        const list = splitPhrases(phrases)
        if(!list.length) return 'Напишите, что вы будете говорить, например: «открой ютуб».'
        if(list.length>8) return 'Не больше 8 вариантов фразы.'
        if(!target.trim()) return kind==='site' ? 'Вставьте адрес сайта.' : 'Выберите программу или ярлык.'
        if(kind==='site'){
            try{const url = new URL(target.trim()); if(!['http:','https:'].includes(url.protocol)||url.username||url.password) throw 0}
            catch{return 'Адрес сайта должен начинаться с https:// или http://'}
        }
        const taken = commands.filter(c=>c.id!==editing).flatMap(c=>c.phrases.map(p=>p.toLowerCase()))
        const clash = list.find(p=>taken.includes(p.toLowerCase()))
        return clash ? `Фраза «${clash}» уже занята другой командой.` : null
    }

    async function submit(){
        if(!ready||busy) return
        if(kind==='site' && target.trim() && !/^[a-z]+:\/\//i.test(target.trim())) target = 'https://'+target.trim()
        const problem = validate(); if(problem){error = problem; notice = ''; return}
        const item:Command = {id:editing ?? crypto.randomUUID(), phrases:splitPhrases(phrases), kind, target:target.trim(), enabled:true}
        const next = editing ? commands.map(c=>c.id===editing ? {...item, enabled:c.enabled} : c) : [...commands, item]
        if(await persist(next, editing ? 'Команда обновлена' : `Готово. Скажите: «Джарвис, ${item.phrases[0]}»`)) reset()
    }

    function edit(command:Command){
        editing = command.id; phrases = command.phrases.join(', '); kind = command.kind; target = command.target
        error = ''; notice = ''
    }
    function reset(){editing = null; phrases = ''; target = ''; kind = 'site'}

    const remove = (command:Command) => persist(commands.filter(c=>c.id!==command.id), 'Команда удалена').then(()=>{if(editing===command.id) reset()})
    const toggle = (command:Command) => persist(commands.map(c=>c.id===command.id ? {...c, enabled:!c.enabled} : c), command.enabled ? 'Команда выключена' : 'Команда включена')

    async function browse(){
        try{
            const {open} = await import('@tauri-apps/plugin-dialog')
            const path = await open({multiple:false, directory:false, title:'Программа или ярлык', filters:[{name:'Программы и ярлыки', extensions:['exe','lnk','url','bat','cmd']}, {name:'Все файлы', extensions:['*']}]})
            if(typeof path==='string'){
                target = path
                if(!phrases.trim()){const name = path.split(/[\\/]/).pop()?.replace(/\.[^.]+$/,'') ?? ''; if(name) phrases = 'открой '+name.toLowerCase()}
            }
        }catch(e){error = 'Не удалось открыть выбор файла: '+String(e)}
    }

    const short = (text:string) => text.length>60 ? '…'+text.slice(-58) : text
</script>

<section class="custom" aria-label="Свои команды">
    <p class="eyebrow">СВОИ КОМАНДЫ</p>
    <h2>Открыть приложение или сайт голосом</h2>
    <p>Придумайте фразу — Джарвис по ней запустит программу или откроет сайт. Можно сказать фразу целиком или только название: «ютуб» сработает и как «открой ютуб».</p>

    <form on:submit|preventDefault={submit}>
        <div class="kinds" role="radiogroup" aria-label="Что открывать">
            <button type="button" role="radio" aria-checked={kind==='site'} class:active={kind==='site'} on:click={()=>{kind='site'; target=''}} disabled={busy}>Сайт</button>
            <button type="button" role="radio" aria-checked={kind==='app'} class:active={kind==='app'} on:click={()=>{kind='app'; target=''}} disabled={busy}>Приложение</button>
        </div>
        <label>Что вы скажете
            <input bind:value={phrases} maxlength="400" placeholder={kind==='site' ? 'открой ютуб, включи ютуб' : 'открой телеграм, запусти телегу'} disabled={!ready||busy}/>
            <small>Несколько вариантов — через запятую.</small>
        </label>
        {#if kind==='site'}
            <label>Адрес сайта
                <input type="url" bind:value={target} maxlength="2000" placeholder="https://youtube.com" disabled={!ready||busy}/>
            </label>
        {:else}
            <label>Программа или ярлык
                <div class="pick">
                    <input bind:value={target} maxlength="1000" placeholder="C:\Program Files\…\app.exe" disabled={!ready||busy}/>
                    <button type="button" class="ghost" on:click={browse} disabled={!ready||busy}>Выбрать…</button>
                </div>
            </label>
        {/if}
        <div class="actions">
            <button disabled={!ready||busy}>{busy ? 'Сохранение…' : editing ? 'Сохранить изменения' : 'Добавить команду'}</button>
            {#if editing}<button type="button" class="ghost" on:click={reset} disabled={busy}>Отмена</button>{/if}
        </div>
    </form>

    {#if error}<p class="error" role="alert">{error}</p>{/if}
    {#if notice}<p class="notice" role="status">{notice}</p>{/if}

    {#if commands.length}
        <ul>
            {#each commands as command (command.id)}
                <li class:off={!command.enabled} class:editing={editing===command.id}>
                    <span class="badge">{command.kind==='site' ? 'Сайт' : 'Приложение'}</span>
                    <div class="body">
                        <strong>«{command.phrases.join('», «')}»</strong>
                        <small title={command.target}>{short(command.target)}</small>
                    </div>
                    <div class="row-actions">
                        <button type="button" class="ghost" on:click={()=>toggle(command)} disabled={busy} title={command.enabled ? 'Выключить' : 'Включить'}>{command.enabled ? 'Вкл' : 'Выкл'}</button>
                        <button type="button" class="ghost" on:click={()=>edit(command)} disabled={busy}>Изменить</button>
                        <button type="button" class="ghost danger" on:click={()=>remove(command)} disabled={busy} aria-label="Удалить команду">Удалить</button>
                    </div>
                </li>
            {/each}
        </ul>
    {:else if ready}
        <p class="empty">Пока нет своих команд. Добавьте первую — например, сайт расписания или любимую игру.</p>
    {/if}
</section>

<style>
    .custom{--accent:#52fefe;--border:#28474e;font-family:'Manrope Variable',sans-serif;padding:1.2rem;border:1px solid var(--border);border-radius:12px;background:#0c191d;color:#eaf8fa;margin-bottom:1rem}
    .eyebrow{color:var(--accent);font-size:.67rem;letter-spacing:.15em;margin:0}
    h2{font-size:1.2rem;margin:.3rem 0}
    p{line-height:1.6;color:#96b5bc}
    form{display:grid;gap:.8rem;margin:1rem 0}
    label{display:grid;gap:.45rem;font-size:.85rem}
    small{color:#7f9ea5;font-size:.74rem}
    input{padding:.7rem;border:1px solid var(--border);border-radius:7px;background:#071215;color:#eaf8fa;font:inherit;min-width:0}
    button{padding:.65rem 1rem;background:var(--accent);border:0;border-radius:7px;color:#062024;cursor:pointer;font:inherit;font-weight:700;width:fit-content;transition:background .2s,border-color .2s,opacity .2s}
    button:disabled{opacity:.5;cursor:default}
    .ghost{background:transparent;border:1px solid var(--border);color:#cfe9ec;font-weight:600}
    .ghost:hover:not(:disabled){border-color:#3f7c84}
    .danger:hover:not(:disabled){border-color:#a85454;color:#ffb4b4}
    .kinds{display:flex;gap:.4rem}
    .kinds button{background:transparent;border:1px solid var(--border);color:#96b5bc}
    .kinds button.active{background:rgba(82,254,254,.12);border-color:var(--accent);color:#eaf8fa}
    .pick{display:flex;gap:.5rem}.pick input{flex:1}
    .actions{display:flex;gap:.5rem}
    ul{list-style:none;margin:1rem 0 0;padding:0;display:grid;gap:.5rem}
    li{display:flex;align-items:center;gap:.8rem;padding:.7rem .8rem;border:1px solid var(--border);border-radius:9px;background:#091417;transition:opacity .2s,border-color .2s}
    li.off{opacity:.5}
    li.editing{border-color:var(--accent)}
    .badge{flex:none;font-size:.66rem;padding:.2rem .5rem;border-radius:99px;background:rgba(82,254,254,.1);color:var(--accent)}
    .body{display:grid;gap:.15rem;min-width:0;flex:1}
    .body strong{font-size:.85rem;font-weight:600;overflow-wrap:anywhere}
    .body small{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
    .row-actions{display:flex;gap:.35rem;flex:none}
    .row-actions button{padding:.4rem .65rem;font-size:.75rem}
    .error{color:#ffabab}
    .notice{color:#7fe7c9}
    .empty{font-size:.85rem;margin:0}
    input:focus-visible,button:focus-visible{outline:2px solid var(--accent);outline-offset:2px}
    @media (max-width:640px){li{flex-wrap:wrap}.row-actions{width:100%}}
</style>

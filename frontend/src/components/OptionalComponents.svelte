<script lang="ts">
    import {onMount, onDestroy} from 'svelte'
    import {invoke} from '@tauri-apps/api/core'

    type Item = {state:string, detail:string, done:number, total:number}
    type Status = {running:boolean, s2:Item, ollama:Item}

    let status:Status|null = null
    let error = ''
    let timer:ReturnType<typeof setInterval>|undefined

    async function refresh(){
        try{status = await invoke<Status>('components_status'); error = ''}
        catch(e){error = 'Не удалось узнать состояние загрузок: '+String(e)}
    }
    async function retry(){
        try{await invoke('components_start'); await refresh()}
        catch(e){error = String(e)}
    }
    onMount(()=>{refresh(); timer = setInterval(refresh, 1000)})
    onDestroy(()=>clearInterval(timer))

    const LABELS:Record<string,string> = {waiting:'Ожидает', checking:'Проверка', downloading:'Загрузка', installing:'Установка', ready:'Готово', unsupported:'Недоступно', error:'Ошибка'}
    const gb = (bytes:number) => (bytes/1073741824).toLocaleString('ru-RU',{maximumFractionDigits:1})
    const percent = (item:Item) => item.total ? Math.min(100, Math.round(item.done/item.total*100)) : 0
    $: rows = status ? [
        {title:'Голос Fish S2', note:'Голос Джарвиса на видеокарте. До окончания загрузки говорит системный голос.', item:status.s2},
        {title:'Локальный ИИ (Ollama)', note:'Чат, перевод и обработка текста без API-ключа.', item:status.ollama},
    ] : []
    $: failed = rows.some(row=>row.item.state==='error')
</script>

<article class="components" aria-labelledby="components-title">
    <p class="eyebrow">ДОПОЛНИТЕЛЬНЫЕ КОМПОНЕНТЫ</p>
    <h3 id="components-title">Загрузка голоса и локального ИИ</h3>
    <p>Скачиваются один раз в фоне. Если связь оборвётся, загрузка продолжится с того же места.</p>
    {#if error}<p class="error">{error}</p>{/if}
    <ul>
        {#each rows as row}
            <li class={row.item.state}>
                <div class="head"><strong>{row.title}</strong><span class="badge">{LABELS[row.item.state] ?? row.item.state}</span></div>
                <small>{row.item.detail || row.note}</small>
                {#if row.item.state==='downloading' && row.item.total}
                    <div class="bar" role="progressbar" aria-valuemin="0" aria-valuemax="100" aria-valuenow={percent(row.item)}><span style="width:{percent(row.item)}%"></span></div>
                    <small>{gb(row.item.done)} из {gb(row.item.total)} ГБ · {percent(row.item)}%</small>
                {/if}
            </li>
        {/each}
    </ul>
    {#if failed && !status?.running}<button type="button" on:click={retry}>Повторить</button>{/if}
    <p class="license">Модель голоса Fish Audio S2 Pro распространяется по некоммерческой исследовательской лицензии, файлы скачиваются с Hugging Face и GitHub. Ollama — с ollama.com.</p>
</article>

<style>
    .components{--accent:#52fefe;--border:#28474e;font-family:'Manrope Variable',sans-serif;padding:1.2rem;border:1px solid var(--border);border-radius:12px;background:#0c191d;color:#eaf8fa;margin-bottom:1rem}
    .eyebrow{color:var(--accent);font-size:.67rem;letter-spacing:.15em;margin:0}
    h3{font-size:1.1rem;margin:.3rem 0}
    p{line-height:1.6;color:#96b5bc;margin:.3rem 0}
    ul{list-style:none;margin:.8rem 0;padding:0;display:grid;gap:.5rem}
    li{display:grid;gap:.35rem;padding:.75rem .85rem;border:1px solid var(--border);border-radius:9px;background:#091417}
    .head{display:flex;justify-content:space-between;align-items:center;gap:.6rem}
    strong{font-size:.88rem;font-weight:600}
    small{color:#7f9ea5;font-size:.76rem;line-height:1.45}
    .badge{flex:none;font-size:.66rem;padding:.2rem .55rem;border-radius:99px;background:rgba(82,254,254,.1);color:var(--accent)}
    li.ready .badge{background:rgba(127,231,201,.12);color:#7fe7c9}
    li.error .badge{background:rgba(255,171,171,.12);color:#ffabab}
    li.unsupported .badge{background:rgba(150,181,188,.12);color:#96b5bc}
    .bar{height:6px;border-radius:99px;background:#16292e;overflow:hidden}
    .bar span{display:block;height:100%;background:var(--accent);transition:width .4s ease}
    button{padding:.6rem 1rem;background:var(--accent);border:0;border-radius:7px;color:#062024;cursor:pointer;font:inherit;font-weight:700}
    .error{color:#ffabab}
    .license{font-size:.72rem;color:#6f8d94}
</style>

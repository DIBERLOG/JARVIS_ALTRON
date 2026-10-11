<script lang="ts">
    import {onMount,createEventDispatcher} from 'svelte'
    import {invoke} from '@tauri-apps/api/core'
    const dispatch=createEventDispatcher()
    let url='',ready=false,busy=false,error='',notice=''
    onMount(async()=>{try{url=await invoke<string>('db_read',{key:'schedule_website'});ready=true}catch(e){error=String(e)}})
    async function save(){
        if(!ready||busy)return
        busy=true;error='';notice=''
        try{
            const raw=url.trim()
            if(raw){const parsed=new URL(raw);if(!['http:','https:'].includes(parsed.protocol)||!parsed.hostname||parsed.username||parsed.password)throw new Error('Введите ссылку http/https без логина и пароля в адресе')}
            await invoke('db_write_many',{entries:[{key:'schedule_website',val:raw}]})
            url=raw;notice=raw?'Ссылка на расписание сохранена':'Ссылка удалена';dispatch('saved',raw)
        }catch(e){error=String(e)}finally{busy=false}
    }
</script>
<section class="website" aria-label="Сайт расписания">
    <p class="eyebrow">ВАША КОМАНДА</p><h2>Сайт расписания</h2><p>Вставьте ссылку на учебное, рабочее или любое другое расписание. Команда «Джарвис, открой расписание» откроет её в браузере.</p>
    <form on:submit|preventDefault={save}><label for="schedule-link">Ссылка на сайт<input id="schedule-link" type="url" maxlength="4096" bind:value={url} placeholder="https://example.com/schedule" disabled={!ready||busy}/></label><button disabled={!ready||busy}>{busy?'Сохранение…':'Сохранить ссылку'}</button></form>
    <small>Только http/https. Пустое поле удаляет сохранённую ссылку. Это отдельная команда от календаря и «расписания на сегодня» в Центре.</small>
    {#if error}<p class="error" role="alert">{error}</p>{/if}{#if notice}<p role="status">{notice}</p>{/if}
</section>
<style>.website{--accent:#52fefe;--border:#28474e;font-family:'Manrope Variable',sans-serif;padding:1.2rem;border:1px solid var(--border);border-radius:12px;background:#0c191d;color:#eaf8fa}.eyebrow{color:var(--accent);font-size:.67rem;letter-spacing:.15em}h2{font-size:1.2rem;margin:.3rem 0}p,small{line-height:1.6}p:not(.eyebrow),small{color:#96b5bc}form,label{display:grid;gap:.65rem}form{margin:1rem 0}label{font-size:.85rem}input{padding:.75rem;border:1px solid var(--border);border-radius:7px;background:#071215;color:#eaf8fa;font:inherit;min-width:0}button{padding:.7rem 1rem;background:var(--accent);border:0;border-radius:7px;color:#062024;cursor:pointer;font-weight:700;width:fit-content}button:disabled{opacity:.5}small{font-size:.75rem}.error{color:#ffabab!important}input:focus-visible,button:focus-visible{outline:2px solid var(--accent);outline-offset:2px}</style>

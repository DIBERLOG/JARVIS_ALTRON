<script lang="ts">
    import {onMount,tick} from 'svelte'
    import {invoke} from '@tauri-apps/api/core'
    import {open} from '@tauri-apps/plugin-shell'
    import ScheduleWebsite from './ScheduleWebsite.svelte'
    let dialog:HTMLDialogElement,visible=false,error='',openAfterSave=false,busy=false
    async function show(event:Event){
        if(busy)return
        busy=true;error='';openAfterSave=!(event as CustomEvent).detail?.configure
        try{
            const url=await invoke<string>('db_read',{key:'schedule_website'})
            if(url&&openAfterSave){const parsed=new URL(url);if(!['http:','https:'].includes(parsed.protocol)||parsed.username||parsed.password)throw new Error('Недопустимая сохранённая ссылка');await open(url);return}
            visible=true;await tick();if(!dialog.open)dialog.showModal()
            invoke('animate_window_in',{reducedMotion:true}).catch(()=>{})
        }catch(e){error=String(e);visible=true;await tick();if(!dialog.open)dialog.showModal()}finally{busy=false}
    }
    async function saved(event:CustomEvent<string>){
        error=''
        try{if(openAfterSave&&event.detail)await open(event.detail);dialog.close();visible=false}catch(e){error=`Ссылка сохранена, но браузер не открылся: ${String(e)}`}
    }
    onMount(()=>{window.addEventListener('jarvis-schedule-open',show);return()=>window.removeEventListener('jarvis-schedule-open',show)})
</script>
<dialog bind:this={dialog} aria-label="Настроить сайт расписания" on:close={()=>visible=false}>
    {#if visible}<ScheduleWebsite on:saved={saved}/>{/if}
    {#if error}<p role="alert">{error}</p>{/if}
    <button type="button" class="close" on:click={()=>dialog.close()}>Закрыть</button>
</dialog>
<style>dialog{width:min(560px,calc(100vw - 40px));box-sizing:border-box;background:#091619;color:#eaf8fa;border:1px solid #37626a;border-radius:15px;padding:1rem;box-shadow:0 25px 90px #0008;font-family:'Manrope Variable',sans-serif}dialog::backdrop{background:#020a0dcc}.close{margin-top:.8rem;padding:.6rem 1rem;background:#17353b;color:#b6e7e8;border:1px solid #37626a;border-radius:7px;cursor:pointer}p{color:#ffabab;overflow-wrap:anywhere}</style>

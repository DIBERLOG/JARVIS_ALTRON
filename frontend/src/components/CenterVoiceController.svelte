<script lang="ts">
    import { onMount, tick } from 'svelte'
    import { goto } from '@roxi/routify'
    import { handleCenterVoice, centerVoiceError, navigateCenter, trainingVoiceTab, resetCenterVoice, remoteCenterEditing } from '@/lib/centerVoice'
    import { sendAction, activeNotification } from '@/lib/ipc'
    import {get} from 'svelte/store'
    import {invoke} from '@tauri-apps/api/core'
    let navigate: (path: string) => void
    $: navigate = $goto
    onMount(() => {
        let queue = Promise.resolve()
        const open = () => navigate('/center')
        const command = (event: Event) => {
            const text = (event as CustomEvent<string>).detail
            queue = queue.then(() => handleCenterVoice(text)).catch(error => {
                centerVoiceError(error)
            })
        }
        const remote=(event:Event)=>{const {id,request_id}=(event as CustomEvent<{id:string,request_id:string}>).detail;queue=queue.then(async()=>{
            if(get(remoteCenterEditing)){activeNotification.set({id:Date.now(),title:'Команда с телефона',primary:'Сначала сохраните открытую заметку на ПК'});await invoke('lan_sync_command_result',{id:request_id,status:'failed'});return}
            const pages:Record<string,string>={center_open:'calendar',center_news_open:'news',center_calendar:'calendar',center_notes:'notes',center_timer_open:'timer',center_reminders:'reminders',center_birthdays:'birthdays',center_weather:'weather',center_habits:'habits',center_training:'workouts',center_stats:'workouts',center_history:'request-history'};
            if(id==='center_schedule'){resetCenterVoice();await handleCenterVoice('открой расписание')}else if(pages[id]){resetCenterVoice();if(id==='center_training'||id==='center_stats')trainingVoiceTab.set(id==='center_stats'?'stats':'today');navigateCenter(pages[id]);await tick()}else{await invoke('lan_sync_command_result',{id:request_id,status:'failed'});return}
            await invoke('lan_sync_command_result',{id:request_id,status:'shown'});
        }).catch(()=>invoke('lan_sync_command_result',{id:request_id,status:'failed'}));};
        window.addEventListener('jarvis-remote-center-command',remote)
        window.addEventListener('jarvis-center-open',open)
        window.addEventListener('jarvis-center-command',command)
        return () => {window.removeEventListener('jarvis-remote-center-command',remote);window.removeEventListener('jarvis-center-open',open);window.removeEventListener('jarvis-center-command',command)}
    })
</script>

<script lang="ts">
    import { onMount } from "svelte"
    import { invoke } from '@tauri-apps/api/core'
    import { activeNotification, sendAction, ipcConnected } from "@/lib/ipc"
    import { get } from 'svelte/store'
    import { birthdayDate, birthdayOccursInYear, dayKey, loadCenterData } from "@/lib/center"
    import { reminderAlert } from '@/lib/reminders'

    onMount(() => {
        const shown = new Set<string>()
        let busy = false
        async function check() {
            if (busy) return
            busy = true
            try {
                const data = await loadCenterData()
                const ledger:Record<string,string[]> = JSON.parse(await invoke<string>('db_read',{key:'center_reminder_announcements'}) || '{}')
                const now = new Date()
                const due = data.reminders.map(item=>({item,alert:reminderAlert({...item,announced:[...(item.announced||[]),...(ledger[item.id]||[])]},now.getTime())})).find(({alert})=>alert)
                if (due?.alert && get(ipcConnected)) {
                    const {item,alert}=due
                    const nextLedger=Object.fromEntries(data.reminders.filter(r=>ledger[r.id]).map(r=>[r.id,ledger[r.id]]))
                    nextLedger[item.id]=[...(ledger[item.id] || []).slice(-30),alert.key]
                    if(!await invoke<boolean>('db_write',{key:'center_reminder_announcements',val:JSON.stringify(nextLedger)}))throw new Error('Не удалось сохранить отметку уведомления')
                    activeNotification.set({ id: Date.now(), title: "Напоминание", primary: item.title, detail: alert.detail })
                    sendAction('center_reply',{text:alert.text,reply_id:'reminder_notice',follow_up:false})
                    return
                }
                const birthday = data.birthdays.find(item => birthdayOccursInYear(item, now.getFullYear()) && dayKey(birthdayDate(item, now.getFullYear())) === dayKey(now) && !shown.has(`birthday-${item.id}-${dayKey(now)}`))
                if (birthday) {
                    shown.add(`birthday-${birthday.id}-${dayKey(now)}`)
                    activeNotification.set({ id: Date.now(), title: "День рождения", primary: birthday.name, detail: "Сегодня важная дата", })
                }
            } catch (error) {
                console.error("Не удалось проверить напоминания Центра", error)
            } finally { busy = false }
        }
        check()
        const timer = window.setInterval(check, 15_000)
        return () => window.clearInterval(timer)
    })
</script>

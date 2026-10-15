import type { Reminder } from './center'

export function parseReminderDate(raw: string, now = new Date(), allowPast=false): Date | null {
    let text = raw.toLowerCase().replace(/ё/g, 'е')
    const numbers: Record<string, number> = {один:1,одну:1,одна:1,два:2,две:2,три:3,четыре:4,пять:5,шесть:6,семь:7,восемь:8,девять:9,десять:10,пятнадцать:15,двадцать:20,тридцать:30}
    const clockNumbers:Record<string,number>={...numbers,ноль:0,одиннадцать:11,двенадцать:12,тринадцать:13,четырнадцать:14,шестнадцать:16,семнадцать:17,восемнадцать:18,девятнадцать:19,сорок:40,пятьдесят:50}
    const number=(s:string)=>{if(/^\d+$/.test(s))return Number(s);const parts=s.trim().split(/\s+/);const values=parts.map(p=>clockNumbers[p]);return values.every(n=>n!==undefined)?values.reduce((a,b)=>a+b,0):NaN}
    // Speech recognition often returns words, rather than a colon-separated clock.
    if(!text.includes('через')&&!/\d{1,2}[:.]\d{2}/.test(text)) {
        const spoken=text.match(/\sв\s+(.+?)(?:\s+(утра|вечера|дня|ночи))?$/)
        if(spoken){
            const chunks=spoken[1].replace(/\s+(часа|часов|час|минуты|минут|минута)/g,'').trim().split(/\s+/)
            let hour=number(chunks[0]),used=1
            if(hour>=20&&hour%10===0&&chunks.length>1&&number(chunks[1])<10){hour+=number(chunks[1]);used=2}
            const minute=chunks.length>used?number(chunks.slice(used).join(' ')):0
            if((spoken[2]==='вечера'||spoken[2]==='дня')&&hour<12)hour+=12
            if(hour===12&&spoken[2]==='ночи')hour=0
            if(Number.isFinite(hour)&&Number.isFinite(minute))text=text.slice(0,spoken.index)+' в '+hour+':'+String(minute).padStart(2,'0')
        }
    }
    const relative = text.match(/через\s+(\d+|[а-я]+)\s*(секунд|минут|час|день|дня|дней|недел)/)
    if (relative) {
        const amount = Number(relative[1]) || numbers[relative[1]]
        const unit = relative[2], scale = unit.startsWith('секунд') ? 1000 : unit.startsWith('минут') ? 60000 : unit.startsWith('час') ? 3600000 : unit.startsWith('недел') ? 604800000 : 86400000
        return amount > 0 && amount <= 365 ? new Date(now.getTime() + amount * scale) : null
    }
    const match = text.match(/(?:(\d{1,2})[.\/-](\d{1,2})(?:[.\/-](\d{4}))?|(послезавтра|завтра|сегодня|вчера))\s*(?:в\s*)?(\d{1,2})[:.](\d{2})/)
    if (!match) return null
    const due = new Date(now)
    if (match[4]) due.setDate(due.getDate() + (match[4] === 'послезавтра' ? 2 : match[4] === 'завтра' ? 1 : match[4]==='вчера'?-1:0))
    else {
        const year=Number(match[3] || now.getFullYear()),month=Number(match[2])-1,day=Number(match[1])
        due.setFullYear(year,month,day)
        if (due.getFullYear()!==year || due.getMonth()!==month || due.getDate()!==day) return null
    }
    const hour=Number(match[5]),minute=Number(match[6])
    if (hour>23 || minute>59) return null
    due.setHours(hour,minute,0,0)
    return allowPast || due.getTime()>now.getTime() ? due : null
}

export function reminderAlert(item: Reminder, now = Date.now()): {key:string;text:string;detail:string} | null {
    if (item.done) return null
    let due=Date.parse(item.dueAt)
    if (!Number.isFinite(due)) return null
    if(item.repeat==='daily') {
        const anchor=new Date(due), today=new Date(now)
        today.setHours(anchor.getHours(),anchor.getMinutes(),anchor.getSeconds(),anchor.getMilliseconds())
        due=today.getTime()
        if(due<Date.parse(item.dueAt) || due<Date.parse(item.createdAt || item.dueAt) || due>now)return null
        const key=`${today.toISOString()}:daily`
        if(item.announced?.includes(key))return null
        return {key,text:`Сэр, пришло время: ${item.title}.`,detail:`Ежедневно · ${today.toLocaleTimeString('ru-RU',{hour:'2-digit',minute:'2-digit'})}`}
    }
    if (!Number.isFinite(due)) return null
    if(item.silentPast && due<=Date.parse(item.createdAt || item.dueAt))return null
    const offsets=[0,...new Set(item.advanceDays || [])].filter(n=>Number.isInteger(n)&&n>=0&&n<=365).sort((a,b)=>a-b)
    for (const days of offsets) {
        const at=due-days*86400000,key=`${item.dueAt}:${days}`
        // Don't replay missed advance warnings or warnings predating creation.
        if (now<at || (days>0 && (now>=due || now-at>86400000 || at<Date.parse(item.createdAt || '1970-01-01')))) continue
        if (item.announced?.includes(key)) continue
        return {key,text:days ? `Сэр, напоминание заранее: ${item.title}. Запланировано на ${new Date(due).toLocaleString('ru-RU')}.` : `Сэр, пришло время: ${item.title}.`,detail:days ? `Предупреждение за ${days} дн. · ${new Date(due).toLocaleString('ru-RU')}` : new Date(due).toLocaleString('ru-RU')}
    }
    return null
}

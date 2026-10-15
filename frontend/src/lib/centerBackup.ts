import { invoke } from '@tauri-apps/api/core'
import { loadCenterData, type CenterData } from './center'
import { getWeatherCity } from './weather'
import { sanitizeNoteHtml } from './richNotes'
import {validateMindMap} from './mindMap'

export type CenterBackup = {format:'jarvis-personal-data';version:1;exportedAt:string;city:string;data:CenterData}
export async function createCenterBackup(): Promise<CenterBackup> {
    const data=await loadCenterData()
    const ledger:Record<string,string[]>=JSON.parse(await invoke<string>('db_read',{key:'center_reminder_announcements'})||'{}')
    data.reminders=data.reminders.map(item=>({...item,announced:[...new Set([...(item.announced||[]),...(ledger[item.id]||[])])]}))
    return {format:'jarvis-personal-data',version:1,exportedAt:new Date().toISOString(),city:await getWeatherCity(),data}
}
export function parseCenterBackup(raw: string): CenterBackup {
    if(raw.length>50*1024*1024)throw new Error('Файл больше 50 МБ.')
    const parsed=JSON.parse(raw)
    const fail=()=>{throw new Error('Некорректный файл данных JARVIS. Импорт отменён.')}
    const string=(v:unknown)=>typeof v==='string'
    const date=(v:unknown)=>string(v)&&Number.isFinite(Date.parse(v as string))
    const array=(v:unknown)=>Array.isArray(v)&&v.length<=100000
    const record=(v:unknown)=>!!v&&typeof v==='object'&&!Array.isArray(v)
    if(!parsed||parsed.format!=='jarvis-personal-data'||parsed.version!==1||!date(parsed.exportedAt)||!string(parsed.city)||!parsed.city.trim()||parsed.city.length>100)fail()
    const d=parsed.data
    if(!d||!['reminders','birthdays','notes','habits'].every(key=>array(d[key])))fail()
    for(const key of ['reminders','birthdays','notes','habits']){
        const ids=new Set<string>()
        for(const item of d[key]){if(!record(item)||!string(item.id)||!item.id||ids.has(item.id))fail();ids.add(item.id)}
    }
    for(const r of d.reminders)if(!string(r.title)||!date(r.dueAt)||typeof r.done!=='boolean'||(r.repeat!==undefined&&r.repeat!=='none'&&r.repeat!=='daily')||(r.advanceDays!==undefined&&(!array(r.advanceDays)||!r.advanceDays.every((n:number)=>Number.isInteger(n)&&n>=1&&n<=365)))||(r.announced!==undefined&&(!array(r.announced)||!r.announced.every(string)))||(r.createdAt!==undefined&&!date(r.createdAt)))fail()
    for(const b of d.birthdays){const dt=new Date(2024,b.month-1,b.day);if(!string(b.name)||!Number.isInteger(b.day)||!Number.isInteger(b.month)||dt.getMonth()!==b.month-1||dt.getDate()!==b.day||(b.year!==undefined&&(!Number.isInteger(b.year)||b.year<1||b.year>9999)))fail()}
    for(const n of d.notes){if(!string(n.title)||!string(n.text)||!date(n.updatedAt)||(n.items!==undefined&&(!array(n.items)||!n.items.every((i:any)=>record(i)&&string(i.id)&&string(i.text)&&typeof i.done==='boolean'))))fail();if(n.type==='mindmap'){n.mindMap=validateMindMap(n.mindMap)}if(n.html!==undefined){if(!string(n.html))fail();n.html=sanitizeNoteHtml(n.html)}}
    for(const h of d.habits)if(!string(h.title)||!string(h.caption)||!string(h.icon)||!string(h.unit)||!date(h.createdOn)||!Number.isInteger(h.target)||h.target<1||!record(h.entries)||!Object.values(h.entries).every(n=>typeof n==='number'&&Number.isFinite(n)&&n>=0))fail()
    if(d.training!==undefined){
        const t=d.training
        if(!record(t)||!['exercises','plans','sessions'].every(key=>array(t[key])))fail()
        for(const e of t.exercises)if(!record(e)||!['id','name','muscle','equipment'].every(k=>string(e[k])))fail()
        for(const p of t.plans)if(!record(p)||!string(p.id)||!string(p.name)||!array(p.items)||!p.items.every((i:any)=>record(i)&&string(i.exerciseId)&&['sets','minReps','maxReps','rest'].every(k=>typeof i[k]==='number'&&Number.isFinite(i[k]))))fail()
        for(const s of t.sessions)if(!record(s)||!string(s.id)||!string(s.name)||!date(s.startedAt)||!['active','paused','completed','cancelled'].includes(s.status)||!array(s.items)||!array(s.sets)||!array(s.warmup)||typeof s.elapsed!=='number'||!Number.isFinite(s.elapsed)||!string(s.note))fail()
        if(t.profile&&(!record(t.profile)||!array(t.profile.measurements)||!t.profile.measurements.every((m:any)=>record(m)&&string(m.id)&&date(m.date)&&typeof m.weight==='number'&&Number.isFinite(m.weight))))fail()
        // A transferred active workout must not accumulate time while the computer was off.
        for(const s of t.sessions)if(s.status==='active'){s.status='paused';s.resumedAt=null}
    }
    return {format:'jarvis-personal-data',version:1,exportedAt:parsed.exportedAt,city:parsed.city.trim(),data:{reminders:d.reminders,birthdays:d.birthdays,notes:d.notes,habits:d.habits,training:d.training}}
}
export async function restoreCenterBackup(backup:CenterBackup) {
    await invoke('center_restore_data',{data:JSON.stringify(backup.data),city:backup.city})
}

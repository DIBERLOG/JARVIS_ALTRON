import {get,writable} from 'svelte/store'
import {invoke} from '@tauri-apps/api/core'
export type RequestEntry={id:string,text:string,source:'chat'|'voice',at:string}
const key='jarvis-request-history-v1'
let saved:RequestEntry[]=[]
try{const value=JSON.parse(localStorage.getItem(key)||'[]');if(Array.isArray(value))saved=value.filter(item=>typeof item.id==='string'&&typeof item.text==='string'&&item.text.length<=12000&&['chat','voice'].includes(item.source)&&Number.isFinite(Date.parse(item.at))).slice(0,500)}catch{}
export const requestHistory=writable<RequestEntry[]>([])
let pending=Promise.resolve(),clearedBeforeReady=false
const ready=invoke<string>('db_read',{key:'request_history_v1'}).then(raw=>{
    const loaded:RequestEntry[]=raw?JSON.parse(raw):saved
    if(!Array.isArray(loaded))throw new Error('Некорректная защищённая история запросов')
    const current=get(requestHistory),ids=new Set(current.map(e=>e.id))
    requestHistory.set([...current,...(clearedBeforeReady?[]:loaded.filter(e=>!ids.has(e.id)))].sort((a,b)=>b.at.localeCompare(a.at)).slice(0,500))
    // Keep legacy localStorage intact. New writes use OS-protected settings only.
})
ready.catch(()=>console.warn('Не удалось открыть защищённую историю запросов; исходные данные сохранены'))
function persist(){pending=pending.catch(()=>{}).then(async()=>{await ready;if(!await invoke<boolean>('db_write',{key:'request_history_v1',val:JSON.stringify(get(requestHistory))}))throw new Error('История не сохранена')});pending.catch(()=>console.warn('Не удалось сохранить защищённую историю запросов'))}

export function recordRequest(text:string,source:RequestEntry['source']){
    if(!text.trim())return
    requestHistory.update(entries=>{const next=[{id:crypto.randomUUID(),text:text.trim().slice(0,12000),source,at:new Date().toISOString()},...entries].slice(0,500);queueMicrotask(persist);return next})
}
export function clearRequestHistory(){clearedBeforeReady=true;requestHistory.set([]);persist()}

import { invoke } from '@tauri-apps/api/core'
export type ChatMessage={role:'user'|'assistant';content:string}
export type ChatSession={id:string;title:string;updatedAt:string;messages:ChatMessage[]}
export function parseChatHistory(raw:string):ChatSession[]{
    if(!raw)return []
    if(raw.length>20_000_000)throw new Error('История чатов превышает 20 МБ')
    const value=JSON.parse(raw)
    if(!Array.isArray(value)||value.length>10000)throw new Error('Некорректная история чатов')
    const ids=new Set<string>()
    for(const chat of value){
        if(!chat||typeof chat.id!=='string'||!chat.id||ids.has(chat.id)||typeof chat.title!=='string'||chat.title.length>200||!Number.isFinite(Date.parse(chat.updatedAt))||!Array.isArray(chat.messages))throw new Error('Некорректная запись чата')
        ids.add(chat.id)
        if(chat.messages.some((m:ChatMessage)=>!m||!['user','assistant'].includes(m.role)||typeof m.content!=='string'))throw new Error('Некорректное сообщение чата')
    }
    return value
}
let lastSubmitted='[]'
export async function loadChatHistory(){const raw=await invoke<string>('db_read',{key:'chat_history_v1'});const chats=parseChatHistory(raw);lastSubmitted=JSON.stringify(chats);return chats}
let pending=Promise.resolve()
export function saveChatHistory(chats:ChatSession[]){
    const raw=JSON.stringify(chats)
    parseChatHistory(raw)
    const base=lastSubmitted;lastSubmitted=raw
    const write=pending.catch(()=>{}).then(async()=>{await invoke('chat_save_history',{base,data:raw})})
    pending=write
    return write
}
export function chatExport(chats:ChatSession[]){
    // Explicit allowlist: settings, credentials and provider responses are never exported.
    return JSON.stringify({format:'jarvis-chats',version:1,exportedAt:new Date().toISOString(),chats:chats.map(c=>({id:c.id,title:c.title,updatedAt:c.updatedAt,messages:c.messages.map(m=>({role:m.role,content:m.content}))}))},null,2)
}

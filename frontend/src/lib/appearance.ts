import {writable} from 'svelte/store'
import {isTauri} from '@tauri-apps/api/core'
import {getCurrentWindow} from '@tauri-apps/api/window'
export type Appearance='dark'|'light'|'system'
const key='jarvis-appearance-v1'
function saved():Appearance{try{const value=localStorage.getItem(key);return value==='light'||value==='system'?value:'dark'}catch{return 'dark'}}
export const appearance=writable<Appearance>(saved())
export const resolvedAppearance=writable<'dark'|'light'>('dark')
const system=window.matchMedia('(prefers-color-scheme: light)')
let selected:Appearance=saved()
function apply(){document.documentElement.dataset.theme=selected==='system'?(system.matches?'light':'dark'):selected;document.documentElement.style.colorScheme=document.documentElement.dataset.theme!;resolvedAppearance.set(document.documentElement.dataset.theme as "dark"|"light");if(isTauri())getCurrentWindow().setTheme(selected==='system'?null:selected).catch(error=>console.error('Window theme could not be applied',error))}
appearance.subscribe(value=>{selected=value;try{localStorage.setItem(key,value)}catch{}apply()})
system.addEventListener('change',()=>{if(selected==='system')apply()})

<script lang="ts">
import {onMount,tick} from 'svelte'
import CenterNotes from '../../src/components/CenterNotes.svelte'
import {newMindMap,mapText} from '../../src/lib/mindMap'
import type {Note} from '../../src/lib/center'
const mobile=location.search.includes('mobile'),map=newMindMap('Мой проект','Идеи, материалы и план действий')
map.nodes[1].text='Запустить первую версию';map.nodes[2].text='Голосовые заметки\nУмные связи';map.nodes[4].items[0].done=true;map.nodes[5].text='Собрать всё важное в одном месте'
let notes:Note[]=JSON.parse(sessionStorage.getItem('mindmap-fixture')||'null')||[{id:'fictional-map',title:'Мой проект',type:'mindmap',format:'plain',text:mapText(map),mindMap:map,updatedAt:'2026-10-15T12:00:00Z'}],component:CenterNotes
let mounted=true
const fixture={unmount:()=>mounted=false,remount:async()=>{mounted=true;await tick();component.selectId('fictional-map')},failSave:false,saved:[] as Note[],notes:()=>notes,picks:[] as string[]}
;(window as any).fixture=fixture
;(window as any).NativeCenter={pickImage:()=>fixture.picks.push('image'),pickAttachment:()=>fixture.picks.push('file'),exportAttachment:(raw:string)=>fixture.picks.push(JSON.parse(raw).name)}
async function save(note:Note){if(fixture.failSave)return false;notes=[...notes.filter(n=>n.id!==note.id),structuredClone(note)];fixture.saved.push(structuredClone(note));sessionStorage.setItem('mindmap-fixture',JSON.stringify(notes));return true}
onMount(()=>{if(location.search.includes('many'))notes=[...notes,...Array.from({length:20},(_,n)=>({id:'list-fixture-'+n,title:'Заметка с заголовком '+n,type:'text' as const,text:'Содержимое',updatedAt:'2026-10-15T12:00:00Z'}))];component.selectId(location.search.includes('many')?'list-fixture-0':'fictional-map');if(location.search.includes('light'))document.documentElement.dataset.theme='light'})
</script>
<div class="fixture-shell">{#if mounted}<CenterNotes bind:this={component} {mobile} {notes} onSave={save} onDelete={async id=>{notes=notes.filter(n=>n.id!==id);return true}}/>{/if}</div>
<style>:global(body){margin:0;background:#07121b;font-family:'Manrope Variable',sans-serif}.fixture-shell{max-width:1320px;margin:auto;padding:22px}:global(html[data-theme=light]) :global(body){background:#edf5f8}@media(max-width:600px){.fixture-shell{padding:12px}}</style>

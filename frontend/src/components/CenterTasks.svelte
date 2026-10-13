<script lang="ts">
 import type {QuickTask} from '@/lib/center';
 import {makeId} from '@/lib/center';
 export let tasks:QuickTask[]=[];
 export let disabled=false;
 export let onSave:(tasks:QuickTask[])=>Promise<boolean>;
 let text='',error='';
 async function add(){if(!text.trim())return;const now=new Date().toISOString();if(await save([...tasks,{id:makeId(),text:text.trim(),done:false,createdAt:now,updatedAt:now}]))text='';}
 async function save(next:QuickTask[]){try{error='';return await onSave(next)}catch(e){error=String(e);return false}}
 async function edit(task:QuickTask){const value=window.prompt('Текст задачи',task.text);if(value===null||!value.trim())return;await save(tasks.map(row=>row.id===task.id?{...row,text:value.trim().slice(0,1000),updatedAt:new Date().toISOString()}:row));}
 async function toggle(task:QuickTask,event:Event){const input=event.currentTarget as HTMLInputElement;if(!await save(tasks.map(row=>row.id===task.id?{...row,done:input.checked,updatedAt:new Date().toISOString()}:row)))input.checked=task.done;}
</script>
<section class="tasks-panel" aria-label="Общие задачи">
 <h2>Задачи</h2><p>Общий список на компьютере и телефоне. Изменения сохраняются автономно и передаются при обмене.</p>
 <form on:submit|preventDefault={add}><input aria-label="Новая задача" placeholder="Что нужно сделать?" maxlength="1000" bind:value={text} disabled={disabled}/><button disabled={disabled||!text.trim()}>Добавить</button></form>
 {#if !tasks.length}<p>Пока нет задач.</p>{/if}
 {#each tasks as task(task.id)}<div class="row"><input type="checkbox" aria-label={`Выполнено: ${task.text}`} checked={task.done} disabled={disabled} on:change={(event)=>toggle(task,event)}/><span class:done={task.done}>{task.text}</span><button disabled={disabled} on:click={()=>edit(task)}>Изменить</button><button disabled={disabled} on:click={()=>{if(confirm('Удалить задачу на всех устройствах?'))save(tasks.filter(row=>row.id!==task.id))}}>Удалить</button></div>{/each}
 {#if error}<p role="alert">{error}</p>{/if}
</section>
<style>
 .tasks-panel{padding:18px;border:1px solid #295056;border-radius:14px;background:#0d1b20;color:#eafafa;font-family:inherit}h2{margin-top:0}p{color:#a7b2b8;font-size:14px;line-height:1.5}form,.row{display:flex;gap:10px;align-items:center;margin:12px 0}.row span{flex:1;overflow-wrap:anywhere}.done{text-decoration:line-through;color:#8b9a9d}input:not([type=checkbox]){flex:1;min-width:0}input,button{font:inherit;padding:10px;border:1px solid #295056;background:#112a2f;color:#eafafa;border-radius:10px;min-height:42px}input[type=checkbox]{width:22px;accent-color:#a0e4ce}button{cursor:pointer}button:disabled{opacity:.5}@media(max-width:550px){.row{flex-wrap:wrap}.row span{flex-basis:70%}}
</style>

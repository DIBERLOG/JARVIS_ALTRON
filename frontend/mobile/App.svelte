<script context="module" lang="ts">
 declare const NativeCenter:{load:()=>string;save:(data:string)=>string;dirty:(dirty:boolean)=>void};
 declare const JarvisMode:string;
</script>
<script lang="ts">
 import CenterTraining from '../src/components/CenterTraining.svelte';
 import CenterNotes from '../src/components/CenterNotes.svelte';
 import CenterHabits from '../src/components/CenterHabits.svelte';
 import type {TrainingData} from '../src/lib/training';
 import type {Note} from '../src/lib/center';
 import {onMount,tick} from 'svelte';
 import {trainingVoiceTab} from './voice';
 let error='',notice='',notesComponent:CenterNotes,trainingComponent:CenterTraining,trainingDirty=false,revision=0;
 let center=JSON.parse(NativeCenter.load());
 function normalTraining(value:TrainingData|undefined){if(!value)return undefined;const copy=JSON.parse(JSON.stringify(value));for(const session of copy.sessions??[]){session.elapsed??=Math.max(0,Date.parse(session.finishedAt||session.startedAt)-Date.parse(session.startedAt))||0;session.resumedAt??=null;session.note??='';session.warmup??=[false,false,false];session.items=(session.items??[]).map((item:any)=>({...item,sets:item.sets??session.sets?.filter((s:any)=>s.exerciseId===item.exerciseId&&!s.warmup).length??1,minReps:item.minReps??8,maxReps:item.maxReps??12,rest:item.rest??120}));}return copy;}
 let training=normalTraining(center.training);
 function leaveTraining(){if(trainingDirty&&!confirm('Есть несохранённые поля. Перейти без сохранения?'))return false;trainingDirty=false;NativeCenter.dirty(false);return true;}
 (window as any).jarvisCanLeave=()=>JarvisMode!=='notes'?leaveTraining():notesComponent?.requestLeave()??true;
 (window as any).jarvisRefresh=async()=>{if(trainingDirty||notesComponent?.hasChanges())return false;const id=notesComponent?.selectedId(),tab=trainingComponent?.currentTab();center=JSON.parse(NativeCenter.load());training=normalTraining(center.training);if(tab)trainingVoiceTab.set(tab);revision++;await tick();if(id)notesComponent?.selectId(id);return true;};
 onMount(()=>{const changed=()=>{if(JarvisMode!=='notes'){trainingDirty=true;NativeCenter.dirty(true);}};document.addEventListener('input',changed);document.addEventListener('change',changed);return()=>{document.removeEventListener('input',changed);document.removeEventListener('change',changed);};});
 function commit(value:object){error='';notice='';const result=JSON.parse(NativeCenter.save(JSON.stringify(value)));if(!result.ok){error=result.error||'Не удалось сохранить. Исходные данные сохранены.';return false;}center=result.center;training=normalTraining(center.training);trainingDirty=false;NativeCenter.dirty(false);notice='Сохранено на телефоне · будет передано при обмене';return true;}
 async function saveTraining(value:TrainingData,habit?:string){return commit({training:value,habit});}
 async function saveNote(note:Note){return commit({note});}
 async function saveHabits(habits:any[]){return commit({habits});}
 async function deleteNote(id:string){return commit({deleteNote:id});}
</script>
<main id="mobile-center">
 {#key revision}
 {#if center.error}<p role="alert" class="error">{center.error}</p>
 {:else if JarvisMode==='habits'}<CenterHabits habits={center.habits} onSave={saveHabits}/>
 {:else if JarvisMode==='training'}<CenterTraining bind:this={trainingComponent} mobile data={training} onSave={saveTraining} beforeTabChange={leaveTraining}/>
 {:else}<CenterNotes mobile bind:this={notesComponent} notes={center.notes} onSave={saveNote} onDelete={deleteNote} onDirty={(value)=>NativeCenter.dirty(value)}/>{/if}
 {/key}
 {#if error}<p class="error" role="alert">{error}</p>{/if}
 {#if notice}<p class="notice" role="status">{notice}</p>{/if}
</main>

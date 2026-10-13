import {get,writable} from 'svelte/store';
import type {TimerState,TimerPreset} from '../src/lib/timer';
declare const NativeCenter:{timer:(data:string)=>string};
const initial=JSON.parse(NativeCenter.timer(''));
export const timerState=writable<TimerState>(initial);
function save(state:TimerState){const actual=JSON.parse(NativeCenter.timer(JSON.stringify(state)));if(actual.error)throw new Error(actual.error);timerState.set(actual);}
export function selectTimer(preset:TimerPreset){save({label:preset.label,total:preset.seconds,remaining:preset.seconds,deadline:null,phase:preset.seconds>0?'idle':'finished',owner:preset.seconds>0?preset.owner:undefined});}
export function startTimer(){const state=get(timerState);if(state.phase==='running')return;const remaining=state.remaining||state.total;if(remaining<=0)return;save({...state,remaining,deadline:Date.now()+remaining*1000,phase:'running'});}
export function pauseTimer(){const state=get(timerState);if(state.phase==='running')save({...state,remaining:Math.max(0,Math.ceil(((state.deadline??Date.now())-Date.now())/1000)),deadline:null,phase:'paused'});}
export function resetTimer(){const state=get(timerState);save({...state,remaining:state.total,deadline:null,phase:'idle'});}
export function formatTimer(seconds:number,full=false){const pair=(v:number)=>String(v).padStart(2,'0');return `${seconds>=3600||full?pair(Math.floor(seconds/3600))+':':''}${pair(Math.floor(seconds%3600/60))}:${pair(Math.floor(seconds%60))}`;}
setInterval(()=>{const state=get(timerState);if(state.phase!=='running'||state.deadline===null)return;const remaining=Math.max(0,Math.ceil((state.deadline-Date.now())/1000));timerState.set({...state,remaining,phase:remaining?'running':'finished'});},1000);

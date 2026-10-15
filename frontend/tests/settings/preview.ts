import '@fontsource-variable/manrope'
import '../../src/css/appearance.css'
const preferences:Record<string,string>={assistant_voice:'fictional-voice',selected_microphone:'-1',selected_wake_word_engine:'Rustpotter',intent_backend:'IntentClassifier',slots_backend:'None',selected_gliner_model:'',selected_vosk_model:'',noise_suppression:'None',vad_backend:'Energy',gain_normalizer:'false',voice_dialogue_personality:'jarvis',tts_mode:'silero',audio_output_mode:'direct',monitor_self:'false',api_key__picovoice:''}
const fixture={preferences,writes:[] as {key:string,val:string}[][],failWrite:false}
;(window as any).fixture=fixture
;(window as any).__TAURI_INTERNALS__={metadata:{currentWindow:{label:'fixture'},currentWebview:{label:'fixture'}},invoke:async(command:string,args:any={})=>{
    if(command==='db_read'){if(location.search.includes('loadError'))throw new Error('Fictional load failure');return preferences[args.key]||''}
    if(command==='db_write_many'){if(fixture.failWrite)throw new Error('Fictional save failure');fixture.writes.push(args.entries);for(const entry of args.entries)preferences[entry.key]=entry.val;return}
    if(command==='list_voices')return [{voice:{id:'fictional-voice',name:'Голос JARVIS',author:'',languages:['ru']}}]
    if(command==='get_app_version')return '1.1.0-fictional'
    if(command==='pv_get_audio_devices')return ['Тестовый микрофон']
    if(command==='list_vosk_models')return [{name:'fictional-ru',language:'ru',size:'small'}]
    if(command==='list_gliner_models')return []
    if(command==='chat_get_config')return {provider:'deepseek',deepseek_model:'deepseek-flash',local_model:'qwen3:8b',openai_model:'gpt-4.1-mini',deepseek_configured:false,openai_configured:false,speak_responses:false,personality:'jarvis'}
    if(command==='lan_sync_addresses')return ['192.168.1.42']
    if(command==='lan_sync_status')return {prepared:true,restored:false,client:false,running:false,pending:[],devices:[],v2Conflicts:[],shareChats:false,allowVault:false}
    return null
}}
const {translations}=await import('../../src/lib/i18n')
const dictionary=await import('../../../crates/jarvis-core/src/i18n/locales/ru.ftl?raw')
translations.set(Object.fromEntries(dictionary.default.split('\n').filter(line=>/^[\w-]+ = /.test(line)).map(line=>{const split=line.indexOf(' = ');return [line.slice(0,split),line.slice(split+3)]})))
const {assistantVoice}=await import('../../src/stores')
const {ipcConnected,microphoneMuteKnown}=await import('../../src/lib/ipc')
assistantVoice.set('fictional-voice');ipcConnected.set(true);microphoneMuteKnown.set(true)
const Preview=(await import('./Preview.svelte')).default
new Preview({target:document.getElementById('fixture')!})
document.body.style.cssText="margin:0;background:var(--ui-bg);font-family:'Manrope Variable',sans-serif"

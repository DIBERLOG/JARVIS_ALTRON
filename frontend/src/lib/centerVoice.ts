import { writable, get } from 'svelte/store'
import { invoke } from '@tauri-apps/api/core'
import { sendAction } from './ipc'
import { loadCenterData, saveCenterData, dayKey, makeId, type CenterData } from './center'
import { timerPresets, customPresets, selectTimer, startTimer, pauseTimer, resetTimer } from './timer'
import { defaultTraining } from './training'
import { getWeekWeather, weatherPeriod } from './weather'
import { handleMailVoice, resetMailVoice } from './outlook'
import { parseReminderDate } from './reminders'

export const centerSection = writable('calendar')
export const trainingVoiceTab = writable('today')
export const centerVoiceHint = writable('')
let hintTimer: ReturnType<typeof setTimeout>
export const centerRevision = writable(0)
export type VoiceNews = { title: string; url: string }
export const voiceNews = writable<VoiceNews[]>([])
export let newsVoiceAction: ((action: string, index?: number) => Promise<void>) | undefined
export function setNewsVoiceAction(action?: typeof newsVoiceAction) { newsVoiceAction = action }
async function waitForNewsAction() {
    for(let attempt=0;attempt<40;attempt++){
        if(newsVoiceAction)return newsVoiceAction
        await new Promise(resolve=>setTimeout(resolve,100))
    }
    throw new Error('Личный центр сейчас недоступен. Откройте новостную ленту.')
}
type Pending = { kind: string; ids?: string[]; title?: string; id?: string; editId?:string; advanceDays?:number[] }
let pending: Pending | null = null
const clean = (text: string) => text.toLowerCase().replace(/ё/g,'е').trim().replace(/[.!?,]+$/g,'').trim()
export function spokenNumber(text: string): number | undefined {
    const numeric = text.match(/\b\d+\b/); if (numeric) return Number(numeric[0])
    const words = ['один','два','три','четыре','пять','шесть','семь','восемь','девять','десять','одиннадцать','двенадцать']
    const ordinal = ['перв','втор','трет','четверт','пят','шест','седьм','восьм','девят','десят','одиннадцат','двенадцат','тринадцат','четырнадцат','пятнадцат','шестнадцат','семнадцат','восемнадцат','девятнадцат','двадцат']
    const tokens = clean(text).split(/[^а-я]+/)
    const units:Record<string,number>={один:1,одна:1,одну:1,два:2,две:2,три:3,четыре:4,пять:5,шесть:6,семь:7,восемь:8,девять:9,десять:10,одиннадцать:11,двенадцать:12,тринадцать:13,четырнадцать:14,пятнадцать:15,шестнадцать:16,семнадцать:17,восемнадцать:18,девятнадцать:19,двадцать:20,тридцать:30,сорок:40,пятьдесят:50,шестьдесят:60}
    const base=tokens.findIndex(token=>units[token]!==undefined)
    if(base>=0){const value=units[tokens[base]],extra=units[tokens[base+1]];return value+(value>=20&&extra>0&&extra<10?extra:0)}
    const index = words.findIndex(word => tokens.includes(word)); if (index >= 0) return index + 1
    const order = ordinal.findIndex(word => tokens.some(token => new RegExp(`^${word}(ый|ая|ое|ую|ого|ий|ья|ье|ью|ой|ому|ым|ом|его)$`).test(token))); return order >= 0 ? order + 1 : undefined
}
function reply(text: string, id = '', followUp = false) {
    clearTimeout(hintTimer)
    centerVoiceHint.set(text)
    hintTimer=setTimeout(()=>centerVoiceHint.set(''),followUp?90000:6000)
    sendAction('center_reply', {text, reply_id:id, follow_up:followUp})
}
export function resetCenterVoice() { pending=null; resetMailVoice() }
export function centerVoiceError(error: unknown) {
    const message=typeof error==='string'?error:(error as Error)?.message||'Не удалось выполнить действие.'
    const id=/переводится/.test(message)?'news_translation_busy':/перевести|Ollama|перевод/.test(message)?'news_translation_error':/погод|прогноз/.test(message)?'weather_unavailable':/центр сейчас недоступен/i.test(message)?'voice_center_unavailable':'action_failed'
    pending=null;reply(message,id)
}
export function navigateCenter(section: string) { centerSection.set(section); window.dispatchEvent(new CustomEvent('jarvis-center-open')) }
async function save(data: CenterData) { await saveCenterData(data); centerRevision.update(value => value + 1) }
export async function handleCenterVoice(raw: string) {
    const text = clean(raw)
    if (/^(отмена|отмени|отменить|не надо|не нужно|нет|не сохраняй|передумал)$/.test(text)) {pending = null; resetMailVoice(); reply('Действие отменено, сэр.','action_cancelled'); return}
    if(!pending && /^(открой расписание|покажи расписание|расписание)$/.test(text)) {window.dispatchEvent(new CustomEvent('jarvis-schedule-open'));reply('Открываю ваш сайт или окно настройки расписания, сэр.');return}
    if (!pending && await handleMailVoice(raw, { navigate: navigateCenter, reply, number: spokenNumber })) return
    if (pending) {
        const step = pending
        if(step.kind==='duration'){
            const n=spokenNumber(text),seconds=n ? n*(text.includes('час')?3600:text.includes('секунд')?1:60):0
            if(seconds<1||seconds>86400){reply('Назовите время до 24 часов, например десять минут.','timer_duration_invalid',true);return}
            pending=null;selectTimer({id:'voice',label:raw,seconds,icon:'◷'});startTimer();reply('Таймер запущен, сэр.','timer_start');return
        }
        if(step.kind==='weather-period'){
            if(!/недел|месяц|7|30|60/.test(text)){reply('Выберите неделю, месяц или два месяца.','weather_ask_period',true);return}
            pending=null;await handleCenterVoice('погода на '+text);return
        }
        if (['news','habit','append','preset'].includes(step.kind)) {
            const namedPreset=step.kind==='preset'?[...timerPresets,...get(customPresets)].find(item=>clean(item.label)===text):undefined
            const n = namedPreset ? (step.ids?.indexOf(namedPreset.id) ?? -1)+1 : spokenNumber(text)
            if (!n || !step.ids || n > step.ids.length) {reply(`Назовите номер от одного до ${step.ids?.length || 1}, или скажите отмена.`, step.kind==='news'?'news_number_invalid':step.kind==='preset'?'timer_preset_invalid':'selection_invalid', true); return}
            if (step.kind === 'news') {
                if (get(voiceNews)[n-1]?.url !== step.ids[n-1] || !newsVoiceAction) {pending=null;reply('Лента изменилась. Выберите новость заново.','news_list_changed');return}
                pending=null; await newsVoiceAction('translate', n-1); reply(`Новость номер ${n} переведена на русский, сэр.`,'news_translation_done');return
            }
            if (step.kind === 'preset') {
                const preset = [...timerPresets,...get(customPresets)].find(item=>item.id===step.ids![n-1]);
                if (!preset) {pending=null;reply('Этот пресет больше недоступен.','timer_preset_invalid');return}
                pending=null; selectTimer(preset);startTimer();reply(`Запускаю таймер ${preset.label}.`,'timer_start');return
            }
            const data = await loadCenterData()
            if (step.kind === 'habit') {
                const habit = data.habits.find(item=>item.id===step.ids![n-1]); if (!habit) throw new Error('Привычка больше недоступна.')
                const today=dayKey(new Date());if((habit.entries[today]||0)>=habit.target){pending=null;reply('Сегодняшняя цель уже выполнена, сэр.','habit_target_reached');return}habit.entries[today]=Math.min(habit.target,(habit.entries[today]||0)+1)
                await save(data);pending=null;reply(`Отмечена привычка ${habit.title}. ${habit.entries[today]} из ${habit.target}.`,habit.target===1?'habit_marked':'habit_progress_added');return
            }
            pending={kind:'append-body',id:step.ids[n-1]};reply('Что добавить в эту заметку, сэр?', 'note_ask_append_text', true);return
        }
        if (step.kind === 'note-title' || step.kind === 'check-title') {
            pending={kind:step.kind==='check-title'?'check-body':'note-body',title:raw};reply('Продиктуйте содержание, сэр.', step.kind==='check-title'?'checklist_ask_items':'note_ask_content', true);return
        }
        if(step.kind==='city') {
            await getWeekWeather(raw,true);pending=null;reply(`Город изменён: ${raw}, сэр.`,'weather_city_saved');return
        }
        const data=await loadCenterData()
        if(step.kind==='reminder-select') {
            const n=spokenNumber(text),item=n&&data.reminders.find(item=>item.id===step.ids?.[n-1])
            if(!item){reply('Назовите номер напоминания из списка.','selection_invalid',true);return}
            pending={kind:'reminder-title',editId:item.id};reply('Продиктуйте новый текст напоминания, сэр.','',true);return
        }
        if(step.kind==='reminder-title'){pending={kind:'reminder-time',title:raw,editId:step.editId};reply('Назовите дату и время: например, завтра в 18:30.', 'reminder_ask_time', true);return}
        if(step.kind==='reminder-time') {
            const due=parseReminderDate(text,new Date(),true)
            if(!due){reply('Укажите дату и время: завтра в 18:30, вчера в 09:00 или через десять минут.','',true);return}
            pending={...step,kind:'reminder-advance',id:due.toISOString()}
            reply('Предупредить заранее? Скажите: за день, за два дня, за день и два, либо без предупреждения.','',true);return
        }
        if(step.kind==='reminder-advance') {
            const days=/без|не нужно|только в/.test(text)?[]:/день и два|один и два|два и один|два.*день|день.*два/.test(text)?[1,2]:/два|двое|2/.test(text)?[2]:/день|один|сутки|1/.test(text)?[1]:null
            if(!days){reply('Скажите за день, за два дня или без предупреждения.','',true);return}
            pending={...step,kind:'reminder-confirm',advanceDays:days}
            reply(`${step.editId?'Изменить':'Создать'} напоминание: ${step.title}. ${new Date(step.id!).toLocaleString('ru-RU')}. ${days.length?'Предупреждения за '+days.join(' и ')+' дн.':'Без предварительного предупреждения.'} Скажите да или отмена.`,'',true);return
        }
        if(step.kind==='reminder-confirm') {
            if(!/^(да|давай|ага|угу|конечно|хорошо|ладно|окей|ок|согласен|верно|все верно|подтверждаю|подтверди|сохрани|сохранить|да сохрани|да подтверждаю)$/.test(text)){reply('Скажите да для сохранения либо отмена.','confirm_again',true);return}
            const reminder={id:step.editId||makeId(),title:step.title!,dueAt:step.id!,done:false,advanceDays:step.advanceDays||[],announced:[],createdAt:new Date().toISOString(),silentPast:Date.parse(step.id!)<=Date.now()}
            if(step.editId){const index=data.reminders.findIndex(item=>item.id===step.editId);if(index<0)throw new Error('Напоминание уже удалено.');data.reminders[index]=reminder}
            else data.reminders.push(reminder)
            await save(data);pending=null;reply(step.editId?'Напоминание изменено, сэр. Новые текст и время сохранены.':'Напоминание сохранено, сэр.',step.editId?'':'reminder_saved');return
        }
        if(step.kind==='birthday-name'){pending={kind:'birthday-date',title:raw};reply('Назовите день и месяц цифрами: например, 12.04.','birthday_ask_date',true);return}
        if(step.kind==='birthday-date') {
            const match=text.match(/^(\d{1,2})[.\/-](\d{1,2})$/),day=Number(match?.[1]),month=Number(match?.[2]);const test=new Date(2024,month-1,day)
            if(!match||test.getMonth()!==month-1||test.getDate()!==day){reply('Не понял дату. Укажите день и месяц, например 12.04.','birthday_date_invalid',true);return}
            data.birthdays.push({id:makeId(),name:step.title!,day,month});await save(data);pending=null;reply('День рождения сохранён, сэр.','birthday_saved');return
        }
        if(step.kind==='weight') {
            const weight=Number(text.replace(/[^\d,.]/g,'').replace(',','.'))
            if(!Number.isFinite(weight)||weight<20||weight>400){reply('Назовите вес от 20 до 400 килограммов.','weight_invalid',true);return}
            data.training ||= defaultTraining();data.training.profile ||= {heightCm:null,measurements:[]}
            const date=dayKey(new Date());data.training.profile.measurements=data.training.profile.measurements.filter(item=>item.date!==date)
            data.training.profile.measurements.push({id:makeId(),date,weight});await save(data);pending=null;reply(`Вес ${weight} килограммов записан на сегодня, сэр.`,'weight_saved');return
        }
        if (step.kind === 'append-body') {
            const note=data.notes.find(item=>item.id===step.id);if(!note)throw new Error('Заметка больше недоступна.')
            // Preserve rich formatting and images: append escaped plain text to existing HTML.
            note.text += '\n'+raw
            if(note.format==='rich')note.html=(note.html||'')+'<p>'+raw.replace(/[&<>"']/g,char=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[char]!))+'</p>'
            if(note.type==='checklist')note.items=[...(note.items||[]),{id:makeId(),text:raw,done:false}]
            note.updatedAt=new Date().toISOString()
        } else if(step.kind==='note-body'||step.kind==='check-body') {
            data.notes.push({id:makeId(),title:step.title!,text:raw,updatedAt:new Date().toISOString(),type:step.kind==='check-body'?'checklist':'text',format:'plain',...(step.kind==='check-body'?{items:raw.split(/\n|;|следующий пункт/i).filter(item=>item.trim()).map(item=>({id:makeId(),text:item.trim(),done:false}))}:{})})
        } else { pending=null;reply('Это действие пока выполняется в форме Центра.');return }
        await save(data);pending=null;reply('Заметка сохранена, сэр.',step.kind==='append-body'?'note_updated':step.kind==='check-body'?'checklist_saved':'note_saved');return
    }
    if(text.includes('перев')&&text.includes('новост')) {
        navigateCenter('news');const list=get(voiceNews)
        if(!list.length||!newsVoiceAction){reply('Сначала дождитесь загрузки новостной ленты, сэр.','news_wait_loading');return}
        pending={kind:'news',ids:list.map(item=>item.url)}
        const number=spokenNumber(text);if(number){await handleCenterVoice(String(number));return}
        reply(`Какую новость перевести, сэр? Назовите номер от одного до ${list.length}.`,'news_ask_number',true);return
    }
    if(text.includes('таймер')||text.includes('пресет')) {
        navigateCenter('timer')
        if(/пауз|приостанов/.test(text)){pauseTimer();reply('Таймер приостановлен.','timer_pause');return}
        if(/продолж|возобнов/.test(text)){startTimer();reply('Продолжаю отсчёт.','timer_resume');return}
        if(/сброс/.test(text)){resetTimer();reply('Таймер сброшен.','timer_reset');return}
        if(/откр/.test(text)){reply('Открываю таймер, сэр.','timer_open');return}
        const presets=[...timerPresets,...get(customPresets)]
        if(/установ|постав/.test(text)&&!spokenNumber(text)&&!text.includes('пресет')){pending={kind:'duration'};reply('На какое время установить таймер, сэр?','timer_ask_duration',true);return}
        const preset=presets.find(item=>new RegExp('(?:^|\\s)'+clean(item.label).replace(/[.*+?^${}()|[\]\\]/g,'\\$&')+'(?=$|\\s|[.,!?])').test(text))
        if(preset){selectTimer(preset);startTimer();reply(`Запускаю таймер ${preset.label}.`,'timer_start');return}
        const n=spokenNumber(text)
        if(n&&!text.includes('пресет')){const seconds=n*(text.includes('час')?3600:text.includes('секунд')?1:60);if(seconds<=86400&&seconds>0){selectTimer({id:'voice',label:`${n} ${text.includes('час')?'часов':text.includes('секунд')?'секунд':'минут'}`,seconds,icon:'◷'});startTimer();reply('Таймер запущен.','timer_start');return}}
        pending={kind:'preset',ids:presets.map(item=>item.id)};reply('Какой пресет, сэр? '+presets.map((item,i)=>`${i+1}: ${item.label}`).join('; '),'timer_ask_preset',true);return
    }
    if(/привыч/.test(text)&&/отмет|выполн/.test(text)) {
        navigateCenter('habits');const data=await loadCenterData();const habits=data.habits.filter(item=>!item.archivedOn)
        if(!habits.length){reply('Активных привычек пока нет.','habits_empty');return}
        pending={kind:'habit',ids:habits.map(item=>item.id)};const n=spokenNumber(text);if(n){await handleCenterVoice(String(n));return}
        reply('Какую привычку, сэр? '+habits.map((item,i)=>`${i+1}: ${item.title}`).join('; '),'habit_ask_number',true);return
    }
    if(/замет|чек.?лист/.test(text)&&/созда|новую|новый|добав|допол/.test(text)) {
        navigateCenter('notes')
        if(/допол/.test(text)) {const data=await loadCenterData();if(!data.notes.length){reply('Заметок пока нет.','notes_empty');return}pending={kind:'append',ids:data.notes.map(item=>item.id)};reply('Какую заметку дополнить? '+data.notes.map((item,i)=>`${i+1}: ${item.title}`).join('; '),'note_ask_number',true)}
        else {pending={kind:/чек.?лист/.test(text)?'check-title':'note-title'};reply('Как назвать заметку, сэр?',/чек.?лист/.test(text)?'checklist_create':'note_create',true)}return
    }
    if(/напоминан/.test(text)&&/измен|редакт|перенес/.test(text)) {
        navigateCenter('reminders');const data=await loadCenterData(),items=[...data.reminders].sort((a,b)=>a.dueAt.localeCompare(b.dueAt))
        if(!items.length){reply('У вас пока нет напоминаний, сэр.');return}
        pending={kind:'reminder-select',ids:items.map(item=>item.id)};reply('Какое напоминание изменить? Назовите номер на экране.','',true)
        if(spokenNumber(text))await handleCenterVoice(text)
        return
    }
    if(/напоминан|напомни/.test(text)&&/добав|созда|новое|напомни/.test(text)){navigateCenter('reminders');pending={kind:'reminder-title'};reply('О чём напомнить, сэр?','reminder_ask_title',true);return}
    if(/рождения/.test(text)&&/добав|запи/.test(text)){navigateCenter('birthdays');pending={kind:'birthday-name'};reply('Назовите имя, сэр.','birthday_ask_name',true);return}
    if(/город/.test(text)&&/погод|прогноз|измени|установ/.test(text)){navigateCenter('weather');pending={kind:'city'};reply('Назовите город для прогноза, сэр.','weather_city',true);return}
    if(/вес|измерение/.test(text)&&/запи|добав|мой/.test(text)){navigateCenter('workouts');trainingVoiceTab.set('profile');pending={kind:'weight'};reply('Назовите вес в килограммах, сэр.','weight_add',true);return}
    if(/расписание.*сегодня|планы.*сегодня/.test(text)){navigateCenter('calendar');const data=await loadCenterData(),today=dayKey(new Date());reply(data.reminders.filter(item=>!item.done&&dayKey(new Date(item.dueAt))===today).map(item=>item.title).join('; ')||'На сегодня нет активных напоминаний, сэр.','schedule_today');return}
    if(/обнов/.test(text)&&/новост|лент/.test(text)){navigateCenter('news');const action=await waitForNewsAction();await action('refresh');reply('Новости обновлены, сэр.','news_refresh');return}
    if(/останов|выключ/.test(text)&&/озвуч/.test(text)){await invoke('chat_stop_speech');reply('Озвучка остановлена, сэр.','speech_stop');return}
    const pages:[RegExp,string,string][]=[[/новост|лент/,'news','news_open'],[/календар/,'calendar','calendar_open'],[/замет/,'notes','notes_open'],[/диктов|голос в текст/,'voice-text',''],[/напоминан/,'reminders',''],[/рождения/,'birthdays',''],[/погод|прогноз/,'weather','weather_week'],[/привыч/,'habits','habits_open'],[/зарядк/,'workouts','charge_open'],[/параметр/,'workouts','profile_open'],[/трениров/,'workouts','training_open'],[/статистик/,'workouts','stats_open'],[/хранилищ|парол/,'passwords','vault_open'],[/логи|истори/,'request-history',''],[/центр/,'calendar','center_open']]
    const page=pages.find(([pattern])=>pattern.test(text));if(page){
        if(page[1]==='weather'){
            if(!/недел|месяц|7|30|60/.test(text)){pending={kind:'weather-period'};navigateCenter('weather');reply('На какой период показать погоду, сэр?','weather_ask_period',true);return}
            const period=/два месяц|два месяца|60/.test(text)?60:/месяц|30/.test(text)?30:7
            weatherPeriod.set(period);navigateCenter('weather');reply(period>7?`Открываю сезонную оценку на ${period} дней. Это не точный прогноз по дням, сэр.`:'Запрашиваю прогноз на семь дней, сэр.',period===7?'weather_week':period===30?'weather_month':'weather_two_months');return
        }
        if(page[1]==='workouts')trainingVoiceTab.set(/зарядк/.test(text)?'charge':/параметр/.test(text)?'profile':/статистик/.test(text)?'stats':'today')
        navigateCenter(page[1]);reply('Открываю выбранный раздел, сэр.',page[2]);return
    }
    reply('Уточните действие в Центре, сэр. Можно сказать: открыть новости, перевести новость, создать заметку или запустить таймер.')
}

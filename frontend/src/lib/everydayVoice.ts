import { invoke } from '@tauri-apps/api/core'
import { loadCenterData, dayKey, makeId, type CenterData } from './center'
import { getWeekWeather, clothingFor } from './weather'
import { parseReminderDate } from './reminders'

type News = { title:string; summary:string; source:string; url:string; published_at:string }
type Bridge = { reply:(text:string,id?:string,followUp?:boolean)=>void; navigate:(section:string)=>void; save:(data:CenterData)=>Promise<void>; editing:()=>boolean }
let lastNews: News[] = []
let lastFiles: {name:string;path:string}[] = []
const plain = (s:string) => s.replace(/<[^>]*>/g,' ').replace(/\s+/g,' ').trim()
const clean = (s:string) => s.toLowerCase().replace(/ё/g,'е').trim().replace(/[.!?]+$/,'')
export function resetEverydayVoice() { lastNews=[];lastFiles=[] }
export function newsTopic(text:string) {
    return /технолог/.test(text)?'tech':/искусственн|нейросет/.test(text)?'ai':/игр/.test(text)?'games':/наук/.test(text)?'science':/спорт/.test(text)?'sport':/космос/.test(text)?'space':null
}
async function news(text:string):Promise<News[]> {
    const items=await invoke<News[]>('center_get_news',{topic:newsTopic(text)||'all',region:null,source:null})
    return items.filter(item=>!text.includes('сегодня') || dayKey(new Date(item.published_at))===dayKey(new Date())).slice(0,5)
}
export async function handleEverydayVoice(raw:string, bridge:Bridge):Promise<boolean> {
    const text=clean(raw),{reply,navigate,save}=bridge
    if(/^(не |не надо |не нужно )/.test(text))return false
    if(/^(что у меня сегодня|утренний обзор|расскажи о моем дне|доброе утро)$/.test(text)) {
        const data=await loadCenterData(),today=dayKey(new Date())
        const plans=data.reminders.filter(x=>!x.done&&dayKey(new Date(x.dueAt))===today).sort((a,b)=>a.dueAt.localeCompare(b.dueAt))
        const tasks=(data.tasks||[]).filter(x=>!x.done)
        const parts=[plans.length?'Напоминания: '+plans.map(x=>new Date(x.dueAt).toLocaleTimeString('ru-RU',{hour:'2-digit',minute:'2-digit'})+' — '+x.title).join('; '):'На сегодня нет напоминаний.',tasks.length?'Дела: '+tasks.slice(0,5).map(x=>x.text).join('; '):'']
        try {const w=await getWeekWeather();const d=w.days[0];parts.push(`${w.city}: от ${d.low} до ${d.high} градусов. ${clothingFor(d).map(x=>x.text).join('. ')}${w.warning?' Использован ранее полученный прогноз.':''}`)} catch {parts.push('Погода сейчас недоступна.')}
        try {lastNews=await news('сегодня');parts.push(lastNews.length?'Новости: '+lastNews.slice(0,3).map(x=>`${x.title}. Источник: ${x.source}`).join('. '):'Свежих новостей за сегодня не найдено.')}catch{parts.push('Новости сейчас недоступны.')}
        navigate('calendar');reply(parts.filter(Boolean).join('\n'));return true
    }
    if(/зонт|как одеться|что надеть|похолодает/.test(text)) {
        const w=await getWeekWeather(),d=w.days[text.includes('завтра')?1:0]
        if(!d)throw new Error('Прогноз на этот день недоступен.')
        if(text.includes('вечер')) {
            const hours=(w.hours||[]).filter(h=>h.time.startsWith(d.date)&&Number(h.time.slice(11,13))>=18&&Number(h.time.slice(11,13))<=23)
            if(hours.length){const rain=hours.filter(h=>h.rain!==null&&h.rain>=40);const low=Math.round(Math.min(...hours.map(h=>h.temperature))),high=Math.round(Math.max(...hours.map(h=>h.temperature)))
                navigate('weather');reply(`${w.city}, вечером: от ${low} до ${high} градусов. ${rain.length?'Возьмите зонт: вероятность осадков от 40% около '+rain.map(h=>h.time.slice(11,16)).join(', ')+'.':hours.some(h=>h.rain===null)?'Вероятность осадков известна не для всех часов.':'Вероятность осадков ниже 40%, но дождь всё равно возможен.'}${low<10?' Нужна тёплая куртка.':''}${w.warning?' Использован ранее полученный прогноз.':''}`);return true}
        }
        navigate('weather');reply(`${w.city}: от ${d.low} до ${d.high} градусов. ${clothingFor(d).map(x=>x.text).join('. ')}${text.includes('вечер')?' Это прогноз за весь день; точное время дождя в этих данных отсутствует.':''}${w.warning?' Показан ранее полученный прогноз; обновление недоступно.':''}`);return true
    }
    if(/^(что нового|расскажи новости|прочитай новости|новости по теме)/.test(text)) {
        if(/по теме/.test(text)&&!newsTopic(text)){reply('Назовите тему: технологии, нейросети, игры, наука, спорт или космос.');return true}
        lastNews=await news(text);navigate('news')
        reply(lastNews.length?lastNews.map((x,i)=>`${i+1}. ${x.title}. ${plain(x.summary).slice(0,350)} Источник: ${x.source}. ${x.url}`).join('\n'):'За указанный период новости не найдены.');return true
    }
    if(/^подробнее (?:о |про )?(?:новост|перв|втор|трет|\d)/.test(text)) {
        const index=text.match(/\d+/)?.[0];const n=index?Number(index):/перв/.test(text)?1:/втор/.test(text)?2:/трет/.test(text)?3:0
        const item=lastNews[n-1];reply(item?`${item.title}. ${plain(item.summary)} Источник: ${item.source}. Полная статья: ${item.url}`:'Сначала запросите новости, затем скажите, например: подробнее о второй новости.');return true
    }
    const idea=raw.match(/^(?:запомни|запиши)\s+идею\s*:?\s*(.+)$/i)
    if(idea){if(bridge.editing())throw new Error('Сначала сохраните открытую заметку.')
        const data=await loadCenterData(),body=idea[1].trim();if(body.length>12000)throw new Error('Сократите заметку до 12 000 символов.')
        data.notes.push({id:makeId(),title:'Идея: '+body.slice(0,70),text:body,type:'text',format:'plain',updatedAt:new Date().toISOString()});await save(data);navigate('notes');reply('Идея сохранена.','note_saved');return true}
    if(/^(какие идеи|найди заметк|найди иде|что я записывал)/.test(text)) {
        // Word stems, so «про музыку» finds «управление музыкой».
        const data=await loadCenterData();let terms=text.replace(/^(какие идеи|найди заметк[уи]|найди иде[юи]|что я записывал)/,'').replace(/для проекта|я записывал|про |о |на прошлой неделе/g,'').trim().split(/\s+/).filter(Boolean).map(t=>t.length>5?t.slice(0,5):t)
        const weekEnd=new Date();weekEnd.setHours(0,0,0,0);weekEnd.setDate(weekEnd.getDate()-(weekEnd.getDay()+6)%7)
        const weekStart=new Date(weekEnd);weekStart.setDate(weekStart.getDate()-7)
        const lastWeek=text.includes('прошлой неделе')
        const found=data.notes.filter(n=>(!lastWeek||(Date.parse(n.updatedAt)>=weekStart.getTime()&&Date.parse(n.updatedAt)<weekEnd.getTime()))&&(!text.startsWith('какие идеи')||n.title.startsWith('Идея:'))&&terms.every(t=>(n.title+' '+n.text).toLowerCase().includes(t))).slice(-10)
        navigate('notes');reply(found.length?found.map(n=>`${n.title}: ${plain(n.text).slice(0,400)}`).join('\n'):'Подходящих заметок не найдено.');return true
    }
    const reminder=raw.match(/^(через\s+.+?)\s+напомни(?:\s+мне)?\s+(.+)$/i)||raw.match(/^напомни(?:\s+мне)?\s+(через\s+\S+(?:\s+\S+)?\s+(?:минут\S*|час\S*|секунд\S*))\s+(.+)$/i)
    if(reminder){const date=parseReminderDate(reminder[1]);if(!date){reply('Не понял время. Скажите: через сорок минут напомни выключить духовку.');return true}
        const data=await loadCenterData();data.reminders.push({id:makeId(),title:reminder[2].trim(),dueAt:date.toISOString(),done:false,createdAt:new Date().toISOString(),advanceDays:[],announced:[]});await save(data);navigate('reminders');reply(`Напомню ${date.toLocaleString('ru-RU')}: ${reminder[2]}. Джарвис должен оставаться запущенным.`);return true}
    const add=raw.match(/^добавь\s+(.+?)\s+в\s+(покупки|список дел)$/i)
    const read=/^(прочитай|покажи|что в)\s+(?:список |списке )?(покупок|покупки|дел)$/.test(text)
    const done=raw.match(/^отметь\s+(.+?)\s+(?:купленным|купленной|выполненным|выполненной)$/i)
    if(add||read||done){if(bridge.editing()&&!read)throw new Error('Сначала сохраните открытую заметку.')
        const data=await loadCenterData();const title=(add?clean(add[2])==='список дел':read?text.endsWith('дел'):/выполненн/.test(text))?'Дела':'Покупки'
        let note=data.notes.find(n=>n.type==='checklist'&&n.title===title)
        if(add){if(!note){note={id:makeId(),title,text:'',type:'checklist',format:'plain',items:[],updatedAt:new Date().toISOString()};data.notes.push(note)}
            const items=add[1].split(/\s+и\s+|[,;]/).map(x=>x.trim()).filter(Boolean);note.items||=[];for(const item of items)if(!note.items.some(x=>clean(x.text)===clean(item)&&!x.done))note.items.push({id:makeId(),text:item,done:false})
        }else if(done){const item=note?.items?.find(x=>clean(x.text)===clean(done[1]));if(!item){reply('Такой пункт не найден. Назовите его точно как в списке.');return true}item.done=true}
        if(note&&(add||done)){note.text=(note.items||[]).map(x=>x.text).join('\n');note.updatedAt=new Date().toISOString();await save(data)}
        navigate('notes');reply(read?(note?.items?.filter(x=>!x.done).map(x=>x.text).join(', ')||'Список пуст.'):'Список сохранён.');return true
    }
    if(/^(найди|поищи) (?:мой |мои )?(файл|документ)/.test(text)) {
        const query=text.replace(/^(найди|поищи) (?:мой |мои )?(?:файлы?|документы?)\s*/,'').replace(/с курсовой/,'курсов').replace(/который я вчера редактировал|вчера|который я редактировал/g,'').replace(/[,.:;]+/g,' ').trim()
        if(query.length<2){reply('Уточните часть имени файла, например: найди документ с курсовой.');return true}
        const result=await invoke<{files:{name:string;path:string}[];limited:boolean}>('everyday_find_files',{query,yesterday:text.includes('вчера')});lastFiles=result.files
        reply((lastFiles.length?lastFiles.map((f,i)=>`${i+1}. ${f.name}\n${f.path}`).join('\n'):'Файлы не найдены.')+' Поиск по имени в Документах, Загрузках и на Рабочем столе.'+(result.limited?' Просмотрена только часть папок.':'')+' Скажите «покажи файл номер один», чтобы открыть его папку.');return true
    }
    if(/^покажи файл номер /.test(text)){const words=['один','два','три','четыре','пять'];const n=Number(text.match(/\d+/)?.[0])||words.indexOf(text.split(' ').at(-1)!)+1;const f=lastFiles[n-1];if(!f){reply('Сначала найдите файл и назовите номер результата.');return true}await invoke('show_in_folder',{path:f.path});reply('Открываю папку найденного файла.');return true}
    if(/^(переведи это|объясни проще|сократи это|исправь ошибки в тексте)$/.test(text)) {
        // The selection of the active window is copied first; otherwise the clipboard is used as is.
        const {text:content,from_selection}=await invoke<{text:string;from_selection:boolean}>('everyday_clipboard_text')
        if(!content.trim()){reply('Выделите текст в любом окне или скопируйте его через Ctrl+C и повторите команду.');return true}
        if(content.length>12000)throw new Error('Выделите фрагмент не длиннее 12 000 символов.')
        const task={'переведи это':'Переведи текст на русский. Если он на русском — на английский.','объясни проще':'Объясни этот текст простыми словами.','сократи это':'Сократи текст, сохранив главное.'}[text]??'Исправь орфографические, грамматические и пунктуационные ошибки, сохранив смысл.'
        const answer=await invoke<{content:string}>('agent_run',{kind:'documents',task,context:content});reply(`Результат для ${from_selection?'выделенного текста':'текста из буфера обмена'}:\n${answer.content}`);return true
    }
    if(/^(включи|запусти) режим работы$/.test(text)){reply(await invoke<string>('everyday_work_mode'));return true}
    const login=text.match(/^открой вход в (.+)$/)
    if(login){reply(await invoke<string>('everyday_open_login',{site:login[1]}));return true}
    return false
}

const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict'),{Module}=require('node:module'),ts=require('typescript')
function load(file,stubs){const filename=path.resolve(file),mod=new Module(filename,module);mod.filename=filename;mod.paths=module.paths;mod.require=name=>stubs[name]||require(name);mod._compile(ts.transpileModule(fs.readFileSync(file,'utf8'),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022}}).outputText,filename);return mod.exports}
async function main(){
 let db={notes:[],reminders:[],tasks:[],birthdays:[],habits:[]},writes=0,last='',editing=false,failSave=false,failWeather=false
 const calls=[],core={loadCenterData:async()=>structuredClone(db),makeId:()=>crypto.randomUUID(),dayKey:d=>`${d.getFullYear()}-${String(d.getMonth()+1).padStart(2,'0')}-${String(d.getDate()).padStart(2,'0')}`}
 const reminders=load('src/lib/reminders.ts',{})
 const invoke=async(name,args)=>{calls.push({name,args});if(name==='center_get_news')return [{title:'Новый процессор',summary:'Детали процессора',source:'Источник',url:'https://example.com/news',published_at:new Date().toISOString()}];if(name==='everyday_work_mode')return 'VS Code запущен.';if(name==='everyday_find_files')return {files:[{name:'Курсовая.docx',path:'C:/fixtures/Курсовая.docx'}],limited:false};if(name==='everyday_clipboard_text')return {text:'Hello',from_selection:true};if(name==='everyday_open_login')return 'Открываю вход в GitHub.';if(name==='agent_run')return {content:'Привет'};return undefined}
 const voice=load('src/lib/everydayVoice.ts',{'@tauri-apps/api/core':{invoke},'./center':core,'./reminders':reminders,'./weather':{getWeekWeather:async()=>{if(failWeather)throw Error('offline');return {city:'Минск',days:[{date:'2026-11-06',low:4,high:10}],hours:[{time:'2026-11-06T18:00',temperature:7,rain:80},{time:'2026-11-06T21:00',temperature:4,rain:20}]}},clothingFor:()=>[{text:'Возьмите зонт'}]}})
 const bridge={reply:text=>last=text,navigate:()=>{},editing:()=>editing,save:async data=>{if(failSave)throw Error('disk full');db=data;writes++}}
 const say=text=>voice.handleEverydayVoice(text,bridge)
 await say('Запомни идею: добавить управление музыкой');assert.equal(db.notes[0].text,'добавить управление музыкой')
 await say('Какие идеи для проекта я записывал?');assert.match(last,/управление музыкой/)
 await say('Добавь молоко и хлеб в покупки');assert.equal(db.notes[1].items.length,2)
 await say('Добавь молоко в покупки');assert.equal(db.notes[1].items.length,2)
 await say('Отметь молоко купленным');assert.equal(db.notes[1].items[0].done,true)
 await say('Прочитай список покупок');assert.equal(last,'хлеб')
 const before=Date.now();await say('Через сорок минут напомни выключить духовку');assert.equal(db.reminders[0].title,'выключить духовку');assert.ok(Math.abs(Date.parse(db.reminders[0].dueAt)-before-40*60000)<1000)
 assert.equal(reminders.parseReminderDate('через двадцать пять минут',new Date(0)).getTime(),25*60000)
 assert.equal(reminders.parseReminderDate('через неизвестно минут'),null)
 await say('Что нового в технологиях за сегодня?');assert.equal(calls.at(-1).args.topic,'tech');assert.match(last,/https:\/\/example.com/)
 await say('Подробнее о первой новости');assert.match(last,/Детали процессора/)
 await say('Вечером нужен зонт?');assert.match(last,/зонт/);assert.match(last,/18:00/);assert.match(last,/от 4 до 7/)
 failWeather=true;const newsCalls=calls.length;await say('Что у меня сегодня?');assert.match(last,/выключить духовку/);assert.doesNotMatch(last,/Погода|Новости|градус/);assert.equal(calls.length,newsCalls)
 await say('Найди документ с курсовой, который я вчера редактировал');assert.equal(calls.at(-1).args.yesterday,true);assert.equal(calls.at(-1).args.query,'курсов')
 await say('Покажи файл номер один');assert.equal(calls.at(-1).name,'show_in_folder')
 await say('Переведи это');assert.match(last,/Привет/);assert.match(last,/выделенного/)
 await say('Включи режим работы');assert.equal(calls.at(-1).name,'everyday_work_mode')
 await say('Открой вход в github');assert.equal(calls.at(-1).name,'everyday_open_login');assert.equal(calls.at(-1).args.site,'github')
 await say('Какие идеи про музыку');assert.match(last,/управление музыкой/)
 const count=writes;assert.equal(await say('не запомни идею удалить все'),false);assert.equal(writes,count)
 editing=true;await assert.rejects(()=>say('Запомни идею тест'),/сохраните/);assert.equal(writes,count);editing=false
 failSave=true;const old=last;await assert.rejects(()=>say('Запомни идею тест'),/disk full/);assert.equal(last,old)
 console.log('Everyday voice: notes, lists, reminders, source-backed news, weather failure, files, text, work mode, negation and save failures passed')
}
main().catch(e=>{console.error(e);process.exit(1)})

// node --conditions=browser tests/agents-ui.cjs <test dependency directory>
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict'),{createRequire,Module}=require('node:module')
const deps=createRequire(path.resolve(process.argv[2],'package.json')),{JSDOM}=deps('jsdom'),dom=new JSDOM('<body></body>',{url:'http://localhost'})
for(const name of ['window','document','navigator','HTMLElement','Element','DOMParser','Node','Event','MouseEvent','CustomEvent','getComputedStyle','File'])Object.defineProperty(globalThis,name,{value:dom.window[name],configurable:true})
File.prototype.arrayBuffer=async function(){return new Uint8Array([65,66]).buffer}
const {screen,waitFor}=deps('@testing-library/dom'),userEvent=deps('@testing-library/user-event').default,ts=require('typescript'),{compile,preprocess}=require('svelte/compiler')
async function main(){
 const capabilities=JSON.parse(fs.readFileSync('../crates/jarvis-gui/capabilities/default.json','utf8')).permissions
 for(const permission of ['dialog:allow-save','fs:allow-write-text-file','fs:allow-write-file'])assert.ok(capabilities.includes(permission),`Real export requires ${permission}`)
 const svelte=await import('svelte'),{writable,get}=await import('svelte/store')
 const calls=[],exports=[],routes=[],outlook=writable({busy:false,draft:{to:'existing@example.com',subject:'Существующее',body:'Не потерять'}}),centerSection=writable('calendar')
 let fail=false,chainMode='success',pendingChain
 const chainReply=args=>{const mail=['document_mail','reply_mail','weekly_report'].includes(args.scenario)&&args.step>0;return {content:mail?'JSON result '+args.step:'Итог этапа '+args.step,subject:mail?'Проверенное письмо':null,body:mail?'Подтверждённые данные':null,review:mail&&args.step===2?'Уточните срок':null,report:args.scenario==='weekly_report'&&args.step>0?'Недельный отчёт: выполнено 2 задачи':null,provider:'local',model:'test'}}
 const api={invoke:async(action,args)=>{calls.push({action,args});if(action==='chat_get_config')return {provider:'local',local_model:'test'};if(action==='agent_read_attachment'){if(args.name==='broken.docx')throw Error('Повреждённый документ');return {name:args.name,text:args.name.startsWith('big')?'a'.repeat(8000):'Данные документа: цена 10',truncated:args.name==='long.txt',warning:''}}if(action==='agent_run'){if(fail)throw Error('Модель недоступна');return {content:args.kind==='mail'?'JSON':'1. Уточнить срок',subject:args.kind==='mail'?'Запрос цены':null,body:args.kind==='mail'?'Пришлите прайс <script>bad()</script>':null,provider:'local',model:'test'}}throw Error(action)}}
 const filename=path.resolve('src/routes/agents/index.svelte'),processed=await preprocess(fs.readFileSync(filename,'utf8'),{script:({content})=>({code:ts.transpileModule(content,{compilerOptions:{module:ts.ModuleKind.ESNext,target:ts.ScriptTarget.ES2022,verbatimModuleSyntax:true}}).outputText})},{filename})
 const oldInvoke=api.invoke;api.invoke=async(action,args)=>{if(action!=='agent_chain_step')return oldInvoke(action,args);calls.push({action,args});if(chainMode==='pending')return new Promise(resolve=>pendingChain=()=>resolve(chainReply(args)));if(chainMode==='fail'&&args.step===1)throw Error('Этап недоступен');const reply=chainReply(args);if(args.step===0)reply.content='## Факты источника\n- **Дата подтверждена**\n<script>bad()</script>';if(reply.review)reply.review='## Нужно уточнить\n- **Срок ответа**';return reply}
 const chainInvoke=api.invoke;api.invoke=async(action,args)=>{if(action==='agent_export_docx'){calls.push({action,args});return [80,75,3,4]}return chainInvoke(action,args)}
 const chainsFile=path.resolve('src/lib/agentChains.ts'),chainsModule=new Module(chainsFile,module);chainsModule.paths=module.paths;chainsModule.require=name=>name==='@tauri-apps/api/core'?api:require(name);chainsModule._compile(ts.transpileModule(fs.readFileSync(chainsFile,'utf8'),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022}}).outputText,chainsFile)
 const stubs={svelte,'@fontsource-variable/manrope/wght.css':{},'@roxi/routify':{goto:writable(route=>routes.push(route))},'@tauri-apps/api/core':api,'@tauri-apps/plugin-dialog':{save:async()=>'/chosen/result.txt'},'@tauri-apps/plugin-fs':{writeTextFile:async(p,text)=>exports.push(text)},'@/lib/outlook':{outlook,editDraft:patch=>outlook.update(state=>({...state,draft:{...state.draft,...patch}}))},'@/lib/centerVoice':{centerSection}}
 stubs['@/lib/agentChains']=chainsModule.exports
 stubs['@/lib/outlook'].openMailDraft=async()=>{calls.push({action:'open_native_draft'})}
 const formattingFile=path.resolve('src/lib/noteFormatting.ts'),formattingModule=new Module(formattingFile,module);formattingModule._compile(ts.transpileModule(fs.readFileSync(formattingFile,'utf8'),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022}}).outputText,formattingFile);stubs['@/lib/noteFormatting']=formattingModule.exports
 const exportFile=path.resolve('src/lib/agentExport.ts'),exportModule=new Module(exportFile,module);exportModule.require=name=>name==='./noteFormatting'?formattingModule.exports:require(name);exportModule._compile(ts.transpileModule(fs.readFileSync(exportFile,'utf8'),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022}}).outputText,exportFile);stubs['@/lib/agentExport']=exportModule.exports
 stubs['@tauri-apps/plugin-dialog'].save=async options=>'/chosen/'+options.defaultPath
 stubs['@tauri-apps/plugin-fs'].writeFile=async(p,bytes)=>exports.push({p,bytes})
 const mod=new Module(filename,module);mod.paths=module.paths;mod.require=name=>stubs[name]||require(name);mod._compile(ts.transpileModule(compile(processed.code,{filename,generate:'dom',css:'external'}).js.code,{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022}}).outputText,filename)
 const instance=new mod.exports.default({target:document.body}),user=userEvent.setup({document})
 assert.equal(screen.getByRole('button',{name:'Подготовить результат →'}).disabled,true)
 await user.click(screen.getByRole('button',{name:'Вставить пример поручения'}));await user.type(screen.getByLabelText(/Контекст и исходные/),'Поставщик: тест')
 await user.upload(screen.getByLabelText('Прикрепить документы'),new File(['data'],'sample.docx'));await screen.findByRole('button',{name:'Удалить sample.docx'})
 await user.click(screen.getByRole('button',{name:'Подготовить результат →'}));await screen.findByLabelText('Тема письма')
 assert.match(calls.find(c=>c.action==='agent_run').args.context,/Поставщик: тест/);assert.match(calls.find(c=>c.action==='agent_run').args.context,/Документ: sample.docx/);assert.match(calls.find(c=>c.action==='agent_run').args.context,/цена 10/)
 await user.click(screen.getByRole('button',{name:'Удалить sample.docx'}));assert.equal(screen.queryByRole('button',{name:'Удалить sample.docx'}),null)
 await user.upload(screen.getByLabelText('Прикрепить документы'),new File(['data'],'long.txt'));await screen.findByRole('button',{name:'Удалить long.txt'});assert.equal(screen.getByRole('button',{name:'Подготовить результат →'}).disabled,true)
 await user.click(screen.getByLabelText('Использовать только извлечённые фрагменты сокращённых документов'));assert.equal(screen.getByRole('button',{name:'Подготовить результат →'}).disabled,false)
 await user.click(screen.getByRole('button',{name:'Удалить long.txt'}));await user.upload(screen.getByLabelText('Прикрепить документы'),new File(['bad'],'broken.docx'));await screen.findByRole('alert');assert.equal(screen.queryByRole('button',{name:'Удалить broken.docx'}),null)
 await user.upload(screen.getByLabelText('Прикрепить документы'),[new File(['data'],'big1.txt'),new File(['data'],'big2.txt')]);await screen.findByRole('button',{name:'Удалить big2.txt'});assert.equal(screen.getByRole('button',{name:'Подготовить результат →'}).disabled,true,'Oversized combined context must not be silently sent')
 await user.click(screen.getByRole('button',{name:'Удалить big1.txt'}));await user.click(screen.getByRole('button',{name:'Удалить big2.txt'}));assert.equal(screen.getByRole('button',{name:'Подготовить результат →'}).disabled,false)
 assert.equal(screen.getByLabelText('Текст письма').value,'Пришлите прайс <script>bad()</script>');assert.equal(document.querySelector('script'),null)
 await user.clear(screen.getByLabelText('Текст письма'));await user.type(screen.getByLabelText('Текст письма'),'## Встреча\n\n**Важно**\n- Подтвердите участие\n<script>bad()</script>\n<img src="https://example.com/tracker">')
 assert.ok(screen.getByRole('heading',{name:'Встреча'}));assert.ok(screen.getByText('Важно').closest('strong'));assert.ok(screen.getByText(/• Подтвердите участие/));assert.equal(document.querySelector('script, img'),null)
 await user.type(screen.getByLabelText('Текст письма'),'\nИзменение');assert.ok(screen.getByText('Изменение'))
 await user.type(screen.getByLabelText('Получатели'),'test@jarvis.test')
 assert.equal(screen.getByRole('button',{name:'Передать черновик в Outlook →'}).disabled,true)
 await user.click(screen.getByLabelText('Заменить существующий черновик JARVIS этим письмом'));await user.click(screen.getByRole('button',{name:'Передать черновик в Outlook →'}))
 assert.equal(get(outlook).draft.to,'test@jarvis.test');assert.equal(get(centerSection),'outlook');assert.deepEqual(routes,['/center'])
 await user.click(screen.getByRole('button',{name:'Экспортировать .txt'}));await waitFor(()=>assert.equal(exports.length,1));assert.match(exports[0],/test@jarvis.test/)
 await user.click(screen.getByRole('button',{name:/Задачи/}));await user.click(screen.getByRole('button',{name:'Вставить пример поручения'}));await user.click(screen.getByRole('button',{name:'Подготовить результат →'}));await screen.findByLabelText('Редактируемый текст')
 assert.equal(screen.getByLabelText('Редактируемый текст').value,'1. Уточнить срок')
 fail=true;await user.click(screen.getByRole('button',{name:'Подготовить результат →'}));await screen.findByRole('alert');assert.equal(screen.getByLabelText('Редактируемый текст').value,'1. Уточнить срок')
 fail=false;await user.click(screen.getByRole('button',{name:/Саб-агенты/}));assert.equal(deps('@testing-library/dom').within(screen.getByLabelText('Сценарий саб-агентов')).getAllByRole('option').length,6)
 await user.type(screen.getByLabelText(/Контекст и исходные/),'Исходные подтверждённые данные')
 for(const scenario of chainsModule.exports.agentScenarios){
  await user.selectOptions(screen.getByLabelText('Сценарий саб-агентов'),scenario.id);await user.click(screen.getByRole('button',{name:'Вставить пример поручения'}))
  assert.equal(screen.getByRole('button',{name:'Подготовить результат →'}).disabled,true,'Chain costs must be acknowledged')
  await user.click(screen.getByLabelText('Разрешаю до трёх запросов с этими данными; в облаке возможны расходы'))
  const start=calls.length;await user.click(screen.getByRole('button',{name:'Подготовить результат →'}));await waitFor(()=>assert.equal(screen.getByRole('button',{name:'Подготовить результат →'}).disabled,false))
  const chainCalls=calls.slice(start).filter(c=>c.action==='agent_chain_step');assert.equal(chainCalls.length,3);assert.deepEqual(chainCalls.map(c=>c.args.previous.length),[0,1,2]);assert.equal(screen.getAllByText('Готово',{exact:true}).length,3)
  await user.click(screen.getByText('Результат этапа 1'));assert.ok(screen.getByRole('heading',{name:'Факты источника'}));assert.ok(screen.getByText('Дата подтверждена').closest('strong'));assert.equal(document.querySelector('script, img'),null)
  if(scenario.mail){const resultQueries=deps('@testing-library/dom').within(screen.getByRole('region',{name:'Результат агента'}));assert.ok(resultQueries.getByRole('heading',{name:'Нужно уточнить'}));assert.ok(resultQueries.getByText('Срок ответа').closest('strong'))}
  if(scenario.mail){assert.equal(screen.getByRole('button',{name:'Передать черновик в Outlook →'}).disabled,true,'Final review required')}
  if(scenario.id==='weekly_report'){
   assert.match(screen.getByLabelText('Недельный отчёт').value,/выполнено 2 задачи/)
   await user.click(screen.getByLabelText('Я проверил итог, замечания и адресатов перед передачей в Outlook'));await user.click(screen.getByLabelText('Заменить существующий черновик JARVIS этим письмом'));assert.equal(screen.getByRole('button',{name:'Передать черновик в Outlook →'}).disabled,false)
   await user.type(screen.getByLabelText('Тема письма'),'!');assert.equal(screen.getByRole('button',{name:'Передать черновик в Outlook →'}).disabled,true,'Editing invalidates approval')
   await user.click(screen.getByRole('button',{name:'Экспортировать .txt'}));await waitFor(()=>assert.equal(exports.length,2));assert.match(exports[1],/Недельный отчёт:/);assert.match(exports[1],/Замечания проверяющего:/)
  }
 }
 chainMode='fail';let before=calls.length;await user.click(screen.getByRole('button',{name:'Подготовить результат →'}));await screen.findByRole('alert');assert.equal(calls.slice(before).filter(c=>c.action==='agent_chain_step').length,2);assert.equal(screen.getByLabelText('Редактируемый текст').value,'Итог этапа 2')
 chainMode='pending';before=calls.length;await user.click(screen.getByRole('button',{name:'Подготовить результат →'}));await screen.findByRole('button',{name:'Остановить цепочку'});await waitFor(()=>assert.equal(typeof pendingChain,'function'));await user.click(screen.getByRole('button',{name:'Остановить цепочку'}));pendingChain();await screen.findByText(/Цепочка остановлена\. Следующие этапы/);assert.equal(calls.slice(before).filter(c=>c.action==='agent_chain_step').length,1);assert.equal(screen.getByLabelText('Редактируемый текст').value,'Итог этапа 2')
 assert.ok(calls.some(c=>c.action==='open_native_draft'),'Transfer opens the native draft')
 const tableText='## План\n| Задача | Исполнитель |\n|---|---|\n| **Проверить договор** | Иван |\n| <script>bad()</script> | Не назначен |'
 await user.clear(screen.getByLabelText('Редактируемый текст'));await user.type(screen.getByLabelText('Редактируемый текст'),tableText)
 assert.equal(screen.getAllByRole('columnheader').length,2);assert.ok(screen.getByRole('cell',{name:'Проверить договор'}));assert.equal(document.querySelector('script'),null)
 const blocks=exportModule.exports.exportBlocks(tableText);assert.equal(blocks[1].kind,'table');assert.equal(blocks[1].rows[1][0][0].bold,true)
 for(const format of ['md','html','docx']) {
  await user.selectOptions(screen.getByLabelText('Формат экспорта'),format);await user.click(screen.getByRole('button',{name:`Экспортировать .${format}`}));await screen.findByText('Результат экспортирован.')
  if(format==='html'){assert.match(exports.at(-1),/<table>/);assert.match(exports.at(-1),/&lt;script&gt;/)}
  if(format==='docx'){assert.ok(exports.at(-1).bytes instanceof Uint8Array);assert.match(exports.at(-1).p,/\.docx$/)}
 }
 assert.ok(calls.every(c=>['chat_get_config','agent_run','agent_read_attachment','agent_chain_step','open_native_draft','agent_export_docx'].includes(c.action)),'No tool execution, mail reading or send calls')
 instance.$destroy();console.log('Agents: categories, generation, safe rendering, draft overwrite confirmation, export, mail handoff and error preservation passed')
}
main().then(()=>process.exit(0)).catch(e=>{console.error(e);process.exit(1)})

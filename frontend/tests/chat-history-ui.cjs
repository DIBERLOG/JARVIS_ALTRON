// node --conditions=browser tests/chat-history-ui.cjs <test dependency directory>
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict'),{createRequire,Module}=require('node:module')
const deps=createRequire(path.resolve(process.argv[2],'package.json')),{JSDOM}=deps('jsdom'),dom=new JSDOM('<body></body>',{url:'http://localhost'})
for(const name of ['window','document','navigator','HTMLElement','Node','Event','MouseEvent','getComputedStyle'])Object.defineProperty(globalThis,name,{value:dom.window[name],configurable:true})
const {screen,waitFor,within}=deps('@testing-library/dom'),userEvent=deps('@testing-library/user-event').default,ts=require('typescript'),{compile,preprocess}=require('svelte/compiler')
function load(filename,code,stubs){const mod=new Module(filename,module);mod.paths=module.paths;mod.require=name=>stubs[name]||require(name);mod._compile(ts.transpileModule(code,{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022}}).outputText,filename);return mod.exports}
async function component(relative,stubs){const filename=path.resolve(relative),processed=await preprocess(fs.readFileSync(filename,'utf8'),{script:({content})=>({code:ts.transpileModule(content,{compilerOptions:{module:ts.ModuleKind.ESNext,target:ts.ScriptTarget.ES2022,verbatimModuleSyntax:true}}).outputText})},{filename});return load(filename,compile(processed.code,{filename,generate:'dom',css:'external'}).js.code,stubs).default}
async function main(){
 const svelte=await import('svelte');let raw='',exports=[],calls=[],config={provider:'local',local_model:'qwen3:8b',deepseek_model:'deepseek-flash',deepseek_configured:false,openai_model:'gpt-4.1-mini',openai_configured:false,speak_responses:false,personality:'jarvis'}
 const api={invoke:async(action,args)=>{
  calls.push({action,args})
  if(action==='chat_get_config')return {...config}
  if(action==='db_read')return raw
  if(action==='db_write'){raw=args.val;return true}
  if(action==='db_write_many'){const map=Object.fromEntries(args.entries.map(e=>[e.key,e.val]));config.provider=map.chat_provider;config.openai_model=map.openai_chat_model;if(map.api_key__openai)config.openai_configured=true;return}
  if(action==='chat_get_news')return []
  if(action==='chat_is_speaking')return false
  if(action==='chat_send')return {content:'Ответ на '+args.clientMessages.at(-1).content}
  throw Error(action)
 }}
 const history=load(path.resolve('src/lib/chatHistory.ts'),fs.readFileSync('src/lib/chatHistory.ts','utf8'),{'@tauri-apps/api/core':api})
 assert.throws(()=>history.parseChatHistory('{}'));assert.throws(()=>history.parseChatHistory('[{"id":"x"}]'))
 const exported=JSON.parse(history.chatExport([{id:'x',title:'Тест',updatedAt:new Date().toISOString(),messages:[{role:'user',content:'Текст',secret:'DO_NOT_EXPORT'}],apiKey:'DO_NOT_EXPORT'}]))
 assert.ok(!JSON.stringify(exported).includes('DO_NOT_EXPORT'))
 const provider=await component('src/components/ChatProviderSettings.svelte',{svelte,'@tauri-apps/api/core':api})
 let instance=new provider({target:document.body});const user=userEvent.setup({document})
 await waitFor(()=>assert.equal(screen.getByLabelText('Источник ответов').matches(':disabled'),false))
 await user.selectOptions(screen.getByLabelText('Источник ответов'),'openai')
 await user.type(screen.getByLabelText('API-ключ OpenAI'),'fake-key-for-test')
 await user.click(screen.getByRole('button',{name:'Сохранить настройки моделей'}))
 await screen.findByText('Настройки моделей сохранены')
 assert.equal(config.provider,'openai');assert.equal(config.openai_configured,true);assert.equal(screen.getByLabelText('API-ключ OpenAI').value,'')
 const writesBefore=calls.filter(c=>c.action==='db_write_many').length
 await user.click(screen.getByRole('button',{name:'Сохранить настройки моделей'}));await waitFor(()=>assert.equal(calls.filter(c=>c.action==='db_write_many').length,writesBefore+1))
 assert.ok(!calls.filter(c=>c.action==='db_write_many').at(-1).args.entries.some(e=>e.key==='api_key__openai'),'empty field must not erase saved key')
 instance.$destroy();document.body.innerHTML=''
 const empty=load(path.resolve('empty.svelte'),compile('<span></span>',{generate:'dom'}).js.code,{svelte})
 const Chat=await component('src/routes/chat/index.svelte',{svelte,'@tauri-apps/api/core':api,'@tauri-apps/plugin-shell':{open:async()=>{}},'@/lib/requestHistory':{recordRequest(){}},'@/lib/chatHistory':history,'@/components/LoadingSpinner.svelte':empty,'@/components/ChatProviderSettings.svelte':{default:provider},'@tauri-apps/plugin-dialog':{save:async()=>'/test/chat.json'},'@tauri-apps/plugin-fs':{writeTextFile:async(file,text)=>exports.push(JSON.parse(text))}})
 instance=new Chat({target:document.body})
 await waitFor(()=>assert.equal(screen.getByRole('button',{name:'Отправить'}).disabled,false))
 await user.type(screen.getByLabelText('Сообщение ассистенту'),'Первая беседа');await user.click(screen.getByRole('button',{name:'Отправить'}))
 await screen.findByText('Ответ на Первая беседа');assert.equal(JSON.parse(raw).length,1)
 await user.click(screen.getByRole('button',{name:'＋ Новый чат'}));await user.type(screen.getByLabelText('Сообщение ассистенту'),'Вторая беседа');await user.click(screen.getByRole('button',{name:'Отправить'}))
 await screen.findByText('Ответ на Вторая беседа');assert.equal(JSON.parse(raw).length,2)
 const sent=calls.filter(c=>c.action==='chat_send').at(-1);assert.equal(sent.args.clientMessages.length,1,'new chat must not receive other chat context')
 await user.click(within(screen.getByRole('navigation',{name:'Сохранённые чаты'})).getByRole('button',{name:/Первая беседа/}));assert.ok(screen.getByText('Ответ на Первая беседа'))
 await user.click(screen.getByRole('button',{name:'↓ Экспорт текущего чата'}));await waitFor(()=>assert.equal(exports.length,1));assert.equal(exports[0].chats.length,1)
 await user.click(screen.getByRole('button',{name:'↓ Экспорт всех чатов'}));await waitFor(()=>assert.equal(exports.length,2));assert.equal(exports[1].chats.length,2)
 instance.$destroy();document.body.innerHTML='';instance=new Chat({target:document.body});await screen.findByText('Ответ на Вторая беседа')
 instance.$destroy();document.body.innerHTML=''
 raw=JSON.stringify([{id:'long',title:'Длинная беседа',updatedAt:new Date().toISOString(),messages:Array.from({length:24},(_,i)=>({role:i%2?'assistant':'user',content:`Сообщение ${i}`}))}])
 instance=new Chat({target:document.body});await screen.findByText('Сообщение 23');await user.type(screen.getByLabelText('Сообщение ассистенту'),'Продолжение');await user.click(screen.getByRole('button',{name:'Отправить'}));await screen.findByText('Ответ на Продолжение')
 assert.equal(calls.filter(c=>c.action==='chat_send').at(-1).args.clientMessages.length,20);assert.equal(JSON.parse(raw)[0].messages.length,26,'full transcript must not be truncated')
 instance.$destroy();document.body.innerHTML=''
 raw='{broken';instance=new Chat({target:document.body});await screen.findByText(/История не загружена/);assert.equal(screen.getByRole('button',{name:'Отправить'}).disabled,true);instance.$destroy()
 console.log('OpenAI settings, key isolation, persistent chat sidebar, separate contexts, exports and corrupt-history protection passed')
}
main().then(()=>process.exit(0)).catch(e=>{console.error(e);process.exit(1)})

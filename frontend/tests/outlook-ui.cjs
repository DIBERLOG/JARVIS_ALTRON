// Run: node --conditions=browser tests/outlook-ui.cjs <test dependency directory>
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict'),{createRequire,Module}=require('node:module')
const deps=createRequire(path.resolve(process.argv[2],'package.json')),{JSDOM}=deps('jsdom')
const dom=new JSDOM('<!doctype html><body></body>',{url:'http://localhost'})
for(const name of ['window','document','navigator','HTMLElement','Node','Event','MouseEvent','getComputedStyle','localStorage'])Object.defineProperty(globalThis,name,{value:dom.window[name],configurable:true})
const {screen,waitFor}=deps('@testing-library/dom'),userEvent=deps('@testing-library/user-event').default
const ts=require('typescript'),{compile,preprocess}=require('svelte/compiler')
function load(filename,source,stubs){const mod=new Module(filename,module);mod.filename=filename;mod.paths=module.paths;mod.require=name=>stubs[name]||require(name);mod._compile(ts.transpileModule(source,{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022}}).outputText,filename);return mod.exports}
async function main(){
    const svelte=await import('svelte'),store=await import('svelte/store'),calls=[],opened=[]
    let connected=false,localSupported=true
    const invoke=async(command,args)=>{
        assert.equal(command,'outlook_request');calls.push(args)
        if(args.action==='status')return {connected,pending:false,localSupported,account:connected?{email:'me@example.com'}:{}}
        if(args.action==='connect')return {code:'ABCD1234',interval:60}
        if(args.action==='connect_local'){connected=true;return {connected:true,provider:'local',account:{email:'angel@jarvis.test'}}}
        if(args.action==='inbox')return {messages:[],hasMore:false}
        if(args.action==='prepare_send')return {ticket:'TICKET'}
        if(args.action==='send')return {accepted:true}
        return {}
    }
    const filename=path.resolve('src/components/CenterOutlook.svelte'),mailFile=path.resolve('src/lib/outlook.ts')
    const mail=load(mailFile,fs.readFileSync(mailFile,'utf8'),{'svelte/store':store,'@tauri-apps/api/core':{invoke}})
    const processed=await preprocess(fs.readFileSync(filename,'utf8'),{script:({content,attributes})=>attributes.lang==='ts'?{code:ts.transpileModule(content,{compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.ESNext,verbatimModuleSyntax:true}}).outputText}:undefined},{filename})
    const compiled=compile(processed.code,{filename,generate:'dom',css:'external'})
    const Component=load(filename,compiled.js.code,{svelte,'@/lib/outlook':mail,'@tauri-apps/plugin-shell':{open:async url=>opened.push(url)}}).default
    const user=userEvent.setup({document}),component=new Component({target:document.body})
    assert.ok(await screen.findByRole('region',{name:'Классический Outlook / Kerio'}))
    assert.ok(screen.getByRole('region',{name:'Microsoft 365 / Outlook.com'}))
    assert.ok(screen.getByText('Экспериментально'))
    assert.ok(screen.getByText(/не проверено на реальном аккаунте/))
    await user.click(screen.getByText('Как подключить Kerio — пошагово'))
    assert.ok(screen.getByText(/Панель управления → Почта → Показать профили/))
    await user.click(screen.getByText('Запасной способ настройки профиля Outlook'))
    assert.ok(screen.getByText('control mlcfg32.cpl'))
    assert.ok(screen.getByRole('button',{name:'В окне Outlook'}))
    assert.ok(screen.getByLabelText('Application (client) ID'))
    assert.ok(screen.getByRole('button',{name:'Получить код входа'}).disabled)
    assert.equal(screen.queryByLabelText(/Пароль/),null)
    await user.click(screen.getByRole('button',{name:/У меня нет Client ID/}))
    assert.ok(screen.getByRole('heading',{name:'Регистрация — один раз'}))
    await user.type(screen.getByLabelText('Application (client) ID'),'01234567-89ab-cdef-0123-456789abcdef')
    await user.click(screen.getByRole('button',{name:'Получить код входа'}))
    await screen.findByText('ABCD1234')
    await user.click(screen.getByRole('link',{name:'Войти в Microsoft ↗'}))
    assert.deepEqual(opened,['https://microsoft.com/devicelogin'])
    component.$destroy();document.body.innerHTML=''
    connected=true;mail.setMailCode('');mail.setMailStatus({connected:true,account:{email:'me@example.com'}})
    const inbox=[{id:'one',subject:'Планы',isRead:false,receivedDateTime:'2026-10-05T08:00:00Z',from:{emailAddress:{name:'Друг',address:'friend@example.com'}}}]
    mail.outlook.update(s=>({...s,messages:inbox,selected:{...inbox[0],body:{contentType:'html',content:'<img src="https://evil.test/tracker"><script>alert(1)</script>'}}}))
    const composed=new Component({target:document.body});await svelte.tick()
    assert.ok(screen.getByText(/<img src=/));assert.equal(document.querySelector('img,script'),null)
    await user.type(screen.getByLabelText('Кому'),'friend@example.com')
    await user.type(screen.getByLabelText('Тема'),'Планы')
    await user.type(screen.getByLabelText('Текст письма'),'Встретимся завтра.')
    await user.click(screen.getByRole('button',{name:'Проверить и отправить →'}))
    await screen.findByRole('region',{name:'Подтверждение отправки'})
    assert.equal(calls.filter(c=>c.action==='send').length,0)
    await user.type(screen.getByLabelText('Текст письма'),' В десять.')
    assert.equal(screen.queryByRole('button',{name:'Да, отправить'}),null)
    await user.click(screen.getByRole('button',{name:'Проверить и отправить →'}))
    await user.click(await screen.findByRole('button',{name:'Да, отправить'}))
    await screen.findByText(/Microsoft принял письмо к отправке/)
    assert.equal(calls.filter(c=>c.action==='send').length,1)
    composed.$destroy()
    connected=false;mail.setMailStatus({connected:false})
    const local=new Component({target:document.body})
    await user.click(await screen.findByRole('button',{name:'Подключить открытый Outlook'}))
    await screen.findByText('angel@jarvis.test')
    await user.type(screen.getByLabelText('Кому'),'test@jarvis.test; second@jarvis.test')
    await user.type(screen.getByLabelText('Тема'),'Рассылка')
    await user.type(screen.getByLabelText('Текст письма'),'Локальный тест.')
    await user.click(screen.getByRole('button',{name:'Проверить и отправить →'}))
    await screen.findByRole('region',{name:'Подтверждение отправки'})
    assert.equal(calls.filter(c=>c.action==='send').length,1,'local prepare must not send')
    await user.click(screen.getByRole('button',{name:'Да, отправить'}))
    await screen.findByText(/Outlook принял письмо к отправке/)
    assert.equal(calls.filter(c=>c.action==='send').length,2)
    local.$destroy()
    connected=false;localSupported=false;mail.setMailStatus({connected:false});mail.setMailInOutlook(true)
    const unix=new Component({target:document.body})
    await screen.findByText(/Подключение Kerio доступно только в Windows/)
    await waitFor(()=>assert.equal(store.get(mail.mailInOutlook),false))
    assert.equal(screen.queryByRole('button',{name:'Подключить открытый Outlook'}),null)
    assert.equal(screen.queryByRole('button',{name:'В окне Outlook'}),null)
    assert.ok(screen.getByRole('region',{name:'Microsoft 365 / Outlook.com'}))
    unix.$destroy()
    console.log('Outlook UI: registration, login code, system browser, safe plain-text reading, exact confirmation, edit cancellation and one send passed')
}
main().then(()=>process.exit(0)).catch(error=>{console.error(error);process.exit(1)})

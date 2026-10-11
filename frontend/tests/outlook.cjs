const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict'),{Module}=require('node:module'),ts=require('typescript')
function load(file,stubs){const filename=path.resolve(file),mod=new Module(filename,module);mod.filename=filename;mod.paths=module.paths;mod.require=name=>stubs[name]||require(name);mod._compile(ts.transpileModule(fs.readFileSync(file,'utf8'),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022}}).outputText,filename);return mod.exports}
async function main(){
    const store=await import('svelte/store'),calls=[],spoken=[]
    let response={ticket:'TICKET',accepted:true},failure=null
    const mail=load('src/lib/outlook.ts',{'svelte/store':store,'@tauri-apps/api/core':{invoke:async(action,data)=>{calls.push({action,...data});if(failure)throw failure;return structuredClone(response)}}})
    const bridge={navigate:section=>assert.equal(section,'outlook'),reply:(text,id,followUp)=>spoken.push({text,followUp}),number:text=>/втор|2/.test(text)?2:undefined}
    assert.equal(mail.spokenMailAddress('test собачка джимейл точка ком'),'test@gmail.com')
    assert.equal(mail.spokenMailAddress('pat@gmail.com'),'pat@gmail.com')
    assert.equal(mail.spokenMailAddress('dotty at gmail dot com'),'dotty@gmail.com')
    assert.equal(mail.spokenMailAddress('test нижнее подчеркивание one собака яндекс точка ру'),'test_one@yandex.ru')
    assert.equal(mail.spokenMailAddress('Иван Петров'),null,'never fabricate an email from a name')
    assert.equal(mail.formatMailText('  привет   это тест  \n\n  вторая строка  '),'Привет это тест\n\nвторая строка.')
    assert.equal(mail.formatMailText('pat@gmail.com'),'pat@gmail.com')
    assert.equal(mail.formatMailText('https://example.test/path'),'https://example.test/path')
    await mail.handleMailVoice('напиши письмо',bridge)
    assert.match(spoken.at(-1).text,/подключите/);assert.equal(calls.at(-1).action,'status')
    await mail.handleMailVoice('запусти классический outlook',bridge)
    assert.equal(calls.at(-1).action,'open_classic')
    failure='Классический Outlook не найден на этом компьютере.'
    await mail.handleMailVoice('открой почту',bridge)
    assert.match(spoken.at(-1).text,/не найден/)
    failure=null;calls.length=0
    mail.setMailStatus({connected:true,account:{email:'me@example.com'}})
    await mail.handleMailVoice('напиши письмо',bridge)
    await mail.handleMailVoice('friend собака example точка com',bridge)
    await mail.handleMailVoice('Планы',bridge)
    await mail.handleMailVoice('Отправь письмо завтра, когда я вернусь.',bridge)
    assert.equal(store.get(mail.outlook).draft.body,'Отправь письмо завтра, когда я вернусь.')
    assert.equal(calls.length,0,'dictated body must not execute a command')
    await mail.handleMailVoice('отправь письмо',bridge)
    assert.equal(store.get(mail.outlook).ticket,'TICKET')
    await mail.handleMailVoice('возможно',bridge)
    assert.equal(calls.filter(call=>call.action==='send').length,0)
    await mail.handleMailVoice('да, отправить',bridge)
    assert.equal(calls.filter(call=>call.action==='send').length,1)
    assert.equal(store.get(mail.outlook).draft.body,'')
    await mail.handleMailVoice('напиши письмо',bridge)
    mail.editDraft({to:'typed@example.com'});await mail.handleMailVoice('продолжить письмо',bridge)
    await mail.handleMailVoice('Тема',bridge);await mail.handleMailVoice('Текст',bridge)
    assert.deepEqual(store.get(mail.outlook).draft,{to:'typed@example.com',subject:'Тема',body:'Текст'})
    await mail.prepareMail();mail.editDraft({body:'Изменено'});assert.equal(store.get(mail.outlook).ticket,'')
    await assert.rejects(mail.sendMail(),/Сначала проверьте/)
    await mail.prepareMail();failure='Статус отправки неизвестен'
    await assert.rejects(mail.sendMail(),error=>error===failure)
    assert.equal(store.get(mail.outlook).ticket,'');assert.equal(store.get(mail.outlook).draft.body,'Изменено')
    const sends=calls.filter(call=>call.action==='send').length
    await assert.rejects(mail.sendMail(),/Сначала проверьте/)
    assert.equal(calls.filter(call=>call.action==='send').length,sends,'timeouts must never retry send')
    failure=null
    mail.outlook.update(s=>({...s,messages:[{id:'a',subject:'A',receivedDateTime:''},{id:'b',subject:'B',receivedDateTime:''}]}))
    response={id:'b',subject:'B',receivedDateTime:''}
    await mail.handleMailVoice('открой второе письмо',bridge)
    assert.equal(calls.at(-1).data.id,'b');assert.equal(store.get(mail.outlook).selected.id,'b')
    response={ticket:'TICKET',accepted:true};await mail.prepareMail();await mail.handleMailVoice('не отправляй',bridge)
    assert.equal(store.get(mail.outlook).ticket,'');assert.equal(store.get(mail.outlook).draft.body,'Изменено')
    await mail.disconnectMail();assert.equal(store.get(mail.outlook).connected,false);assert.equal(store.get(mail.outlook).messages.length,0);assert.equal(store.get(mail.outlook).draft.body,'')
    assert.equal(await mail.handleMailVoice('погода на неделю',bridge),false)
    mail.setMailStatus({connected:true,provider:'local',account:{email:'angel@jarvis.test'}})
    mail.setMailInOutlook(true)
    const nativeBridge={...bridge,navigate:()=>{throw new Error('native mode must not navigate to JARVIS mail')}}
    await mail.handleMailVoice('открой почту',nativeBridge)
    assert.equal(calls.at(-1).action,'show_inbox')
    await mail.handleMailVoice('напиши письмо',nativeBridge)
    assert.equal(calls.at(-1).action,'compose','new letter opens an Outlook composer immediately')
    await mail.handleMailVoice('ошибочный адрес',nativeBridge)
    assert.equal(spoken.at(-1).followUp,true,'invalid address keeps listening')
    assert.match(spoken.at(-1).text,/повторите/)
    await mail.handleMailVoice('повтори',nativeBridge)
    assert.equal(spoken.at(-1).followUp,true)
    await mail.handleMailVoice('тест собака джарвис точка тест',nativeBridge)
    assert.equal(store.get(mail.outlook).draft.to,'test@jarvis.test')
    await mail.handleMailVoice('исправь адрес',nativeBridge)
    await mail.handleMailVoice('test собака jarvis точка test',nativeBridge)
    await mail.handleMailVoice('Тест Outlook',nativeBridge)
    await mail.handleMailVoice('Текст теста.',nativeBridge)
    assert.equal(spoken.at(-1).followUp,true,'body completion continues listening for send or correction')
    assert.equal(store.get(mail.outlook).ticket,'','showing filled draft is not send confirmation')
    assert.equal(calls.filter(c=>c.action==='send').length,sends,'opening composer or preview never sends')
    const beforeFormatting=calls.filter(c=>c.action==='send').length
    mail.editDraft({body:'привет   это тест'})
    await mail.handleMailVoice('оформи текст',nativeBridge)
    assert.equal(store.get(mail.outlook).draft.body,'Привет это тест.')
    assert.equal(store.get(mail.outlook).ticket,'','formatting cannot authorize sending')
    assert.equal(calls.filter(c=>c.action==='send').length,beforeFormatting)
    const before=calls.filter(c=>c.action==='send').length
    await mail.handleMailVoice('отправь письмо',nativeBridge)
    assert.equal(calls.at(-1).data.native,true)
    assert.equal(calls.filter(c=>c.action==='send').length,before)
    await mail.handleMailVoice('да отправить',nativeBridge)
    assert.equal(calls.filter(c=>c.action==='send').length,before+1)
    mail.setMailInOutlook(false)
    const saved=new Map()
    global.localStorage={getItem:key=>saved.get(key)??null,setItem:(key,value)=>saved.set(key,value)}
    mail.setMailInOutlook(true)
    const restoredCalls=[]
    const restarted=load('src/lib/outlook.ts',{'svelte/store':store,'@tauri-apps/api/core':{invoke:async(_,data)=>{
        restoredCalls.push(data.action)
        if(data.action==='status')return {connected:true,provider:'local',account:{email:'angel@jarvis.test'}}
        if(data.action==='show_inbox')throw 'Outlook temporarily unavailable'
        return {}
    }}})
    assert.equal(store.get(restarted.mailInOutlook),true,'native mode survives restart')
    await restarted.handleMailVoice('открой почту',nativeBridge)
    assert.deepEqual(restoredCalls,['status','show_inbox'],'voice restores saved connection without visiting the mail page')
    assert.equal(store.get(restarted.outlook).connected,true,'temporary Outlook failure must not clear the binding')
    await restarted.disconnectMail()
    assert.equal(store.get(restarted.outlook).connected,false,'explicit disconnect still works')
    restarted.setMailInOutlook(false)
    delete global.localStorage
    let manualReady=false
    const manualCalls=[]
    const manual=load('src/lib/outlook.ts',{'svelte/store':store,'@tauri-apps/api/core':{invoke:async(_,data)=>{
        manualCalls.push(data.action)
        if(data.action==='compose_read') {
            if(!manualReady)throw 'Введите адрес в Outlook и нажмите Tab'
            return {draft:{to:'test@jarvis.test',subject:'',body:''}}
        }
        return {}
    }}})
    manual.setMailStatus({connected:true,provider:'local',account:{email:'angel@jarvis.test'}})
    manual.setMailInOutlook(true)
    await manual.handleMailVoice('напиши письмо',nativeBridge)
    await manual.handleMailVoice('адрес указал',nativeBridge)
    assert.equal(spoken.at(-1).followUp,true,'uncommitted Outlook address keeps dialogue active')
    manualReady=true
    await manual.handleMailVoice('Джарвис, адрес указал',nativeBridge)
    assert.equal(store.get(manual.outlook).draft.to,'test@jarvis.test','manual Outlook recipient is imported')
    assert.match(spoken.at(-1).text,/тема/)
    await manual.handleMailVoice('Проверка ручного адреса',nativeBridge)
    assert.equal(store.get(manual.outlook).draft.subject,'Проверка ручного адреса')
    assert.equal(manualCalls.includes('send'),false,'importing manual address never sends')
    manual.resetMailVoice()
    assert.equal(await manual.handleMailVoice('готово',nativeBridge),false,'unrelated ready command must not start a mail dialogue')
    const suggestedCalls=[]
    const suggested=load('src/lib/outlook.ts',{'svelte/store':store,'@tauri-apps/api/core':{invoke:async(_,data)=>{
        suggestedCalls.push(data)
        if(data.action==='recipient_suggestions')return {candidates:[{name:'Первый',email:'first@jarvis.test'},{name:'Второй',email:'second@jarvis.test'}]}
        return {}
    }}})
    suggested.setMailStatus({connected:true,provider:'local',account:{email:'angel@jarvis.test'}})
    suggested.setMailInOutlook(true)
    await suggested.handleMailVoice('напиши письмо',nativeBridge)
    await suggested.handleMailVoice('Джарвис, предложи адресатов',nativeBridge)
    assert.equal(store.get(suggested.outlook).draft.to,'','suggestions do not silently select a recipient')
    await suggested.handleMailVoice('второй',nativeBridge)
    assert.match(spoken.at(-1).text,/second@jarvis.test/)
    await suggested.handleMailVoice('отправь письмо',nativeBridge)
    assert.equal(suggestedCalls.some(c=>c.action==='send'||c.action==='prepare_send'),false,'send cannot bypass recipient confirmation')
    await suggested.handleMailVoice('Джарвис, да этот адрес',nativeBridge)
    assert.equal(store.get(suggested.outlook).draft.to,'second@jarvis.test')
    assert.match(spoken.at(-1).text,/тема/)
    failure=null;response={ticket:'DRAFT-ONLY'}
    mail.outlook.set({...store.get(mail.outlook),connected:true,provider:'local',busy:false,ticket:'',draft:{to:'test@jarvis.test',subject:'Draft',body:'Text'}})
    const draftStart=calls.length
    await mail.openMailDraft()
    assert.deepEqual(calls.slice(draftStart).map(c=>c.action),['open_classic','prepare_send','cancel_send'])
    assert.equal(calls[draftStart+1].data.native,true)
    assert.equal(store.get(mail.outlook).ticket,'','Opening draft must leave no send permission')
    assert.equal(store.get(mail.outlook).draft.body,'Text')
    console.log('Outlook: application and native modes, dictation, confirmation, edit cancellation and no automatic resend passed')
}
main().catch(error=>{console.error(error);process.exit(1)})

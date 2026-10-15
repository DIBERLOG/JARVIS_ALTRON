const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict'),{Module,createRequire}=require('node:module'),ts=require('typescript')
const deps=createRequire(path.resolve(process.argv[2],'package.json')),{JSDOM}=deps('jsdom'),dom=new JSDOM('')
global.window=dom.window;global.DOMParser=dom.window.DOMParser
function load(file,stubs={}){const filename=path.resolve(file),mod=new Module(filename,module);mod.paths=module.paths;mod.require=name=>stubs[name]||require(name);mod._compile(ts.transpileModule(fs.readFileSync(file,'utf8'),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022}}).outputText,filename);return mod.exports}
async function main(){
 const reminders=load('src/lib/reminders.ts'),now=new Date('2026-10-05T12:00:00')
 assert.equal(reminders.parseReminderDate('31.02 в 15:00',now),null)
 assert.equal(reminders.parseReminderDate('сегодня в 09:00',now),null)
 assert.equal(reminders.parseReminderDate('вчера в 09:00',now,true).getDate(),4)
 assert.equal(reminders.reminderAlert({id:'past',title:'Архив',dueAt:'2026-10-04T09:00:00',createdAt:now.toISOString(),done:false,silentPast:true},now.getTime()),null)
 assert.equal(reminders.parseReminderDate('завтра в 23:99',now),null)
 assert.equal(reminders.parseReminderDate('через два дня',now).getTime(),now.getTime()+172800000)
 assert.equal(reminders.parseReminderDate('послезавтра в 15:30',now).getDate(),7)
 assert.equal(reminders.parseReminderDate('завтра в шесть вечера',now).getHours(),18)
 assert.equal(reminders.parseReminderDate('завтра в восемнадцать тридцать',now).getMinutes(),30)
 const r={id:'r',title:'Встреча',dueAt:'2026-10-08T12:00:00',done:false,advanceDays:[1,2],createdAt:'2026-10-04T12:00:00'}
 assert.equal(reminders.reminderAlert(r,now.getTime()),null)
 const first=reminders.reminderAlert(r,Date.parse('2026-10-06T12:00:01'));assert.match(first.detail,/2 дн/)
 assert.equal(reminders.reminderAlert({...r,announced:[first.key]},Date.parse('2026-10-06T12:01:00')),null)
 assert.match(reminders.reminderAlert(r,Date.parse('2026-10-08T12:00:00')).text,/пришло время/)
 assert.equal(reminders.reminderAlert({...r,done:true},Date.parse('2026-10-09T12:00:00')),null)
 const daily={...r,repeat:'daily',dueAt:'2026-10-05T09:00:00',createdAt:'2026-10-05T08:00:00',advanceDays:[1]}
 const alert=reminders.reminderAlert(daily,Date.parse('2026-10-05T09:00:01'))
 assert.match(alert.detail,/Ежедневно/)
 assert.equal(reminders.reminderAlert({...daily,announced:[alert.key]},Date.parse('2026-10-05T10:00:00')),null)
 assert.equal(reminders.reminderAlert(daily,Date.parse('2026-10-06T08:59:59')),null)
 assert.notEqual(reminders.reminderAlert({...daily,announced:[alert.key]},Date.parse('2026-10-06T09:00:00')).key,alert.key)
 assert.equal(reminders.reminderAlert({...daily,done:true},Date.parse('2026-10-06T09:00:00')),null)
 assert.equal(reminders.reminderAlert({...daily,createdAt:'2026-10-05T10:00:00'},Date.parse('2026-10-05T11:00:00')),null)
 assert.equal(reminders.reminderAlert({...daily,dueAt:'bad'},Date.now()),null)
 const rich=load('src/lib/richNotes.ts',{'dompurify':{default:require('dompurify')}}),training=load('src/lib/training.ts'),data={reminders:[r],birthdays:[{id:'b',name:'Друг',month:2,day:29}],notes:[{id:'n',title:'Запись',text:'Привет',updatedAt:now.toISOString(),html:'<p>Привет</p><script>evil()</script>'}],habits:[],training:training.defaultTraining()}
 const api={invoke:async()=> '{}'},backup=load('src/lib/centerBackup.ts',{'@tauri-apps/api/core':api,'./center':{loadCenterData:async()=>structuredClone(data)},'./weather':{getWeatherCity:async()=> 'Москва'},'./richNotes':rich,'./mindMap':load('src/lib/mindMap.ts',{'./center':{makeId:()=>require('node:crypto').randomUUID()}})})
 const snapshot=await backup.createCenterBackup(),parsed=backup.parseCenterBackup(JSON.stringify(snapshot))
 assert.equal(backup.parseCenterBackup(JSON.stringify({...snapshot,data:{...data,reminders:[daily]}})).data.reminders[0].repeat,'daily');
 assert.throws(()=>backup.parseCenterBackup(JSON.stringify({...snapshot,data:{...data,reminders:[{...r,repeat:'invalid'}]}})));
 const mindMap=JSON.parse(fs.readFileSync('../mobile/android/app/src/androidTest/assets/desktop-mindmap-fixture.json','utf8'))
 const mapBackup={...snapshot,data:{...data,notes:[{...data.notes[0],type:'mindmap',mindMap}]}}
 assert.equal(backup.parseCenterBackup(JSON.stringify(mapBackup)).data.notes[0].mindMap.nodes[0].files[0].data,'YWJj')
 const brokenMap=structuredClone(mapBackup);brokenMap.data.notes[0].mindMap.edges[0].to='missing';assert.throws(()=>backup.parseCenterBackup(JSON.stringify(brokenMap)))
 assert.equal(parsed.city,'Москва');assert.equal(parsed.data.training.plans.length,4);assert.ok(!parsed.data.notes[0].html.includes('script'))
 assert.throws(()=>backup.parseCenterBackup(JSON.stringify({...snapshot,version:999})))
 assert.throws(()=>backup.parseCenterBackup(JSON.stringify({...snapshot,data:{...data,reminders:[{...r,dueAt:'bad'}]}})))
 assert.throws(()=>backup.parseCenterBackup(JSON.stringify({...snapshot,data:{...data,notes:[data.notes[0],data.notes[0]]}})))
 if(process.argv[3]){
   const actual=JSON.parse(fs.readFileSync(process.argv[3],'utf8'))
   if(typeof actual.center_data==='string'&&actual.center_data){backup.parseCenterBackup(JSON.stringify({...snapshot,data:JSON.parse(actual.center_data)}));console.log('Existing personal data passes import validation (read-only check)')}
 }
 console.log('Reminder dates, advance alerts, deduplication, backup round-trip, HTML sanitization and invalid-file rejection passed')
}
main().catch(error=>{console.error(error);process.exitCode=1})

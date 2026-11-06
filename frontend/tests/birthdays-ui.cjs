// node --conditions=browser tests/birthdays-ui.cjs <test dependency directory>
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict'),{createRequire,Module}=require('node:module')
const deps=createRequire(path.resolve(process.argv[2],'package.json')),{JSDOM}=deps('jsdom'),dom=new JSDOM('<body></body>',{url:'http://localhost'})
for(const name of ['window','document','navigator','HTMLElement','Node','Event','MouseEvent','getComputedStyle'])Object.defineProperty(globalThis,name,{value:dom.window[name],configurable:true})
const {screen,fireEvent}=deps('@testing-library/dom'),userEvent=deps('@testing-library/user-event').default,ts=require('typescript'),{compile,preprocess}=require('svelte/compiler')
function load(filename,code,stubs){const mod=new Module(filename,module);mod.paths=module.paths;mod.require=name=>stubs[name]||require(name);mod._compile(ts.transpileModule(code,{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022}}).outputText,filename);return mod.exports}
async function main(){
 const svelte=await import('svelte'),store=await import('svelte/store');let data={reminders:[],birthdays:[],notes:[],habits:[]}
 const api={invoke:async(action,args)=>{if(action==='db_read')return JSON.stringify(data);if(action==='db_write'){data=JSON.parse(args.val);return true}}}
 const center=load(path.resolve('src/lib/center.ts'),fs.readFileSync('src/lib/center.ts','utf8'),{'@tauri-apps/api/core':api})
 const future={id:'future',name:'Будущая дата',month:12,day:25,year:2030},now=new Date(2026,9,6)
 assert.equal(center.nextBirthday(future,now).getFullYear(),2030)
 assert.equal(center.birthdayOccursInYear(future,2026),false)
 assert.equal(center.birthdayOccursInYear(future,2030),true)
 assert.equal(center.nextBirthday({...future,year:2000},now).getFullYear(),2026)
 assert.equal(center.nextBirthday({...future,day:1,month:1,year:2000},now).getFullYear(),2027)
 const filename=path.resolve('src/routes/center/index.svelte')
 const processed=await preprocess(fs.readFileSync(filename,'utf8'),{script:({content})=>({code:ts.transpileModule(content,{compilerOptions:{module:ts.ModuleKind.ESNext,target:ts.ScriptTarget.ES2022,verbatimModuleSyntax:true}}).outputText})},{filename})
 const empty=load(filename+'.stub',compile('<div></div>',{generate:'dom'}).js.code,{svelte})
 const stubs={svelte,'@/lib/center':center,'@/lib/centerVoice':{centerSection:store.writable('birthdays'),centerRevision:store.writable(0)},'@/lib/ipc':{sendAction(){}}}
 const mod=new Module(filename,module);mod.paths=module.paths;mod.require=name=>name.endsWith('.svelte')?empty:name.endsWith('.css')?{}:stubs[name]||require(name)
 mod._compile(ts.transpileModule(compile(processed.code,{filename,generate:'dom',css:'external'}).js.code,{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022}}).outputText,filename)
 const component=new mod.exports.default({target:document.body}),user=userEvent.setup({document})
 await screen.findByText(/Добавьте важную дату/)
 const input=screen.getByLabelText('Дата рождения')
 assert.equal(input.max,'9999-12-31','future dates are not limited to today')
 await user.type(screen.getByLabelText('Имя именинника'),'Будущая дата')
 const futureYear=new Date().getFullYear()+2
 // The native calendar picker is not simulated by userEvent.
 fireEvent.input(input,{target:{value:`${futureYear}-12-25`}})
 await user.click(screen.getByRole('button',{name:'+ Сохранить дату'}))
 await screen.findByText('День рождения сохранён')
 assert.equal(data.birthdays[0].year,futureYear)
 assert.ok(screen.getByText(/первая дата/))
 assert.equal(screen.queryByText(/исполнится -/),null)
 component.$destroy();console.log('Future birthday selection, persistence, first occurrence and existing anniversaries passed')
}
main().then(()=>process.exit(0)).catch(error=>{console.error(error);process.exit(1)})

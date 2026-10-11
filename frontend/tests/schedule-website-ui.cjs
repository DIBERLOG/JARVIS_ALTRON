// node --conditions=browser tests/schedule-website-ui.cjs <test dependency directory>
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict'),{createRequire,Module}=require('node:module')
const deps=createRequire(path.resolve(process.argv[2],'package.json')),{JSDOM}=deps('jsdom'),dom=new JSDOM('<body></body>',{url:'http://localhost'})
for(const name of ['window','document','navigator','HTMLElement','Node','Event','MouseEvent','CustomEvent','getComputedStyle'])Object.defineProperty(globalThis,name,{value:dom.window[name],configurable:true})
dom.window.HTMLDialogElement.prototype.showModal=function(){this.open=true}
dom.window.HTMLDialogElement.prototype.close=function(){this.open=false;this.dispatchEvent(new Event('close'))}
const {screen,waitFor}=deps('@testing-library/dom'),userEvent=deps('@testing-library/user-event').default,ts=require('typescript'),{compile,preprocess}=require('svelte/compiler')
async function component(relative,stubs){const filename=path.resolve(relative),processed=await preprocess(fs.readFileSync(filename,'utf8'),{script:({content})=>({code:ts.transpileModule(content,{compilerOptions:{module:ts.ModuleKind.ESNext,target:ts.ScriptTarget.ES2022,verbatimModuleSyntax:true}}).outputText})},{filename});const mod=new Module(filename,module);mod.paths=module.paths;mod.require=name=>stubs[name]||require(name);mod._compile(ts.transpileModule(compile(processed.code,{filename,generate:'dom',css:'external'}).js.code,{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022}}).outputText,filename);return mod.exports.default}
async function main(){
 const svelte=await import('svelte');let saved='',opened=[],writes=0
 const api={invoke:async(action,args)=>{if(action==='db_read')return saved;if(action==='db_write_many'){saved=args.entries[0].val;writes++;return}if(action==='animate_window_in')return;throw Error(action)}}
 const Editor=await component('src/components/ScheduleWebsite.svelte',{svelte,'@tauri-apps/api/core':api})
 const Dialog=await component('src/components/ScheduleWebsiteDialog.svelte',{svelte,'@tauri-apps/api/core':api,'@tauri-apps/plugin-shell':{open:async url=>opened.push(url)},'./ScheduleWebsite.svelte':{default:Editor}})
 const instance=new Dialog({target:document.body}),user=userEvent.setup({document})
 window.dispatchEvent(new CustomEvent('jarvis-schedule-open'))
 await screen.findByRole('dialog',{name:'Настроить сайт расписания'});await waitFor(()=>assert.equal(screen.getByLabelText('Ссылка на сайт').disabled,false))
 await user.type(screen.getByLabelText('Ссылка на сайт'),'javascript:alert(1)');await user.click(screen.getByRole('button',{name:'Сохранить ссылку'}));await screen.findByRole('alert');assert.equal(writes,0)
 await user.clear(screen.getByLabelText('Ссылка на сайт'));await user.type(screen.getByLabelText('Ссылка на сайт'),'https://example.com/schedule?a=1&b=2');await user.click(screen.getByRole('button',{name:'Сохранить ссылку'}))
 await waitFor(()=>assert.equal(opened.length,1));assert.equal(saved,opened[0]);await waitFor(()=>assert.equal(screen.queryByRole('dialog'),null))
 window.dispatchEvent(new CustomEvent('jarvis-schedule-open'));await waitFor(()=>assert.equal(opened.length,2));assert.equal(screen.queryByRole('dialog'),null)
 window.dispatchEvent(new CustomEvent('jarvis-schedule-open',{detail:{configure:true}}));await screen.findByRole('dialog');await waitFor(()=>assert.equal(screen.getByLabelText('Ссылка на сайт').value,saved));await user.clear(screen.getByLabelText('Ссылка на сайт'));await user.click(screen.getByRole('button',{name:'Сохранить ссылку'}));await waitFor(()=>assert.equal(saved,''));assert.equal(opened.length,2)
 window.dispatchEvent(new CustomEvent('jarvis-schedule-open'));await screen.findByRole('dialog');await user.click(screen.getByRole('button',{name:'Закрыть'}));assert.equal(writes,2,'cancel must not change settings')
 instance.$destroy();console.log('Schedule link dialog, persistence, opening, editing, clearing, cancellation and unsafe URL rejection passed')
}
main().then(()=>process.exit(0)).catch(e=>{console.error(e);process.exit(1)})

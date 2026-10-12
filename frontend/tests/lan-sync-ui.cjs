// Run: node --conditions=browser tests/lan-sync-ui.cjs <test dependency directory>
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict'),{createRequire,Module}=require('node:module');
const deps=createRequire(path.resolve(process.argv[2],'package.json')),{JSDOM}=deps('jsdom');
const dom=new JSDOM('<!doctype html><body></body>',{url:'http://localhost'});
for(const name of ['window','document','navigator','HTMLElement','Node','Event','MouseEvent','getComputedStyle'])Object.defineProperty(globalThis,name,{value:dom.window[name],configurable:true});
const {screen,waitFor}=deps('@testing-library/dom'),userEvent=deps('@testing-library/user-event').default;
const ts=require('typescript'),{compile,preprocess}=require('svelte/compiler');
async function main(){
 const svelte=await import('svelte'),calls=[];let approved=false,allowVault=false,running=false,confirmResult=false,prepared=false;
 global.confirm=()=>confirmResult;
 const invoke=async(command,args)=>{calls.push({command,args});if(command==='sync_v2_prepare'){prepared=true;return 'fictional-test-profile/recovery.json';}if(command==='lan_sync_addresses')return ['192.168.1.2'];if(command==='lan_sync_start')running=true;if(command==='lan_sync_stop')running=false;if(command==='lan_sync_approve')approved=true;if(command==='lan_sync_vault_enabled')allowVault=args.enabled;return {version:2,prepared,client:false,v2Conflicts:[],running,address:'192.168.1.2:47832',qr:running?'fictional-qr':null,allowVault,devices:[],conflicts:[],pending:running&&!approved?[{id:'test',name:'Test phone',code:'123456'}]:[]};};
 const filename=path.resolve('src/components/LanSync.svelte');
 const processed=await preprocess(fs.readFileSync(filename,'utf8'),{script:({content,attributes})=>attributes.lang==='ts'?{code:ts.transpileModule(content,{compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.ESNext,verbatimModuleSyntax:true}}).outputText}:undefined},{filename});
 const compiled=compile(processed.code,{filename,generate:'dom',css:'external'}),mod=new Module(filename,module);mod.filename=filename;mod.paths=module.paths;
 mod.require=name=>name==='svelte'?svelte:name==='@tauri-apps/api/core'?{invoke}:name==='@tauri-apps/plugin-dialog'?{save:async()=>null,open:async()=>null}:name==='@tauri-apps/plugin-fs'?{writeTextFile:async()=>{},readTextFile:async()=>''}:name==='qrcode'?{default:{toDataURL:async()=> 'data:image/png;base64,AA=='}}:require(name);
 mod._compile(ts.transpileModule(compiled.js.code,{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022}}).outputText,filename);
 const component=new mod.exports.default({target:document.body}),user=userEvent.setup({document});
 try{
  assert.equal(screen.queryByRole('button',{name:'Включить обмен по Wi-Fi'}),null);
  const create=await screen.findByRole('button',{name:'Создать хаб и копию восстановления'});assert.equal(create.disabled,true);
  await user.type(screen.getByLabelText(/Пароль копии восстановления/),'fictional master password');await user.click(create);
  await waitFor(()=>assert.equal(screen.getByLabelText(/Пароль копии восстановления/).value,''));
  assert.ok(calls.some(c=>c.command==='sync_v2_prepare'));
  const start=await screen.findByRole('button',{name:'Включить обмен по Wi-Fi'});await waitFor(()=>assert.equal(start.disabled,false));
  assert.equal(calls.some(c=>c.command==='lan_sync_start'),false);
  await user.click(start);await screen.findByText('123456');assert.ok(screen.getByRole('img',{name:'QR-код сопряжения телефона'}));
  await user.click(screen.getByRole('button',{name:'Код совпадает — разрешить'}));await waitFor(()=>assert.equal(screen.queryByText('123456'),null));
  const vault=screen.getByRole('checkbox',{name:'Разрешить синхронизацию зашифрованного сейфа'});await user.click(vault);await waitFor(()=>assert.equal(vault.checked,false));assert.equal(calls.some(c=>c.command==='lan_sync_vault_enabled'),false);
  confirmResult=true;await user.click(vault);await waitFor(()=>assert.equal(vault.checked,true));
  await user.click(screen.getByRole('button',{name:'Остановить обмен'}));await screen.findByRole('button',{name:'Включить обмен по Wi-Fi'});
  console.log('PASS: recovery prerequisite, password cleared, opt-in start, pairing approval, vault consent cancellation/acceptance and stop');
 }finally{component.$destroy();dom.window.close();}
}
main().catch(e=>{console.error(e);process.exitCode=1;});

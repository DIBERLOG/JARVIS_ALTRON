const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict'),{Module}=require('node:module'),ts=require('typescript');
async function main(){
 const store=await import('svelte/store'),sockets=[];
 global.WebSocket=class{static OPEN=1;static CONNECTING=0;constructor(url){this.url=url;this.readyState=0;this.sent=[];sockets.push(this)}send(value){this.sent.push(JSON.parse(value))}close(){this.readyState=3;this.onclose?.()}};
 const file=path.resolve('src/lib/ipc.ts'),mod=new Module(file,module);mod.filename=file;mod.paths=module.paths;
 mod.require=name=>name==='svelte/store'?store:name==='@tauri-apps/api/core'?{invoke:async command=>{assert.equal(command,'ipc_credentials');return 'test-secret'}}:name==='./requestHistory'?{recordRequest(){}}:require(name);
 mod._compile(ts.transpileModule(fs.readFileSync(file,'utf8'),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022}}).outputText,file);
 const ipc=mod.exports;
 try{
  await ipc.connectIpc();const socket=sockets[0];socket.readyState=WebSocket.OPEN;socket.onopen();
  assert.equal(socket.url,'ws://127.0.0.1:9712');assert.deepEqual(socket.sent,[{action:'authenticate',token:'test-secret'}]);
  assert.equal(store.get(ipc.ipcConnected),false);assert.equal(ipc.sendTextCommand('must not run yet'),false);
  socket.onmessage({data:JSON.stringify({event:'speech_recognized',text:'not authenticated'})});assert.equal(store.get(ipc.lastRecognizedText),'');
  socket.onmessage({data:'{"event":"authenticated"}'});assert.equal(store.get(ipc.ipcConnected),true);assert.equal(socket.sent.at(-1).action,'get_muted');
  assert.equal(ipc.sendTextCommand('test only'),true);assert.equal(socket.sent.at(-1).action,'text_command');
  await ipc.connectIpc();assert.equal(sockets.length,1);
  console.log('PASS: no commands/events before authentication; native credential lookup; one connection; authenticated command');
 }finally{ipc.disableIpc();}
}
main().catch(error=>{console.error(error);process.exitCode=1});

import {readFile,writeFile,cp,readdir} from 'node:fs/promises';
import {build} from 'vite';
import {svelte} from '@sveltejs/vite-plugin-svelte';
import preprocess from 'svelte-preprocess';
import {fileURLToPath} from 'node:url';
import path from 'node:path';
const here=path.dirname(fileURLToPath(import.meta.url)),root=path.resolve(here,'..');
const result=await build({configFile:false,publicDir:false,root,plugins:[svelte({preprocess:preprocess({typescript:true}),onwarn:(w,handler)=>{if(w.code!=='css-unused-selector')handler(w);}})],resolve:{alias:[
 {find:'@/lib/centerVoice',replacement:path.join(here,'voice.ts')},
 {find:'@/lib/center',replacement:path.join(here,'center.ts')},
 {find:'@/lib/timer',replacement:path.join(here,'timer.ts')},
 {find:'@',replacement:path.join(root,'src')},
]},build:{outDir:path.resolve(root,'../mobile/android/app/build/generated/mobile-center/assets'),emptyOutDir:true,target:'chrome100',cssCodeSplit:false,lib:{entry:path.join(here,'main.ts'),name:'JarvisMobileCenter',formats:['iife'],fileName:()=> 'mobile-center.js'},rollupOptions:{output:{assetFileNames:'mobile-center.css'}}}});

const bundle=path.resolve(root,'../mobile/android/app/build/generated/mobile-center/assets/mobile-center.js');
const body='data:image/png;base64,'+(await readFile(path.join(root,'public/images/training-body-hud-v1.png'))).toString('base64');
await writeFile(bundle,(await readFile(bundle,'utf8')).replaceAll('/images/training-body-hud-v1.png',body));

const assets=path.dirname(bundle);
await cp(path.resolve(root,'../third_party/licenses'),path.join(assets,'licenses'),{recursive:true});
await cp(path.resolve(root,'../THIRD_PARTY_NOTICES.md'),path.join(assets,'THIRD_PARTY_NOTICES.md'));
await cp(path.resolve(root,'../LICENSE.txt'),path.join(assets,'LICENSE.txt'));
const packages=new Set();
for(const output of Array.isArray(result)?result:[result])for(const chunk of output.output??[])for(const module of Object.keys(chunk.modules??{})){
 const id=module.split(String.fromCharCode(92)).join('/'),at=id.lastIndexOf('/node_modules/');if(at<0)continue;
 const parts=id.slice(at+14).split('/'),name=parts[0].startsWith('@')?parts.slice(0,2).join('/'):parts[0];packages.add(id.slice(0,at+14)+name);
}
const packageNotices=[];
for(const directory of packages){try{const pkg=JSON.parse(await readFile(path.join(directory,'package.json'),'utf8'));packageNotices.push(`${pkg.name} ${pkg.version}: ${pkg.license??'see included license'}`);for(const file of await readdir(directory))if(/^(licen[cs]e|copying|notice)/i.test(file))await cp(path.join(directory,file),path.join(assets,'licenses',`${pkg.name.replaceAll('/','-')}-${file}`),{recursive:true});}catch(e){if(e.code!=='ENOENT')throw e;}}
await writeFile(path.join(assets,'BUNDLED_JS_NOTICES.txt'),packageNotices.sort().join(String.fromCharCode(10)));
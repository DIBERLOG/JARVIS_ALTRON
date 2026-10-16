// Headless UI check against fictional, in-memory preferences. No real Tauri IPC.
import {createRequire} from 'node:module';
import {fileURLToPath,pathToFileURL} from 'node:url';
import path from 'node:path';
import fs from 'node:fs';
import assert from 'node:assert/strict';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const requireFrontend=createRequire(path.join(root,'frontend/package.json'));
const {createServer}=await import(pathToFileURL(path.join(root,'frontend/node_modules/vite/dist/node/index.js')).href);
const {svelte}=await import(pathToFileURL(path.join(root,'frontend/node_modules/@sveltejs/vite-plugin-svelte/src/index.js')).href);
const preprocess=requireFrontend('svelte-preprocess');
const {chromium}=process.env.JARVIS_TEST_NODE_MODULES?createRequire(path.join(process.env.JARVIS_TEST_NODE_MODULES,'playwright/package.json'))('playwright'):requireFrontend('playwright');
const server=await createServer({configFile:false,root:path.join(root,'frontend'),plugins:[svelte({preprocess:preprocess({typescript:true}),onwarn:(warning,handler)=>{if(warning.code!=='css-unused-selector')handler(warning)}})],resolve:{alias:{'@':path.join(root,'frontend/src')}},server:{host:'127.0.0.1',port:1438,strictPort:true,fs:{allow:[root]}}});
await server.listen();
const browser=await chromium.launch({headless:true,executablePath:process.env.JARVIS_TEST_CHROMIUM});
const page=await browser.newPage({viewport:{width:1440,height:1160},acceptDownloads:true});
page.setDefaultTimeout(15000);const errors=[];page.on('pageerror',e=>errors.push(e.message));
const url='http://127.0.0.1:1438/tests/mindmap/index.html';
try{
 await page.goto(url);await page.getByRole('button',{name:'Карточка: Мой проект',exact:true}).waitFor();
 await page.screenshot({path:path.join(root,'target/mindmap-desktop.png'),fullPage:true});
 await page.getByRole('button',{name:'Карточка: Цель',exact:true}).click();
 await page.getByLabel('Название карточки',{exact:true}).fill('Наша цель');await page.getByLabel('Текст карточки',{exact:true}).fill('Проверенная цель проекта');
 await page.getByRole('button',{name:'Шаг плана',exact:true}).click();await page.getByLabel('Шаг плана 1',{exact:true}).fill('Подготовить материалы');
 await page.getByRole('button',{name:'Картинка',exact:true}).click();
 await page.locator('input[type=file][accept^="image/"]').setInputFiles(path.join(root,'frontend/public/images/training-body-hud-v1.png'));
 await page.locator('.image-list img').waitFor();
 await page.getByRole('button',{name:'Файл',exact:true}).click();
 await page.locator('input[type=file][accept^=".pdf"]').setInputFiles({name:'План.txt',mimeType:'text/plain',buffer:Buffer.from('План проекта — вымышленный документ')});
 await page.getByText('План.txt',{exact:true}).first().waitFor();
 const download=page.waitForEvent('download');await page.locator('.attachment-row button').first().click();assert.equal((await download).suggestedFilename(),'План.txt');
 await page.getByRole('button',{name:'Закрыть карточку',exact:true}).click();
 await page.getByRole('button',{name:'Сохранить изменения',exact:true}).click();
 let saved=await page.evaluate(()=>window.fixture.saved.at(-1));assert.equal(saved.type,'mindmap');let goal=saved.mindMap.nodes.find(n=>n.title==='Наша цель');assert.equal(goal.images.length,1);assert.equal(goal.files.length,1);assert.equal(Buffer.from(goal.files[0].data,'base64').toString(),'План проекта — вымышленный документ');assert.equal(goal.items[0].text,'Подготовить материалы');
 // Drag a card and verify its stored coordinates rather than transient CSS.
 const heading=page.getByRole('button',{name:'Карточка: Наша цель',exact:true}),box=await heading.boundingBox();await page.mouse.move(box.x+box.width/2,box.y+box.height/2);await page.mouse.down();await page.mouse.move(box.x+box.width/2+55,box.y+box.height/2+30,{steps:8});await page.mouse.up();await page.getByRole('button',{name:'Сохранить изменения',exact:true}).click();
 saved=await page.evaluate(()=>window.fixture.saved.at(-1));assert.ok(saved.mindMap.nodes.find(n=>n.title==='Наша цель').x>goal.x);
 await page.getByRole('button',{name:'Отменить изменение карты',exact:true}).click();await page.getByRole('button',{name:'Сохранить изменения',exact:true}).click();assert.equal((await page.evaluate(()=>window.fixture.saved.at(-1))).mindMap.nodes.find(n=>n.title==='Наша цель').x,goal.x);
 await page.getByRole('button',{name:'Повторить изменение карты',exact:true}).click();await page.getByRole('button',{name:'Сохранить изменения',exact:true}).click();
 await page.getByRole('button',{name:'План',exact:true}).click();await page.getByLabel('Название карточки',{exact:true}).fill('План запуска');await page.getByLabel('Шаг плана 1',{exact:true}).fill('Собрать команду');await page.getByRole('button',{name:'Закрыть карточку',exact:true}).click();await page.getByRole('button',{name:'Показать всю карту',exact:true}).click();
 await page.getByRole('button',{name:'Сохранить изменения',exact:true}).click();saved=await page.evaluate(()=>window.fixture.saved.at(-1));assert.equal(saved.mindMap.nodes.length,7);assert.equal(saved.mindMap.edges.length,6);
 await page.reload();await page.getByRole('button',{name:'Карточка: План запуска',exact:true}).waitFor();
 await page.getByRole('checkbox',{name:'Шаг: Собрать команду',exact:true}).check();await page.getByRole('button',{name:'Сохранить изменения',exact:true}).click();saved=await page.evaluate(()=>window.fixture.saved.at(-1));assert.equal(saved.mindMap.nodes.find(n=>n.title==='План запуска').items[0].done,true);
 await page.getByRole('button',{name:'Карточка: Наша цель',exact:true}).click();await page.getByRole('button',{name:'Закрыть карточку',exact:true}).click();await page.getByRole('button',{name:'Связать карточки',exact:true}).click();await page.getByRole('button',{name:'Карточка: Идеи',exact:true}).click();await page.getByRole('button',{name:'Сохранить изменения',exact:true}).click();assert.equal((await page.evaluate(()=>window.fixture.saved.at(-1))).mindMap.edges.length,7);
 await page.getByRole('button',{name:'Карточка: Наша цель',exact:true}).click();await page.getByText('Связи карточки',{exact:true}).click();await page.locator('.link-row button').last().click();await page.getByRole('button',{name:'Закрыть карточку',exact:true}).click();await page.getByRole('button',{name:'Сохранить изменения',exact:true}).click();assert.equal((await page.evaluate(()=>window.fixture.saved.at(-1))).mindMap.edges.length,6);
 await page.getByRole('button',{name:'Заметка',exact:true}).click();await page.getByLabel('Название карточки',{exact:true}).fill('Удаляемая карточка');page.once('dialog',d=>d.accept());await page.locator('.inspector').getByRole('button',{name:'Удалить',exact:true}).click();await page.getByRole('button',{name:'Сохранить изменения',exact:true}).click();assert.equal((await page.evaluate(()=>window.fixture.saved.at(-1))).mindMap.nodes.length,7);await page.getByRole('button',{name:'Показать всю карту',exact:true}).click();
 await page.screenshot({path:path.join(root,'target/mindmap-desktop-edited.png'),fullPage:true});
 await page.getByRole('button',{name:'Карточка: План запуска',exact:true}).click();await page.getByLabel('Текст карточки',{exact:true}).fill('Проверка сохранения');await page.getByRole('button',{name:'Закрыть карточку',exact:true}).click();await page.getByRole('button',{name:'Развернуть карту',exact:true}).click();await page.evaluate(()=>window.fixture.failSave=true);await page.getByRole('button',{name:'Сохранить карту',exact:true}).click();await page.locator('.mind-map .map-error').waitFor();assert.equal(await page.locator('.mind-map .map-error').isVisible(),true);await page.evaluate(()=>window.fixture.failSave=false);await page.getByRole('button',{name:'Сохранить карту',exact:true}).click();await page.getByRole('button',{name:'Сохранено',exact:true}).waitFor();assert.equal(await page.locator('.mind-map .map-error').count(),0);await page.getByRole('button',{name:'Свернуть карту',exact:true}).click();
 // Desktop window dimensions and nested scrolling match the real center pane.
 await page.setViewportSize({width:1040,height:800});
 await page.addStyleTag({content:'html,body{height:100%;overflow:hidden}.fixture-shell{height:calc(100vh - 82px);overflow-y:auto;margin-top:82px}'});
 for(let cycle=0;cycle<4;cycle++){
  await page.getByRole('button',{name:'Развернуть карту',exact:true}).click();
  await page.getByRole('button',{name:'Свернуть карту',exact:true}).click();
  assert.equal(await page.locator('.mind-map.expanded').count(),0);
  await page.getByRole('button',{name:'Карточка: План запуска',exact:true}).click();
  await page.getByLabel('Название карточки',{exact:true}).fill('План запуска');
  await page.getByRole('button',{name:'Закрыть карточку',exact:true}).click();
  await page.getByRole('button',{name:'Сохранить изменения',exact:true}).click();
 }
 // Leaving and reopening the map must destroy its overlay and remain interactive.
 await page.getByRole('button',{name:'+ Новая',exact:true}).click();
 await page.locator('.note-list button').filter({hasText:'Мой проект'}).click();
 await page.getByRole('button',{name:'Развернуть карту',exact:true}).click();
 await page.reload();
 await page.getByRole('button',{name:'Развернуть карту',exact:true}).click();
 await page.getByRole('button',{name:'Свернуть карту',exact:true}).click();
 await page.getByRole('button',{name:'Показать всю карту',exact:true}).click();
 await page.getByRole('button',{name:'Развернуть карту',exact:true}).click();
 await page.evaluate(()=>window.fixture.unmount());assert.equal(await page.locator('.mind-map').count(),0);
 await page.evaluate(()=>window.fixture.remount());
 await page.getByRole('button',{name:'Развернуть карту',exact:true}).click();
 await page.getByRole('button',{name:'Свернуть карту',exact:true}).click();
 // Both mobile pickers reuse native, user-selected attachments.
 await page.setViewportSize({width:390,height:950});await page.goto(url+'?mobile');await page.getByRole('button',{name:'Карточка: План запуска',exact:true}).waitFor();
 assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),true);
 await page.screenshot({path:path.join(root,'target/mindmap-mobile.png'),fullPage:true});await page.getByRole('button',{name:'Развернуть карту',exact:true}).click();await page.screenshot({path:path.join(root,'target/mindmap-mobile-expanded.png'),fullPage:true});await page.getByRole('button',{name:'Свернуть карту',exact:true}).click();await page.getByRole('button',{name:'Показать всю карту',exact:true}).click();
 await page.getByRole('button',{name:'Карточка: План запуска',exact:true}).click();await page.getByRole('button',{name:'Файл',exact:true}).click();assert.equal(await page.evaluate(()=>window.fixture.picks.at(-1)),'file');
 await page.evaluate(()=>window.dispatchEvent(new CustomEvent('jarvis-attachment',{detail:{file:{name:'phone.txt',mime:'text/plain',size:3,data:'YWJj'}}})));
 await page.getByText('phone.txt',{exact:true}).waitFor();await page.getByRole('button',{name:'Картинка',exact:true}).click();assert.equal(await page.evaluate(()=>window.fixture.picks.at(-1)),'image');
 const image=await page.locator('.node-images img').first().getAttribute('src');await page.evaluate(src=>window.dispatchEvent(new CustomEvent('jarvis-image',{detail:{src}})),image);
 await page.locator('.image-list img').waitFor();await page.getByRole('button',{name:'Закрыть карточку',exact:true}).click();await page.getByRole('button',{name:'Сохранить изменения',exact:true}).click();
 saved=await page.evaluate(()=>window.fixture.saved.at(-1));assert.equal(saved.mindMap.nodes.find(n=>n.title==='План запуска').files[0].data,'YWJj');assert.equal(saved.mindMap.nodes.find(n=>n.title==='План запуска').images.length,1);
 await page.getByRole('button',{name:'Карточка: План запуска',exact:true}).click();await page.screenshot({path:path.join(root,'target/mindmap-mobile-editor.png'),fullPage:true});
 await page.getByRole('button',{name:'Закрыть карточку',exact:true}).click();
 await page.goto(url+'?mobile&light');await page.getByRole('button',{name:'Карточка: План запуска',exact:true}).waitFor();assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),true);await page.screenshot({path:path.join(root,'target/mindmap-mobile-light.png'),fullPage:true});
 for(let cycle=0;cycle<3;cycle++){
  await page.getByRole('button',{name:'Развернуть карту',exact:true}).click();
  await page.getByRole('button',{name:'Свернуть карту',exact:true}).click();
  assert.equal(await page.locator('.mind-map.expanded').count(),0);
 }
 await page.evaluate(()=>window.fixture.unmount());await page.evaluate(()=>window.fixture.remount());
 await page.getByRole('button',{name:'Развернуть карту',exact:true}).click();
 await page.getByRole('button',{name:'Свернуть карту',exact:true}).click();
 // A long list must scroll rather than compress note headings vertically.
 await page.setViewportSize({width:1040,height:800});await page.goto(url+'?many');
 await page.locator('.note-list button').nth(20).waitFor({state:'attached'});
 const noteGeometry=await page.locator('.note-list').evaluate(list=>({scrolls:list.scrollHeight>list.clientHeight,headings:[...list.querySelectorAll('strong')].map(h=>({height:h.clientHeight,line:parseFloat(getComputedStyle(h).lineHeight)}))}));
 assert.equal(noteGeometry.scrolls,true);assert.ok(noteGeometry.headings.every(h=>h.height>=h.line-1),'Note headings must retain a full line of height');
 await page.locator('.note-list button').last().click();
 await page.screenshot({path:path.join(root,'target/notes-list-height-fixed.png'),fullPage:true});
 assert.deepEqual(errors,[]);fs.writeFileSync(path.join(root,'target/mindmap-ui-tests.log'),'PASS: create/edit, images and file bytes, download, drag persistence, undo/redo, checklist, reload, native callbacks, mobile geometry, light theme, repeated collapse, nested desktop scrolling, editor unmount/remount and restart.\n');console.log('PASS: shared mind-map editor, fictional notes only.');
}catch(e){await page.screenshot({path:path.join(root,'target/mindmap-test-failure.png'),fullPage:true});console.error('Browser errors:',errors);throw e}finally{await browser.close();await server.close()}

import { makeId } from './center'
export type MapKind = 'idea' | 'goal' | 'plan' | 'images' | 'note'
export type MapImage = {id:string;src:string;name:string}
export type MapFile = {id:string;name:string;mime:string;data:string;size:number}
export type MapNode = {id:string;kind:MapKind;title:string;text:string;x:number;y:number;width?:number;height?:number;imageHeight?:number;imageFit?:'contain'|'cover';items:{id:string;text:string;done:boolean}[];images:MapImage[];files:MapFile[]}
export type MindMap = {version:1;nodes:MapNode[];edges:{id:string;from:string;to:string}[]}
export const MAP_LIMIT=3_400_000, FILE_LIMIT=700_000
export const fileTypes:Record<string,string>={pdf:'application/pdf',txt:'text/plain',md:'text/markdown',docx:'application/vnd.openxmlformats-officedocument.wordprocessingml.document'}
export function mapNode(kind:MapKind,title:string,x:number,y:number):MapNode{return {id:makeId(),kind,title,text:'',x,y,items:[],images:[],files:[]}}
export function newMindMap(title='Мой проект',text=''):MindMap {
 const center=mapNode('idea',title||'Мой проект',540,380);center.text=text
 const goal=mapNode('goal','Цель',100,160);goal.text='Какого результата хотите достичь?'
 const ideas=mapNode('idea','Идеи',130,650)
 const images=mapNode('images','Вдохновение',1020,100)
 const plan=mapNode('plan','План действий',1020,390);plan.items=['Собрать идеи','Сделать первый шаг','Проверить результат'].map(text=>({id:makeId(),text,done:false}))
 const notes=mapNode('note','Материалы',1020,720)
 const nodes=[center,goal,ideas,images,plan,notes]
 return {version:1,nodes,edges:nodes.slice(1).map(node=>({id:makeId(),from:center.id,to:node.id}))}
}
export function mapText(map:MindMap):string{return map.nodes.map(node=>[node.title,node.text,...node.items.map(item=>(item.done?'☑ ':'☐ ')+item.text),...node.images.map(image=>'Изображение: '+image.name),...node.files.map(file=>'Файл: '+file.name)].filter(Boolean).join('\n')).join('\n\n')}
export function nodeWidth(node:MapNode){return node.width??300}
export function nodeHeight(node:MapNode){return node.height??(100+(node.text?Math.min(3,Math.ceil(node.text.length/30))*22:0)+Math.min(4,node.items.length)*30+(node.images.length?(node.imageHeight??110):0)+(node.files.length?38:0))}
export function mapBounds(map:MindMap){const xs=map.nodes.map(n=>n.x),ys=map.nodes.map(n=>n.y);const x=Math.min(...xs),y=Math.min(...ys);return {x,y,width:Math.max(...map.nodes.map(n=>n.x+nodeWidth(n)))-x,height:Math.max(...map.nodes.map(n=>n.y+nodeHeight(n)))-y}}
export function validateMindMap(value:unknown):MindMap {
 const fail=()=>{throw new Error('Карта мыслей повреждена или превышает допустимый размер. Исходная заметка сохранена.')}
 if(!value||typeof value!=='object')return fail()
 const map=value as MindMap
 if(map.version!==1||!Array.isArray(map.nodes)||!map.nodes.length||map.nodes.length>80||!Array.isArray(map.edges)||map.edges.length>160)return fail()
 const text=(v:unknown,max:number)=>typeof v==='string'&&v.length<=max
 const ids=new Set<string>()
 for(const n of map.nodes){
  if(!n||!text(n.id,128)||!n.id||ids.has(n.id)||!['idea','goal','plan','images','note'].includes(n.kind)||!text(n.title,120)||!text(n.text,4000)||!Number.isFinite(n.x)||!Number.isFinite(n.y)||Math.abs(n.x)>10000||Math.abs(n.y)>10000)return fail()
  for(const [key,min,max] of [['width',240,1000],['height',160,1400],['imageHeight',80,800]] as const){if(n[key]!==undefined&&(!Number.isFinite(n[key])||n[key]!<min||n[key]!>max))return fail()}
  if(n.imageFit!==undefined&&!['contain','cover'].includes(n.imageFit))return fail()
  ids.add(n.id)
  if(!Array.isArray(n.items)||n.items.length>100||!Array.isArray(n.images)||n.images.length>8||!Array.isArray(n.files)||n.files.length>8)return fail()
  for(const list of [n.items,n.images,n.files]){const keys=new Set<string>();for(const item of list){if(!item||!text(item.id,128)||!item.id||keys.has(item.id))return fail();keys.add(item.id)}}
  if(n.items.some(i=>!text(i.text,1000)||typeof i.done!=='boolean'))return fail()
  if(n.images.some(i=>!text(i.name,160)||!text(i.src,2_800_000)||!/^data:image\/(jpeg|png|webp|gif);base64,[a-zA-Z0-9+/]+={0,2}$/.test(i.src)))return fail()
  if(n.files.some(f=>!text(f.name,160)||!Object.values(fileTypes).includes(f.mime)||!Number.isInteger(f.size)||f.size<1||f.size>FILE_LIMIT||!text(f.data,940000)||!f.data||f.data.length%4!==0||!/[A-Za-z0-9+/]/.test(f.data)||!/^[A-Za-z0-9+/]+={0,2}$/.test(f.data)||Math.floor(f.data.length*3/4)-(f.data.endsWith('==')?2:f.data.endsWith('=')?1:0)!==f.size))return fail()
 }
 const pairs=new Set<string>(),edgeIds=new Set<string>()
 for(const e of map.edges){const pair=[e?.from,e?.to].sort().join(':');if(!e||!text(e.id,128)||!e.id||edgeIds.has(e.id)||!ids.has(e.from)||!ids.has(e.to)||e.from===e.to||pairs.has(pair))return fail();pairs.add(pair);edgeIds.add(e.id)}
 if(new TextEncoder().encode(JSON.stringify(map)).length>MAP_LIMIT)throw new Error('Карта переполнена. Уменьшите размер изображений или удалите часть вложений.')
 return map
}
export async function readMapFile(file:File):Promise<MapFile>{
 const extension=file.name.split('.').pop()?.toLowerCase()||'',mime=fileTypes[extension]
 if(!mime)throw new Error('Можно прикрепить PDF, TXT, Markdown или Word (.docx).')
 if(!file.size||file.size>FILE_LIMIT)throw new Error('Файл должен быть меньше 700 КБ. Большие материалы можно добавить ссылкой в текст карточки.')
 const data=await new Promise<string>((resolve,reject)=>{const r=new FileReader();r.onload=()=>resolve(String(r.result).split(',')[1]);r.onerror=()=>reject(new Error('Файл не открылся'));r.readAsDataURL(file)})
 return {id:makeId(),name:file.name.slice(0,160),mime,data,size:file.size}
}

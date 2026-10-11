import { renderNote } from './noteFormatting'
export const exportFormats=[{id:'txt',label:'Текст (.txt)'},{id:'docx',label:'Word (.docx)'},{id:'html',label:'Веб-документ (.html)'},{id:'md',label:'Markdown (.md)'}]
export function exportHtml(text:string) {
    return `<!doctype html><html lang="ru"><head><meta charset="utf-8"><title>Результат JARVIS</title><style>body{font-family:Calibri,sans-serif;margin:40px;color:#17232b;line-height:1.5}table{border-collapse:collapse;width:100%;margin:20px 0}th,td{border:1px solid #a4b4bb;padding:10px;text-align:left;vertical-align:top;overflow-wrap:anywhere}th{background:#e8eff2}p{margin:8px 0}h2,h3,h4{color:#000}</style></head><body>${renderNote(text,true,true)}</body></html>`
}
export function exportBlocks(text:string) {
    const doc=new DOMParser().parseFromString(renderNote(text,true,true),'text/html')
    const runs=(element:Element)=>{
        const result:{text:string;bold:boolean;italic:boolean}[]=[]
        function walk(node:Node,bold=false,italic=false) {
            if(node.nodeType===Node.TEXT_NODE) result.push({text:node.textContent || '',bold,italic})
            else if(node instanceof Element) {
                if(node.tagName==='BR') result.push({text:'\n',bold,italic})
                else node.childNodes.forEach(child=>walk(child,bold||node.tagName==='STRONG',italic||node.tagName==='EM'))
            }
        }
        walk(element);return result
    }
    return Array.from(doc.body.children).map(element=>element.tagName==='TABLE'
        ? {kind:'table',rows:Array.from(element.querySelectorAll('tr')).map(row=>Array.from(row.children).map(cell=>runs(cell)))}
        : {kind:'paragraph',level:/^H[2-4]$/.test(element.tagName)?Number(element.tagName[1])-1:0,runs:runs(element)})
}

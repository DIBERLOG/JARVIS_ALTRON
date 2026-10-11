import type { ChecklistItem } from "./center"

export function textToChecklist(text: string, createId: () => string): ChecklistItem[] {
    return text.split(/\r?\n/).filter(line => line.trim()).map(line => ({
        id: createId(),
        done: /^\s*(?:-\s*\[[xX]\]|☑)/.test(line),
        text: line.replace(/^\s*(?:-\s*\[[ xX]\]\s*|[☑☐]\s*|[-*•]\s+|\d+\.\s+|#{1,3}\s+)/, "").trim()
    }))
}

export function checklistToText(items: ChecklistItem[]): string {
    return items.map(item => `- [${item.done ? "x" : " "}] ${item.text}`).join("\n")
}

// Escape all user input before adding only known formatting tags. No raw HTML or links.
export function renderNote(text: string, formatted: boolean, tables = false): string {
    const escaped = text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;").replace(/'/g, "&#39;")
    if (!formatted) return escaped.replace(/\n/g, "<br>")
    function inline(line: string) {
        return line.replace(/\*\*([^*\n]+)\*\*/g, "<strong>$1</strong>")
            .replace(/\*([^*\n]+)\*/g, "<em>$1</em>")
            .replace(/~~([^~\n]+)~~/g, "<s>$1</s>")
    }
    const lines=escaped.split(/\r?\n/), output:string[]=[]
    const cells=(line:string)=>line.trim().replace(/^\|/, '').replace(/\|$/, '').split(/(?<!\\)\|/).map(cell=>cell.trim().replace(/\\\|/g,'|'))
    for(let i=0;i<lines.length;i++) {
        const line=lines[i]
        const header=cells(line), divider=i+1<lines.length?cells(lines[i+1]):[]
        if(tables && line.includes('|') && header.length>1 && divider.length===header.length && divider.every(cell=>/^:?-+:?$/.test(cell))) {
            const row=(values:string[],tag:string)=>`<tr>${header.map((_,column)=>`<${tag}>${inline(values[column] || '')}</${tag}>`).join('')}</tr>`
            let table=`<table><thead>${row(header,'th')}</thead><tbody>`
            i++
            while(i+1<lines.length && lines[i+1].includes('|') && lines[i+1].trim()) { table+=row(cells(lines[++i]),'td') }
            output.push(table+'</tbody></table>');continue
        }
        const heading = /^(#{1,3})\s+(.*)$/.exec(line)
        if (heading) {output.push(`<h${heading[1].length + 1}>${inline(heading[2])}</h${heading[1].length + 1}>`);continue}
        const check = /^-\s*\[([ xX])\]\s+(.*)$/.exec(line)
        if (check) {output.push(`<p>${check[1].toLowerCase() === "x" ? "☑" : "☐"} ${inline(check[2])}</p>`);continue}
        if (/^[-*]\s+/.test(line)) {output.push(`<p>• ${inline(line.replace(/^[-*]\s+/, ""))}</p>`);continue}
        output.push(`<p>${inline(line) || "<br>"}</p>`)
    }
    return output.join('')
}

export function formatSelection(text: string, start: number, end: number, style: string) {
    const selected = text.slice(start, end)
    const wrappers: Record<string, string> = { bold: "**", italic: "*", strike: "~~" }
    if (wrappers[style]) {
        const marker = wrappers[style], content = selected || "текст"
        return { text: text.slice(0, start) + marker + content + marker + text.slice(end), start: start + marker.length, end: start + marker.length + content.length }
    }
    const lineStart = text.lastIndexOf("\n", start - 1) + 1
    const nextNewline = text.indexOf("\n", end)
    const lineEnd = nextNewline < 0 ? text.length : nextNewline
    const prefix = style === "heading" ? "## " : "- "
    const content = text.slice(lineStart, lineEnd).split("\n").map(line => prefix + line).join("\n")
    return { text: text.slice(0, lineStart) + content + text.slice(lineEnd), start: lineStart, end: lineStart + content.length }
}

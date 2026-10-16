<script lang="ts">
    import RichNoteEditor from "./RichNoteEditor.svelte"
    import MindMapEditor from "./MindMapEditor.svelte"
    import {newMindMap,mapText,validateMindMap,type MindMap} from "@/lib/mindMap"
    import { makeId, type Note, type ChecklistItem } from "@/lib/center"
    import { textToChecklist, checklistToText, renderNote } from "@/lib/noteFormatting"
    import { sanitizeNoteHtml, NOTE_HTML_LIMIT } from "@/lib/richNotes"
    export let notes: Note[] = []
    export let disabled = false
    export let openNoteId = ""
    export let mobile = false
    export let onSave: (note: Note) => Promise<boolean>
    export let onDelete: (id: string) => Promise<boolean>
    export let onDirty: (dirty: boolean) => void = () => {}
    let id = "", title = "", text = "", items: ChecklistItem[] = []
    let type: "text" | "checklist" | "mindmap" = "text"
    let mindMap:MindMap|undefined
    let html = "", dirty = false, localError = "", editorKey = 0
    $: onDirty(dirty)
    $: if(openNoteId && notes.some(note=>note.id===openNoteId) && !dirty){const target=openNoteId;openNoteId="";open(notes.find(note=>note.id===target))}
    $: ordered = [...notes].sort((a, b) => b.updatedAt.localeCompare(a.updatedAt))
    $: completed = items.filter(item => item.done).length
    function canLeave() { return !dirty || window.confirm("Есть несохранённые изменения. Перейти без сохранения?") }
    export function requestLeave() { return canLeave() }
    export function hasChanges() { return dirty }
    export function selectedId() { return id }
    export function selectId(value: string) { open(notes.find(note => note.id === value)) }
    function open(note?: Note) {
        if (!canLeave()) return
        id = note?.id ?? ""; title = note?.title ?? ""; text = note?.text ?? ""
        type = note?.type === "mindmap" ? "mindmap" : note?.type === "checklist" ? "checklist" : "text"
        let mapError="";mindMap=undefined
        if(type==="mindmap"){try{mindMap=structuredClone(validateMindMap(note?.mindMap))}catch(e){mapError=String(e)}}
        html = note?.format === "rich" ? sanitizeNoteHtml(note.html ?? "") : renderNote(text, note?.format === "markdown")
        items = note?.items?.map(item => ({ ...item })) ?? (type === "checklist" ? textToChecklist(text, makeId) : [])
        editorKey++; dirty = false; localError = mapError
    }
    function changeType(next: "text" | "checklist" | "mindmap") {
        if (next === type) return
        if(type==='mindmap'){
            if(!mindMap){localError='Повреждённую карту нельзя преобразовать. Исходная заметка сохранена.';return}
            if(!window.confirm('Преобразовать карту в текст? Расположение карточек, связи и вложения не сохранятся.'))return
            text=mapText(mindMap);html=renderNote(text,false)
        }
        if(next==='mindmap'){
            try{const map=newMindMap(title,type==='checklist'?checklistToText(items):text);const images=Array.from(new DOMParser().parseFromString(sanitizeNoteHtml(html),'text/html').querySelectorAll('img'));map.nodes[0].images=images.map(image=>({id:makeId(),name:(image.alt||'Изображение').slice(0,160),src:image.src}));mindMap=validateMindMap(map)}catch(e){localError=String(e);return}
        } else if (next === "checklist") {
            if (/<img\b/.test(html) && !window.confirm("Чек-лист сохраняет только текст. Картинки и оформление не попадут в него. Продолжить?")) return
            items = textToChecklist(text, makeId)
        } else if(type==='checklist') { text = checklistToText(items); html = renderNote(text, true) }
        type = next; editorKey++; dirty = true
    }
    function addItem() {
        if (items.length >= 200) { localError = "Максимум 200 пунктов"; return }
        items = [...items, { id: makeId(), text: "", done: false }]; dirty = true
    }
    async function save() {
        localError = ""
        const cleanItems = items.filter(item => item.text.trim()).map(item => ({ ...item, text: item.text.trim() }))
        if (type!=="mindmap" && (!title.trim() || (type === "text" ? !text.trim() && !/<img\b/.test(html) : !cleanItems.length))) { localError = "Укажите заголовок и добавьте текст, картинку или хотя бы один пункт"; return }
        if(type==='mindmap'){
            try{const map=validateMindMap(mindMap);if(!title.trim()){localError='Укажите название карты';return}const note:Note={id:id||makeId(),title:title.trim(),text:mapText(map).slice(0,100000),type:'mindmap',format:'plain',mindMap:structuredClone(map),updatedAt:new Date().toISOString()};if(await onSave(note)){id=note.id;dirty=false}else localError='Не удалось сохранить карту. Изменения остались в редакторе; повторите сохранение.';return}catch(e){localError=String(e);return}
        }
        const safeHtml = type === "text" ? sanitizeNoteHtml(html) : undefined
        if ((safeHtml?.length ?? 0) > NOTE_HTML_LIMIT) { localError = "Максимум 8 МБ на заметку. Удалите часть картинок."; return }
        const note: Note = { id: id || makeId(), title: title.trim(), text: type === "checklist" ? checklistToText(cleanItems) : text.trim(), type, format: type === "text" ? "rich" : "plain", html: safeHtml, items: type === "checklist" ? cleanItems : undefined, updatedAt: new Date().toISOString() }
        if (await onSave(note)) { id = note.id; items = cleanItems; dirty = false }
    }
    async function remove() {
        if (!window.confirm("Удалить эту заметку?")) return
        if (await onDelete(id)) { dirty = false; open() }
    }
</script>

<section class="notes-panel" class:map-mode={type==='mindmap'} class:mobile-empty={mobile && !notes.length} aria-label="Заметки">
    <header><div><p class="eyebrow">МЫСЛИ И ПЛАНЫ</p><h2>Заметки</h2></div><button type="button" class="primary" disabled={disabled} on:click={() => open()}>+ Новая</button></header>
    <div class="layout">
        <div class="note-list" aria-label="Сохранённые заметки">
            {#if !notes.length}<p class="empty">Создайте заметку, чек-лист или карту мыслей.</p>{/if}
            {#each ordered as note}<button type="button" class:active={note.id === id} disabled={disabled} on:click={() => open(note)}><strong>{note.title}</strong><small>{note.type === "checklist" ? `☑ Чек-лист · ${(note.items ?? []).filter(item => item.done).length}/${note.items?.length ?? 0}` : note.type === "mindmap" ? `Карта мыслей · ${note.mindMap?.nodes.length??0} карточек` : "✎ Заметка"}</small><small>{new Date(note.updatedAt).toLocaleDateString("ru-RU")}</small></button>{/each}
        </div>
        <form on:submit|preventDefault={save}>
            <fieldset disabled={disabled}>
                <div class="type-switch" aria-label="Тип заметки"><button type="button" class:active={type === "text"} aria-pressed={type === "text"} on:click={() => changeType("text")}>✎ Обычная</button><button type="button" class:active={type === "checklist"} aria-pressed={type === "checklist"} on:click={() => changeType("checklist")}>☑ Чек-лист</button><button type="button" class:active={type === "mindmap"} aria-pressed={type === "mindmap"} on:click={() => changeType("mindmap")}>Карта мыслей</button></div>
                <input aria-label="Заголовок заметки" maxlength="100" placeholder="Заголовок" bind:value={title} on:input={() => dirty = true} required />
                {#if type === "text"}
                    {#key editorKey}<RichNoteEditor {mobile} bind:html bind:plainText={text} {disabled} on:change={() => dirty = true} />{/key}
                {:else if type==='mindmap'}
                    {#if mindMap}{#key editorKey}<MindMapEditor {mobile} {disabled} bind:value={mindMap} {dirty} saveError={localError} on:clearError={()=>localError=''} on:save={save} on:change={()=>dirty=true}/>{/key}{/if}
                {:else}
                    <div class="progress"><span>Выполнено {completed} из {items.length}</span><progress value={completed} max={items.length || 1}></progress></div>
                    <div class="checklist">
                        {#each items as item, index (item.id)}<div class="check-row" class:done={item.done}><input type="checkbox" aria-label={`Выполнено: ${item.text || `пункт ${index + 1}`}`} bind:checked={item.done} on:change={() => dirty = true} /><input aria-label={`Текст пункта ${index + 1}`} maxlength="1000" placeholder="Что нужно сделать?" bind:value={item.text} on:input={() => dirty = true} /><button type="button" class="remove" aria-label={`Удалить пункт ${index + 1}`} on:click={() => { items = items.filter(row => row.id !== item.id); dirty = true }}>×</button></div>{/each}
                    </div>
                    <button type="button" class="add-item" on:click={addItem}>+ Добавить пункт</button>
                    <small>Отметки и изменения сохраняются кнопкой ниже.</small>
                {/if}
                {#if localError}<p class="error" role="alert">{localError}</p>{/if}
                <div class="actions"><button class="primary">{id ? "Сохранить изменения" : "Создать заметку"}</button>{#if id}<button type="button" class="delete" on:click={remove}>Удалить</button>{/if}</div>
                <small aria-live="polite">{dirty ? "Есть несохранённые изменения" : id ? "Сохранено" : "Новая заметка"}</small>
            </fieldset>
        </form>
    </div>
</section>

<style lang="scss">
 .map-mode .layout{grid-template-columns:1fr}.map-mode .note-list{flex-direction:row;max-height:85px;overflow:auto}.map-mode .note-list button{min-width:170px;max-width:230px;flex:none}.map-mode .note-list small:last-child{display:none}
 .note-list button.active strong{color:#09282d}.note-list button.active small{color:#204c50;font-weight:600}.note-list button strong{font-size:1rem;line-height:1.4}.note-list button small{font-size:.76rem;line-height:1.5}.note-list button{gap:.4rem}

    .notes-panel {--accent:#60f3e9;--line:#295056;padding:1rem;border:1px solid var(--line);border-radius:11px;background:linear-gradient(140deg,#143036aa,transparent 55%),#0d1b20;color:#eafafa;font-family:"Manrope Variable",sans-serif}
    header {display:flex;justify-content:space-between;align-items:center;gap:.5rem;margin-bottom:1rem}.eyebrow {margin:0;color:var(--accent);font-size:.6rem;font-weight:800;letter-spacing:.14em}h2 {margin:.3rem 0 0;font-size:1.15rem}
    .layout {display:grid;grid-template-columns:135px minmax(0,1fr);gap:.7rem}.note-list {display:flex;flex-direction:column;gap:.4rem;max-height:440px;overflow:auto}.note-list button {display:flex;flex-direction:column;flex-shrink:0;gap:.25rem;text-align:left}.note-list strong {overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-size:.72rem}
    button {border:1px solid var(--line);border-radius:6px;background:#112a2f;color:#c4dedf;padding:.5rem .6rem;font:600 .68rem "Manrope Variable",sans-serif;cursor:pointer}button:hover:not(:disabled) {border-color:var(--accent)}button.active {border-color:var(--accent);background:#155258;color:#fff}button:focus-visible {outline:2px solid var(--accent);outline-offset:2px}button:disabled {opacity:.5;cursor:default}
    form,fieldset {min-width:0}fieldset {display:flex;flex-direction:column;gap:.55rem;border:0;margin:0;padding:0}.type-switch {display:flex;gap:.4rem}.type-switch button {flex:1}.primary {border-color:var(--accent);background:#1aa7a8;color:#061617;font-weight:800}
    input:not([type="checkbox"]),textarea {width:100%;min-width:0;padding:.6rem;border:1px solid var(--line);border-radius:6px;background:#09171b;color:#e7ffff;font:500 .73rem "Manrope Variable",sans-serif;outline:none}input:focus,textarea:focus {border-color:var(--accent)}textarea {min-height:230px;resize:vertical;line-height:1.6}.toolbar {display:flex;flex-wrap:wrap;gap:.25rem}.toolbar button {padding:.35rem .45rem}.preview-toggle {margin-left:auto}
    .preview {padding:.7rem;min-height:230px;border:1px solid var(--line);border-radius:6px;background:#09171b;overflow-wrap:anywhere;font-size:.78rem;line-height:1.65}.preview :global(p) {margin:.25rem 0}.preview :global(h2),.preview :global(h3),.preview :global(h4) {margin:.6rem 0 .3rem;color:#bafff4}
    small,.empty {color:#8eb0b3;font-size:.6rem;line-height:1.5}.progress {display:flex;flex-direction:column;gap:.4rem;color:#a9d8d4;font-size:.7rem}progress {width:100%;height:5px;accent-color:var(--accent)}progress::-webkit-progress-bar {background:#254348;border-radius:4px}progress::-webkit-progress-value {background:var(--accent);border-radius:4px}
    .checklist {display:flex;flex-direction:column;gap:.4rem;max-height:300px;overflow:auto}.check-row {display:flex;align-items:center;gap:.4rem}.check-row input[type="checkbox"] {width:17px;height:17px;flex:none;accent-color:var(--accent)}.check-row.done input:not([type="checkbox"]) {text-decoration:line-through;color:#719793}.remove {padding:.35rem;color:#f8b2b2;border:0;background:transparent;font-size:1rem}.add-item {border-style:dashed;color:var(--accent)}.actions {display:flex;flex-wrap:wrap;gap:.4rem;margin-top:.2rem}.delete {color:#ffb3b3;border-color:#7c4549;background:#301b20}.error {color:#ffb3b3;font-size:.7rem}
    @media (max-width: 580px) {.layout {grid-template-columns:1fr}.note-list {flex-direction:row;max-height:100px}.note-list button {min-width:130px;max-width:180px}}
    button {min-height:35px;font-size:.76rem;padding:.55rem .7rem}
 .note-list button.active{background:#a0e4ce;border-color:#76baa8;color:#09282d}
</style>

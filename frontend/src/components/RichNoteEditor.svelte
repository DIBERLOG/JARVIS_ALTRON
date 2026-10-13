<script lang="ts">
    import { onMount, onDestroy, createEventDispatcher } from "svelte"
    import { Editor } from "@tiptap/core"
    import StarterKit from "@tiptap/starter-kit"
    import ImageExtension from "@tiptap/extension-image"
    import { TextStyleKit } from "@tiptap/extension-text-style"
    import TextAlign from "@tiptap/extension-text-align"
    import Highlight from "@tiptap/extension-highlight"
    import { sanitizeNoteHtml, readImage, NOTE_HTML_LIMIT } from "@/lib/richNotes"
    export let html = ""
    export let plainText = ""
    export let disabled = false
    export let mobile = false
    let expanded = false
    let host: HTMLDivElement, picker: HTMLInputElement, editor: Editor | undefined
    let revision = 0, imageBusy = false, error = ""
    const dispatch = createEventDispatcher()
    function chooseImage(){
        if(mobile && (window as any).NativeCenter?.pickImage){imageBusy=true;error='';(window as any).NativeCenter.pickImage();}
        else picker.click();
    }
    function nativeImage(event:Event){
        imageBusy=false;
        const result=(event as CustomEvent).detail;
        if(result?.error){error=result.error;return;}
        if(result?.src && /^data:image\/jpeg;base64,[a-zA-Z0-9+/=]+$/.test(result.src))editor?.chain().focus().setImage({src:result.src}).run();
    }
    onMount(()=>{if(mobile)window.addEventListener('jarvis-image',nativeImage);return()=>window.removeEventListener('jarvis-image',nativeImage);});
    const buttons = [
        { name: "Жирный текст", label: "B", mark: "bold", action: "toggleBold" },
        { name: "Курсив", label: "I", mark: "italic", action: "toggleItalic" },
        { name: "Подчёркнутый текст", label: "U", mark: "underline", action: "toggleUnderline" },
        { name: "Зачёркнутый текст", label: "S̶", mark: "strike", action: "toggleStrike" },
        { name: "Выделить маркером", label: "▰", mark: "highlight", action: "toggleHighlight" },
        { name: "Маркированный список", label: "• Список", mark: "bulletList", action: "toggleBulletList" },
        { name: "Нумерованный список", label: "1. Список", mark: "orderedList", action: "toggleOrderedList" },
        { name: "Цитата", label: "❝", mark: "blockquote", action: "toggleBlockquote" },
        { name: "Строка кода", label: "</>", mark: "code", action: "toggleCode" },
        { name: "Блок кода", label: "Код", mark: "codeBlock", action: "toggleCodeBlock" },
    ]
    function run(command: string) {
        if (!editor || disabled) return
        const chain = editor.chain().focus() as any
        chain[command]().run()
    }
    function setHeading(event: Event) {
        const level = Number((event.currentTarget as HTMLSelectElement).value)
        if (level) editor?.chain().focus().setHeading({ level: level as 1 | 2 | 3 }).run()
        else editor?.chain().focus().setParagraph().run()
    }
    function link() {
        if (!editor) return
        const url = window.prompt("Адрес ссылки (https://…); пустая строка — убрать ссылку", editor.getAttributes("link").href ?? "")
        if (url === null) return
        if (!url.trim()) { editor.chain().focus().extendMarkRange("link").unsetLink().run(); return }
        if (!/^(https?:\/\/|mailto:)/i.test(url.trim())) { error = "Введите ссылку с https://, http:// или mailto:"; return }
        editor.chain().focus().extendMarkRange("link").setLink({ href: url.trim() }).run()
    }
    async function insertImages(files: File[]) {
        if (!editor || disabled || imageBusy) return
        const target = editor
        const initialHtml = html
        let position = target.state.selection.from
        imageBusy = true; error = ""
        try {
            for (const file of files) {
                const src = await readImage(file)
                if (target.isDestroyed || (html !== initialHtml && target.getHTML() !== html)) return
                if (target.getHTML().length + src.length > NOTE_HTML_LIMIT) throw new Error("Заметка превышает 8 МБ. Удалите часть картинок.")
                position = Math.min(position, target.state.doc.content.size)
                target.chain().focus().insertContentAt(position, { type: "image", attrs: { src, alt: file.name || "Из буфера обмена" } }).run()
                position = target.state.selection.to
            }
        } catch (e) { error = String(e) }
        finally { imageBusy = false }
    }
    onMount(() => {
        editor = new Editor({
            element: host,
            extensions: [StarterKit.configure({ heading: { levels: [1, 2, 3] }, link: { openOnClick: false } }), ImageExtension.configure({ allowBase64: true }), TextStyleKit, TextAlign.configure({ types: ["heading", "paragraph"] }), Highlight.configure({ multicolor: true })],
            content: sanitizeNoteHtml(html), editable: !disabled,
            editorProps: {
                attributes: { role: "textbox", "aria-label": "Текст заметки", "aria-multiline": "true", spellcheck: "true" },
                transformPastedHTML: sanitizeNoteHtml,
                handlePaste: (_, event) => {
                    const images = Array.from(event.clipboardData?.items ?? []).filter(item => item.kind === "file" && item.type.startsWith("image/")).map(item => item.getAsFile()).filter((file): file is File => !!file)
                    if (!images.length) return false
                    event.preventDefault(); void insertImages(images); return true
                },
                handleDrop: (_, event) => {
                    const images = Array.from(event.dataTransfer?.files ?? []).filter(file => file.type.startsWith("image/"))
                    if (!images.length) return false
                    event.preventDefault(); void insertImages(images); return true
                },
            },
            onUpdate: ({ editor: current }) => { html = current.getHTML(); plainText = current.getText({ blockSeparator: "\n" }); dispatch("change"); revision++ },
            onSelectionUpdate: () => revision++,
            onTransaction: () => revision++,
        })
        plainText = editor.getText({ blockSeparator: "\n" })
    })
    $: if (editor && !editor.isDestroyed && html !== editor.getHTML()) { editor.commands.setContent(sanitizeNoteHtml(html), { emitUpdate: false }); plainText = editor.getText({ blockSeparator: "\n" }) }
    $: if (editor && !editor.isDestroyed) editor.setEditable(!disabled, false)
    onDestroy(() => editor?.destroy())
</script>

<div class="rich-editor" class:mobile-compact={mobile} class:expanded data-revision={revision}>
    {#if mobile}<button class="format-toggle" type="button" aria-expanded={expanded} on:click={() => expanded = !expanded}>{expanded ? 'Скрыть оформление' : 'Ещё оформление'}</button>{/if}
    <div class="toolbar" aria-label="Форматирование заметки">
        <select aria-label="Стиль абзаца" disabled={disabled} on:change={setHeading}><option value="0">Абзац</option><option value="1">Заголовок 1</option><option value="2">Заголовок 2</option><option value="3">Заголовок 3</option></select>
        <select aria-label="Размер текста" disabled={disabled} on:change={(event) => editor?.chain().focus().setFontSize(event.currentTarget.value).run()}>{#each [12, 14, 16, 18, 20, 24, 28, 32] as size}<option value={`${size}px`} selected={size === 16}>{size}</option>{/each}</select>
        {#each buttons as button}<button type="button" aria-label={button.name} title={button.name} aria-pressed={revision >= 0 && (editor?.isActive(button.mark) ?? false)} disabled={disabled} on:mousedown|preventDefault on:click={() => run(button.action)}>{button.label}</button>{/each}
        <label class="color" title="Цвет текста">A<input type="color" aria-label="Цвет текста" value="#60f3e9" disabled={disabled} on:input={(event) => editor?.chain().focus().setColor(event.currentTarget.value).run()} /></label>
        {#each [{ id: "left", label: "≡←", name: "По левому краю" }, { id: "center", label: "≡", name: "По центру" }, { id: "right", label: "→≡", name: "По правому краю" }, { id: "justify", label: "↔", name: "По ширине" }] as alignment}<button type="button" aria-label={alignment.name} title={alignment.name} disabled={disabled} on:mousedown|preventDefault on:click={() => editor?.chain().focus().setTextAlign(alignment.id).run()}>{alignment.label}</button>{/each}
        <button type="button" title="Ссылка" aria-label="Добавить ссылку" disabled={disabled} on:mousedown|preventDefault on:click={link}>↗ Ссылка</button>
        <button type="button" title="Разделитель" aria-label="Вставить разделитель" disabled={disabled} on:mousedown|preventDefault on:click={() => run("setHorizontalRule")}>―</button>
        <button type="button" aria-label="Вставить картинку" disabled={disabled || imageBusy} on:mousedown|preventDefault on:click={chooseImage}>▧ Картинка</button>
        <button type="button" aria-label="Отменить действие" title="Отмена · Ctrl+Z" disabled={disabled || (revision >= 0 && !editor?.can().undo())} on:mousedown|preventDefault on:click={() => run("undo")}>↶</button>
        <button type="button" aria-label="Повторить действие" title="Повтор · Ctrl+Shift+Z" disabled={disabled || (revision >= 0 && !editor?.can().redo())} on:mousedown|preventDefault on:click={() => run("redo")}>↷</button>
        <button type="button" aria-label="Очистить форматирование" disabled={disabled} on:mousedown|preventDefault on:click={() => editor?.chain().focus().unsetAllMarks().clearNodes().run()}>Tx</button>
    </div>
    <input class="file-picker" type="file" accept="image/png,image/jpeg,image/webp,image/gif,image/bmp" multiple bind:this={picker} on:change={(event) => { void insertImages(Array.from(event.currentTarget.files ?? [])); event.currentTarget.value = "" }} />
    <div class="document" bind:this={host}></div>
    <small>{imageBusy ? "Подготавливаю картинку…" : mobile ? "Пишите текст. Оформление и картинка — кнопками выше." : "Оформление видно сразу · Ctrl+B / I / U · Ctrl+V — текст или картинка"}</small>
    {#if error}<p class="error" role="alert">{error}</p>{/if}
</div>

<style lang="scss">
    .mobile-compact:not(.expanded) .toolbar select,.mobile-compact:not(.expanded) .toolbar .color,.mobile-compact:not(.expanded) .toolbar button:not([aria-label="Жирный текст"]):not([aria-label="Курсив"]):not([aria-label="Маркированный список"]):not([aria-label="Вставить картинку"]):not([aria-label="Отменить действие"]) { display:none; }
    .format-toggle { border:0; background:transparent; color:var(--accent); padding:8px 0; font:inherit; font-size:13px; }
    .rich-editor {--accent:#60f3e9;min-width:0}.toolbar {display:flex;flex-wrap:wrap;gap:.35rem;padding:.5rem;border:1px solid #31555b;border-radius:8px 8px 0 0;background:#10292f}
    button,select,.color {min-height:35px;padding:.45rem .6rem;border:1px solid #376268;border-radius:6px;background:#15373d;color:#defafa;font:600 .78rem "Manrope Variable",sans-serif;cursor:pointer}button[aria-pressed="true"] {border-color:var(--accent);background:#21636a;color:#fff}button:hover:not(:disabled) {border-color:var(--accent)}button:disabled {opacity:.4;cursor:default}button:focus-visible,select:focus-visible {outline:2px solid var(--accent);outline-offset:2px}.color {display:flex;align-items:center;gap:.35rem}.color input {width:20px;height:20px;border:0;padding:0;background:none;cursor:pointer}.file-picker {display:none}
    .document {padding:.85rem;min-height:280px;max-height:540px;overflow:auto;border:1px solid #31555b;border-top:0;border-radius:0 0 8px 8px;background:#09171b;color:#e7ffff;font:400 16px/1.6 "Manrope Variable",sans-serif;overflow-wrap:anywhere}
    .document :global(.tiptap) {min-height:250px;outline:none}.document:focus-within {border-color:var(--accent)}.document :global(p) {margin:.35rem 0}.document :global(h1) {font-size:1.8em}.document :global(h2) {font-size:1.45em}.document :global(h3) {font-size:1.2em}.document :global(h1),.document :global(h2),.document :global(h3) {margin:.6em 0 .3em;font-weight:700;line-height:1.25}.document :global(ul) {list-style:disc;padding-left:1.4rem}.document :global(ol) {list-style:decimal;padding-left:1.4rem}.document :global(blockquote) {border-left:3px solid var(--accent);padding:.4rem .8rem;margin:.7rem 0;background:#17363a;color:#b9e6e2}.document :global(pre) {padding:.8rem;background:#040e12;border-radius:7px;white-space:pre-wrap}.document :global(code) {font-family:monospace;background:#163035;border-radius:3px}.document :global(a) {color:#60f3e9;text-decoration:underline}.document :global(img) {display:block;max-width:100%;height:auto;margin:.7rem 0;border-radius:7px}.document :global(.ProseMirror-selectednode) {outline:2px solid var(--accent)}.document :global(hr) {border:0;border-top:1px solid #45676b;margin:1rem 0}.document :global(mark) {color:#101916;background:#f4e98b;padding:0 .15em}small {display:block;margin-top:.5rem;color:#8eb0b3;font-size:.66rem}.error {color:#ffb3b3;font-size:.75rem}
</style>

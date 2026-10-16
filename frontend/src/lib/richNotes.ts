import DOMPurify from "dompurify"

export const NOTE_HTML_LIMIT = 8_000_000
export function sanitizeNoteHtml(html: string): string {
    const safe = DOMPurify.sanitize(html, {
        ALLOWED_TAGS: ["p", "br", "strong", "b", "em", "i", "u", "s", "strike", "h1", "h2", "h3", "blockquote", "pre", "code", "ul", "ol", "li", "hr", "a", "span", "mark", "img"],
        ALLOWED_ATTR: ["href", "src", "alt", "title", "style", "start"],
        ALLOW_DATA_ATTR: false,
    })
    const doc = new DOMParser().parseFromString(safe, "text/html")
    for (const image of Array.from(doc.querySelectorAll("img"))) {
        if (!/^data:image\/(png|jpeg|webp|gif);base64,[a-zA-Z0-9+/=]+$/.test(image.getAttribute("src") ?? "")) image.remove()
    }
    for (const anchor of Array.from(doc.querySelectorAll("a"))) {
        if (!/^(https?:\/\/|mailto:)/i.test(anchor.getAttribute("href") ?? "")) anchor.removeAttribute("href")
    }
    const allowedStyles = new Set(["color", "background-color", "font-size", "font-family", "line-height", "text-align"])
    for (const element of Array.from(doc.querySelectorAll<HTMLElement>("[style]"))) {
        for (const property of Array.from(element.style)) {
            if (!allowedStyles.has(property) || /url\s*\(|expression|@import/i.test(element.style.getPropertyValue(property))) element.style.removeProperty(property)
        }
    }
    return doc.body.innerHTML
}

export function readImage(file: File, maxLength = 2_000_000): Promise<string> {
    if (!file.type.startsWith("image/")) return Promise.reject(new Error("Поддерживаются PNG, JPEG, WebP, GIF и BMP"))
    return new Promise((resolve, reject) => {
        const url = URL.createObjectURL(file), image = new Image()
        const cleanup = () => URL.revokeObjectURL(url)
        image.onerror = () => { cleanup(); reject(new Error("Не удалось прочитать картинку")) }
        image.onload = () => {
            try {
                const scale = Math.min(1, 1600 / Math.max(image.naturalWidth, image.naturalHeight))
                const canvas = document.createElement("canvas")
                canvas.width = Math.max(1, Math.round(image.naturalWidth * scale)); canvas.height = Math.max(1, Math.round(image.naturalHeight * scale))
                const context = canvas.getContext("2d")
                if (!context) throw new Error("Не удалось подготовить изображение")
                context.drawImage(image, 0, 0, canvas.width, canvas.height)
                let result = canvas.toDataURL("image/webp", .88)
                for(let attempt=0;result.length>maxLength&&attempt<8;attempt++){
                    const smaller=document.createElement('canvas');smaller.width=Math.max(1,Math.round(canvas.width*.8));smaller.height=Math.max(1,Math.round(canvas.height*.8));smaller.getContext('2d')!.drawImage(canvas,0,0,smaller.width,smaller.height);canvas.width=smaller.width;canvas.height=smaller.height;context.drawImage(smaller,0,0);result=canvas.toDataURL('image/webp',.8)
                }
                if (result.length > maxLength) throw new Error("Не удалось уменьшить изображение для карты.")
                resolve(result)
            } catch (error) { reject(error) }
            finally { cleanup() }
        }
        image.src = url
    })
}

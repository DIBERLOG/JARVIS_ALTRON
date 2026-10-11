import { writable, get } from 'svelte/store'
import { invoke } from '@tauri-apps/api/core'

export type Mail = { id: string; subject: string; from?: { emailAddress?: { name?: string; address?: string } }; receivedDateTime: string; bodyPreview?: string; isRead?: boolean; body?: { contentType: string; content: string } }
export type Draft = { to: string; subject: string; body: string }
export type OutlookState = {
    connected: boolean; provider?: 'local' | 'graph'; account: { name?: string; email?: string }; busy: boolean; error: string; notice: string
    code: string; messages: Mail[]; selected: Mail | null; hasMore: boolean; draft: Draft; ticket: string
}
export const outlook = writable<OutlookState>({ connected: false, account: {}, busy: false, error: '', notice: '', code: '', messages: [], selected: null, hasMore: false, draft: { to: '', subject: '', body: '' }, ticket: '' })
export const mailUnreadOnly = writable(false)
function savedNativeMode(): boolean {
    try { return localStorage.getItem('jarvis-outlook-native-mode') === 'true' } catch { return false }
}
export const mailInOutlook = writable(savedNativeMode())
export function setMailInOutlook(enabled: boolean) {
    resetMailVoice()
    mailInOutlook.set(enabled)
    try { localStorage.setItem('jarvis-outlook-native-mode', String(enabled)) } catch { /* Storage may be unavailable. */ }
}
const update = (patch: Partial<OutlookState>) => outlook.update(state => ({ ...state, ...patch }))
export const outlookApi = <T = any>(action: string, data?: unknown): Promise<T> => invoke('outlook_request', { action, data: data ?? null })
export function editDraft(patch: Partial<Draft>) {
    outlook.update(state => ({ ...state, draft: { ...state.draft, ...patch }, ticket: '', notice: '' }))
}
export async function mailAction<T>(action: () => Promise<T>): Promise<T> {
    if (get(outlook).busy) throw new Error('Дождитесь завершения текущего запроса Outlook.')
    update({ busy: true, error: '', notice: '' })
    try { return await action() }
    catch (error) { update({ error: String(error) }); throw error }
    finally { update({ busy: false }) }
}
export async function refreshMail() {
    return mailAction(async () => {
        const result = await outlookApi<{ messages: Mail[]; hasMore: boolean }>('inbox')
        update({ messages: result.messages ?? [], hasMore: result.hasMore, notice: 'Входящие обновлены.' })
    })
}
export async function selectMail(id: string) {
    return mailAction(async () => { update({ selected: await outlookApi<Mail>('message', { id }) }) })
}
export async function prepareMail(forceNative = false) {
    return mailAction(async () => {
        const draft = { ...get(outlook).draft }, snapshot = JSON.stringify(draft)
        const result = await outlookApi<{ ticket: string }>('prepare_send', { ...draft, native: (forceNative === true || get(mailInOutlook)) && get(outlook).provider === 'local' })
        if (!result.ticket) throw new Error('Outlook не выдал подтверждение. Письмо не отправлено.')
        if (JSON.stringify(get(outlook).draft) !== snapshot) {
            await outlookApi('cancel_send'); throw new Error('Письмо изменилось. Проверьте его ещё раз.')
        }
        update({ ticket: result.ticket, notice: 'Проверьте получателя и весь текст. Подтверждение действует две минуты.' })
    })
}
export async function sendMail() {
    return mailAction(async () => {
        const ticket = get(outlook).ticket
        if (!ticket) throw new Error('Сначала проверьте письмо перед отправкой.')
        update({ ticket: '' })
        await outlookApi('send', { ticket })
        update({ draft: { to: '', subject: '', body: '' }, notice: get(outlook).provider === 'local' ? 'Outlook принял письмо к отправке. Проверьте доставку в тестовом ящике.' : 'Microsoft принял письмо к отправке. Это ещё не подтверждение доставки.' })
    })
}
export async function saveMailDraft() {
    return mailAction(async () => {
        await outlookApi('draft', get(outlook).draft)
        update({ notice: 'Черновик сохранён в Outlook. Письмо не отправлено.' })
    })
}
export async function openMailDraft() {
    if (!get(outlook).connected || get(outlook).provider !== 'local') {
        throw new Error('Для открытия окна письма подключите классический Outlook. Черновик сохранён в JARVIS.')
    }
    await mailAction(() => outlookApi('open_classic'))
    await prepareMail(true)
    await cancelMailSend()
    update({notice:'Черновик открыт в классическом Outlook. Письмо не отправлено; проверьте его в открытом окне.'})
}
export async function cancelMailSend() {
    update({ ticket: '', notice: 'Отправка отменена. Текст письма оставлен для редактирования.' })
    await outlookApi('cancel_send')
}
export function replyToMail() {
    const selected = get(outlook).selected
    if (!selected) throw new Error('Сначала откройте письмо.')
    // A new message to the sender, not a Graph-thread reply. Explicitly labelled in the UI.
    editDraft({ to: selected.from?.emailAddress?.address ?? '', subject: /^Re:/i.test(selected.subject) ? selected.subject : `Re: ${selected.subject}`, body: '' })
}
export async function disconnectMail() {
    await mailAction(async () => {
        await outlookApi('disconnect')
        update({ connected: false, account: {}, code: '', messages: [], selected: null, ticket: '', draft: { to: '', subject: '', body: '' }, notice: 'Аккаунт отключён. Данные писем убраны из памяти приложения.' })
    })
}
export function setMailStatus(result: { connected: boolean; provider?: 'local' | 'graph'; account?: OutlookState['account'] }) {
    update({ connected: result.connected, provider: result.provider, account: result.account ?? {}, ...(result.connected ? { code: '' } : {}) })
}
export function setMailCode(code: string) { update({ code }) }
export function mailError(error: unknown) { update({ error: String(error) }) }

type VoiceStep = 'recipient' | 'recipient_choice' | 'recipient_confirm' | 'subject' | 'body' | 'review' | 'number' | 'confirm' | null
let step: VoiceStep = null
type Recipient = { email: string; name: string }
let recipientChoices: Recipient[] = []
let recipientCandidate: Recipient | null = null
export function spokenMailAddress(raw: string): string | null {
    const aliases: Record<string, string> = { тест: 'test', джарвис: 'jarvis', джимейл: 'gmail', гмейл: 'gmail', яндекс: 'yandex', мэйл: 'mail', мейл: 'mail', аутлук: 'outlook', ру: 'ru', ком: 'com', орг: 'org', нет: 'net' }
    const address = raw.toLowerCase().replace(/ё/g, 'е').trim()
        .replace(/^(?:адрес|получатель|на почту)\s+/, '')
        .replace(/джи\s+мейл/g, 'gmail')
        .replace(/нижнее\s+подчеркивание/g, '_').replace(/дефис|тире/g, '-')
        .split(/\s+/).map(token => aliases[token] ?? token).join(' ')
        .replace(/(?:^|\s)at(?=\s|$)/g, '@').replace(/(?:^|\s)dot(?=\s|$)/g, '.')
        .replace(/\s*(?:собака|собачка|эт)\s*/g, '@')
        .replace(/\s*точка\s*/g, '.').replace(/\s+/g, '')
    return address.length <= 254 && /^[a-z0-9.!#$%&'*+/=?^_`{|}~-]+@[a-z0-9.-]+\.[a-z]{2,}$/i.test(address) ? address : null
}
export function formatMailText(raw: string): string {
    // Local presentation cleanup, not a rewrite. Keep paragraphs, URLs and addresses.
    let body = raw.replace(/\r\n/g, '\n').split('\n').map(line => line.replace(/[\t ]+/g, ' ').trim()).join('\n').trim()
    const first = body.match(/^\S+/)?.[0] ?? ''
    if (first && !/@|https?:\/\/|www\./i.test(first)) body = body.replace(/^\p{L}/u, letter => letter.toLocaleUpperCase('ru'))
    const last = body.match(/\S+$/)?.[0] ?? ''
    if (/\p{L}$/u.test(body) && !/@|https?:\/\/|www\./i.test(last)) body += '.'
    return body
}
type VoiceBridge = { navigate: (section: string) => void; reply: (text: string, id?: string, followUp?: boolean) => void; number: (text: string) => number | undefined }
export function resetMailVoice() {
    const hadTicket = !!get(outlook).ticket
    step = null; recipientChoices = []; recipientCandidate = null; update({ ticket: '' })
    if (hadTicket) void outlookApi('cancel_send').catch(mailError)
}
export async function handleMailVoice(raw: string, bridge: VoiceBridge): Promise<boolean> {
    try { return await mailVoiceTurn(raw, bridge) }
    catch (error) {
        // Keep the dialogue alive, but never automatically retry a failed send.
        if (step === 'confirm' && !get(outlook).ticket) step = 'review'
        bridge.reply(`${String(error)} ${step ? 'Диалог сохранён. Скажите «повтори» для подсказки или «отмена».' : ''}`, '', step !== null)
        return true
    }
}
async function mailVoiceTurn(raw: string, bridge: VoiceBridge): Promise<boolean> {
    raw = raw.replace(/^\s*(?:джарвис|jarvis)[,\s]+(?=(?:напиши|отправь|открой|исправь|повтори|сохрани|адрес|продолжи|продолжить|получателя|дальше|готово|предложи|покажи|подскажи|оформи|отформатируй|форматируй|без|оставь|да|подтверждаю|другой|верно|правильно)(?:\s|$))/i, '').trim()
    const text = raw.toLowerCase().replace(/ё/g, 'е').trim().replace(/[.!?,]+$/g, '')
    const mailIntent = /почт|письм|outlook|аутлук/.test(text)
    const continueDraft = /^(продолжи(?:ть)?(?: письмо)?|продолжить написание письма|адрес (?:указал|указан|ввел|введен)|(?:я )?(?:указал|ввел) адрес|получателя указал|дальше|готово)$/.test(text)
    if (!step && !mailIntent && !get(outlook).ticket && !(continueDraft && /адрес|получателя/.test(text))) return false
    if (!get(outlook).connected) {
        try { setMailStatus(await outlookApi('status')) }
        catch (error) { bridge.reply(String(error)); return true }
    }
    const openIntent = /^(открой|покажи|открыть|запусти) (почту|outlook|аутлук|классический outlook|классический аутлук)$/.test(text)
    if (openIntent && !get(outlook).connected) {
        try {
            await mailAction(() => outlookApi('open_classic'))
            bridge.reply('Классический Outlook открыт, сэр. Для управления письмами подключите аккаунт в разделе почты JARVIS.')
        } catch (error) { bridge.reply(String(error)) }
        return true
    }
    const native = get(mailInOutlook) && get(outlook).provider === 'local'
    if (!native) bridge.navigate('outlook')
    if (!get(outlook).connected) { step = null; bridge.reply('Сначала подключите аккаунт в разделе «Почта Outlook», сэр.'); return true }
    const say = (message: string, next: VoiceStep = null) => { step = next; bridge.reply(message, '', next !== null) }
    const suggestRecipients = async (name: string) => {
        if (get(outlook).provider !== 'local') { say('Подсказки адресатов доступны для классического Outlook. Продиктуйте полный адрес.', 'recipient'); return }
        let result = await mailAction(() => outlookApi<{ candidates?: Recipient[] }>('recipient_suggestions', { name }))
        if (!result.candidates?.length && name) result = await mailAction(() => outlookApi('resolve_recipient', { name }))
        recipientChoices = (result.candidates ?? []).slice(0, 5)
        recipientCandidate = null
        if (!recipientChoices.length) { say('Подходящих адресатов нет — повторите полный адрес или введите его в Outlook.', 'recipient'); return }
        if (recipientChoices.length === 1) {
            recipientCandidate = recipientChoices[0]
            say(`Найден ${recipientCandidate.name || 'получатель'}: ${recipientCandidate.email}. Это нужный адрес? Скажите «да, этот адрес» или «другой адрес».`, 'recipient_confirm')
        } else {
            const list = recipientChoices.map((entry, index) => `${index + 1}: ${entry.name || 'получатель'}, ${entry.email}`).join('; ')
            update({ notice: `Адресаты из отправленных: ${list}` })
            say(`Предлагаю адресатов из недавно отправленных: ${list}. Назовите номер или скажите «другой адрес».`, 'recipient_choice')
        }
    }
    if (['recipient', 'recipient_choice', 'recipient_confirm'].includes(step ?? '') && /^(предложи|покажи|подскажи).*(адресат|получател)/.test(text)) { await suggestRecipients(''); return true }
    if (step === 'recipient_choice' || step === 'recipient_confirm') {
        if (/^(повтори|повторить|не понял)$/.test(text)) {
            say(recipientCandidate ? `Адрес: ${recipientCandidate.email}. Скажите «да, этот адрес» или «другой адрес».` : recipientChoices.map((entry, index) => `${index + 1}: ${entry.name}, ${entry.email}`).join('; '), step); return true
        }
        if (/^(другой адрес|не тот|не этот|нет другой|назад)$/.test(text)) { recipientCandidate = null; say('Назовите другое имя или полный адрес, сэр.', 'recipient'); return true }
        if (step === 'recipient_confirm' && /^(да|да этот адрес|этот адрес|верно|правильно|подтверждаю адрес)$/.test(text.replace(/,/g, ''))) {
            if (!recipientCandidate) { say('Назовите получателя заново.', 'recipient'); return true }
            editDraft({ to: recipientCandidate.email }); recipientCandidate = null
            say('Адрес подтверждён. Какова тема письма, сэр?', 'subject'); return true
        }
        if (step === 'recipient_choice') {
            const index = bridge.number(text)
            if (index && recipientChoices[index - 1]) {
                recipientCandidate = recipientChoices[index - 1]
                say(`Получатель: ${recipientCandidate.name}, ${recipientCandidate.email}. Скажите «да, этот адрес» или «другой адрес».`, 'recipient_confirm'); return true
            }
        }
        if (!/^(отмена|отмени|нет|не отправляй|отменить отправку)$/.test(text) && !/^(исправь|измени|поменяй) /.test(text)) {
            say(step === 'recipient_choice' ? 'Назовите номер адресата или скажите «другой адрес».' : 'Подтвердите адрес: «да, этот адрес» или «другой адрес».', step); return true
        }
    }
    if (step && /^(повтори|повторить|что дальше|не понял|ошибся|я ошибся)$/.test(text)) {
        const hint = step === 'recipient' ? 'Повторите полный адрес: например, test собака jarvis точка test.' : step === 'subject' ? 'Продиктуйте тему письма.' : step === 'body' ? 'Продиктуйте текст письма.' : step === 'confirm' ? 'Проверьте письмо. Скажите «да, отправить» или «отмена».' : 'Скажите «отправь письмо», «исправь адрес», «исправь тему» или «исправь текст».'
        say(hint, step); return true
    }
    if (step && /^(исправь|измени|поменяй) (адрес|получателя|тему|текст)$/.test(text)) {
        update({ ticket: '' }); await outlookApi('cancel_send')
        const next = /адрес|получателя/.test(text) ? 'recipient' : /тему/.test(text) ? 'subject' : 'body'
        if (next === 'recipient') editDraft({ to: '' })
        say(next === 'recipient' ? 'Назовите адрес заново, сэр.' : next === 'subject' ? 'Продиктуйте новую тему.' : 'Продиктуйте новый текст.', next); return true
    }
    if (/^(отмена|отмени|нет|не отправляй|отменить отправку)$/.test(text)) {
        await cancelMailSend(); say('Отправка отменена, сэр. Текст письма сохранён на экране.'); return true
    }
    if ((step === 'review' || step === 'confirm') && /^(оформи|отформатируй|форматируй)(?: текст| письмо)?$/.test(text)) {
        const original = get(outlook).draft.body
        editDraft({ body: formatMailText(original) })
        await outlookApi('cancel_send'); step = 'review'
        if (get(outlook).provider === 'local') { await prepareMail(true); update({ ticket: '' }); await outlookApi('cancel_send') }
        say('Текст аккуратно оформлен без переписывания, сэр. Проверьте результат. Для отправки скажите «отправь письмо».', 'review'); return true
    }
    if (step === 'review' && /^(без форматирования|оставь как есть|не форматируй)$/.test(text)) {
        say('Оставляю текст как есть. Скажите «отправь письмо» или «исправь текст».', 'review'); return true
    }
    if (continueDraft || /продолж.*письм/.test(text)) {
        if ((!get(outlook).draft.to || native) && get(outlook).provider === 'local') {
            const result = await mailAction(() => outlookApi<{ draft: Draft }>('compose_read'))
            if (result.draft?.to) editDraft(result.draft)
        }
        const draft = get(outlook).draft
        if (!draft.to) { say('Укажите полный адрес в поле «Кому» и скажите «Адрес указал», сэр.', 'recipient'); return true }
        if (!draft.subject) say(`Получатель: ${draft.to}. Какова тема письма, сэр?`, 'subject')
        else if (!draft.body) say('Получатель и тема записаны. Продиктуйте текст письма, сэр.', 'body')
        else say('Получатель, тема и текст записаны. Скажите «отправь письмо» для проверки.', 'review')
        return true
    }
    if (step === 'subject') {
        if (!raw.trim()) { say('Не расслышал тему. Повторите, сэр.', 'subject'); return true }
        editDraft({ subject: raw.trim() }); say('Теперь продиктуйте текст письма, сэр.', 'body'); return true
    }
    if (step === 'body') {
        if (!raw.trim()) { say('Не расслышал текст. Повторите, сэр.', 'body'); return true }
        editDraft({ body: raw.trim() }); step = 'review'
        if (get(outlook).provider === 'local') {
            await prepareMail(true)
            // Showing the filled draft is not permission to send it.
            update({ ticket: '' }); await outlookApi('cancel_send')
        }
        say('Текст записан, сэр. Могу аккуратно оформить его — скажите «оформи текст» или «без форматирования». Для отправки скажите «отправь письмо». Продолжаю слушать.', 'review'); return true
    }
    if (openIntent) {
        try {
            if (get(outlook).provider === 'local') await mailAction(() => outlookApi('show_inbox'))
            else await mailAction(() => outlookApi('open_classic'))
        } catch (error) { say(String(error)); return true }
        say('Почта открыта, сэр. Можно обновить входящие или написать письмо.'); return true
    }
    // Distinct commands remain usable during a dictation/confirmation dialogue.
    if (/^(обнови|обновить|проверь|покажи|прочитай).*(почт|входящие|письма)$/.test(text) || /непрочитанн.*письм/.test(text)) {
        await refreshMail(); mailUnreadOnly.set(/непрочитан/.test(text));
        if (native) await mailAction(() => outlookApi('show_inbox'))
        say(native ? 'Входящие открыты в Outlook. Для выбора по номеру используйте список последних писем JARVIS; сортировка Outlook может отличаться.' : 'Входящие обновлены, сэр. Назовите номер письма из списка, чтобы открыть его.'); return true
    }
    if (/^(напиши|написать|создай|составь|подготовь|новое).*письм/.test(text)) {
        editDraft({ to: '', subject: '', body: '' }); step = 'recipient'
        if (get(outlook).provider === 'local') await mailAction(() => outlookApi('compose'))
        say('Кому написать, сэр? Назовите полный адрес или введите его в Outlook, нажмите Tab и скажите «Адрес указал».', 'recipient'); return true
    }
    if (/сохран.*черновик/.test(text)) { await saveMailDraft(); say('Черновик сохранён в Outlook, сэр. Письмо не отправлено.'); return true }
    if (/ответ.*письм/.test(text)) { replyToMail(); say('Готовлю новое письмо отправителю, сэр. Продиктуйте текст.', 'body'); return true }
    if (/отправ.*письм/.test(text) && step !== 'confirm') {
        await prepareMail(true); say(get(outlook).provider === 'local' ? 'Черновик открыт в Outlook, сэр. Проверьте адресатов и текст. Скажите «да, отправить» или «отмена». Изменения в окне Outlook заблокируют эту отправку.' : 'Проверьте получателя, тему и текст на экране, сэр. Скажите «да, отправить» или «отмена».', 'confirm'); return true
    }
    if (step === 'recipient') {
        const address = spokenMailAddress(text)
        if (!address) {
            if (get(outlook).provider === 'local' && !/@|собак|точка/.test(text)) { await suggestRecipients(text); return true }
            say('Не расслышал полный адрес, сэр. Продолжаю слушать: повторите его со словами «собака» и «точка». Можно ввести адрес в JARVIS и сказать «продолжить письмо».', 'recipient'); return true
        }
        editDraft({ to: address }); say(`Получатель: ${address}. Если неверно, скажите «исправь адрес». Какова тема письма, сэр?`, 'subject'); return true
    }
    if (step === 'confirm') {
        if (/^(да|да отправить|да отправь|подтверждаю|отправить|отправь письмо)$/.test(text.replace(/,/g, ''))) {
            step = 'review'; await sendMail(); say('Письмо принято к отправке, сэр.'); return true
        }
        say('Отправка ещё не подтверждена, сэр. Скажите «да, отправить» или «отмена».', 'confirm'); return true
    }
    if (/откр|прочита/.test(text) || step === 'number') {
        const index = bridge.number(text), messages = get(outlook).messages
        if (!index || !messages[index - 1]) { say('Назовите номер письма из текущего списка, сэр.', 'number'); return true }
        await selectMail(messages[index - 1].id)
        if (native) await mailAction(() => outlookApi('show_message', { id: messages[index - 1].id }))
        const mail = get(outlook).selected!
        // No untrusted mail instructions are executed, and the body is not passed to a model.
        say(`Письмо от ${mail.from?.emailAddress?.name || mail.from?.emailAddress?.address || 'неизвестного отправителя'}. Тема: ${mail.subject || 'без темы'}. Текст показан на экране.`)
        return true
    }
    if (step === 'review') { say('Письмо сохранено в диалоге, сэр. Скажите «отправь письмо», «исправь адрес», «исправь тему», «исправь текст» или «отмена».', 'review'); return true }
    say('Почта открыта, сэр. Можно обновить входящие, открыть письмо по номеру или написать новое письмо.')
    return true
}

<script lang="ts">
    import '@fontsource-variable/manrope/wght.css'
    import { onMount, onDestroy } from 'svelte'
    import { goto } from '@roxi/routify'
    import { invoke } from '@tauri-apps/api/core'
    import { save } from '@tauri-apps/plugin-dialog'
    import { writeTextFile, writeFile } from '@tauri-apps/plugin-fs'
    import { outlook, editDraft, openMailDraft } from '@/lib/outlook'
    import { centerSection } from '@/lib/centerVoice'
    import { agentScenarios, runAgentChain, type ChainStep } from '@/lib/agentChains'
    import { renderNote as renderFormattedNote } from '@/lib/noteFormatting'
    import { exportFormats, exportHtml, exportBlocks } from '@/lib/agentExport'
    const renderNote=(text:string,formatted:boolean)=>renderFormattedNote(text,formatted,true)

    const agents = [
        { id: 'mail', title: 'Почта', icon: '✉', description: 'Письма, ответы и деловая переписка', example: 'Подготовь вежливое письмо поставщику: попроси актуальный прайс и условия доставки.', hint: 'Готовит тему и текст. Получателей выбираете вы; отправка — через проверку в разделе Outlook.' },
        { id: 'documents', title: 'Документы', icon: '▤', description: 'Отчёты, записки и краткие резюме', example: 'Составь краткий отчёт о проделанной работе на основе контекста.', hint: 'Работает с вставленным текстом и выбранными документами. Исходные файлы не изменяются.' },
        { id: 'tasks', title: 'Задачи', icon: '✓', description: 'Поручения, приоритеты и план работы', example: 'Разбей поручение на задачи и отметь, какие сроки и данные нужно уточнить.', hint: 'Готовит план. Напоминания и события автоматически не создаются.' },
        { id: 'chains', title: 'Саб-агенты', icon: '◈', description: 'Команда помощников · 6 сценариев', example: '', hint: 'Три помощника последовательно анализируют, готовят и проверяют результат. Каждый этап — отдельный запрос к модели.' },
    ]
    let selected = 'mail', task = '', context = '', content = '', to = '', subject = '', body = ''
    let busy = false, exporting = false, error = '', notice = '', model = '', overwrite = false
    let resultKind = '', resultModel = ''
    let exportFormat='txt'
    let exportStatus='',exportError=''
    let scenarioId = 'document_mail', chainSteps:ChainStep[] = [], stopRequested = false, allowChain = false
    let resultScenario = '', resultReview = '', resultReport = '', approved = false, disposed = false
    onDestroy(() => { disposed=true; stopRequested=true })
    $: scenario = agentScenarios.find(item=>item.id===scenarioId) || agentScenarios[0]
    type Attachment = {name:string; text:string; truncated:boolean; warning:string; id:number}
    let attachments: Attachment[] = [], reading = false, acceptTruncated = false, nextAttachmentId = 0
    $: combinedContext = [context, ...attachments.map(file => `\nДокумент: ${file.name}\n${file.text}`)].filter(Boolean).join('\n\n')
    $: contextLength = Array.from(combinedContext).length
    $: hasTruncated = attachments.some(file => file.truncated)
    $: canRun = !busy && !reading && !!task.trim() && contextLength <= 12000 && (!hasTruncated || acceptTruncated) && (selected!=='chains' || (!!combinedContext.trim() && allowChain))
    $: agent = agents.find(item => item.id === selected) || agents[0]
    $: hasMailDraft = !!($outlook.draft.to || $outlook.draft.subject || $outlook.draft.body)
    $: canTransfer = !!to.trim() && !!subject.trim() && !!body.trim() && !$outlook.busy && (!hasMailDraft || overwrite) && (!resultScenario || approved)
    onMount(async () => {
        try {
            const config = await invoke<{provider:string; local_model:string; openai_model:string; deepseek_model:string}>('chat_get_config')
            model = `${config.provider} · ${config.provider === 'openai' ? config.openai_model : config.provider === 'deepseek' ? config.deepseek_model : config.local_model}`
        } catch { model = 'Модель из настроек' }
    })
    function choose(id: string) {
        selected = id; task = ''; context = ''; attachments = []; acceptTruncated = false; error = ''; notice = ''
        allowChain=false; chainSteps=[]
        // Keep the prepared result visible until explicitly replaced or cleared.
    }
    async function run() {
        if (!canRun) return
        busy = true; error = ''; notice = ''
        stopRequested=false; approved=false
        try {
            if (selected==='chains') {
                const currentScenario=scenario
                chainSteps=currentScenario.steps.map(title=>({title,status:'waiting'}))
                const result=await runAgentChain({scenario:currentScenario.id,task,context:combinedContext},()=>stopRequested || disposed,(index,step)=>{
                    if (!disposed) chainSteps=chainSteps.map((old,i)=>i===index?step:old)
                })
                if (disposed) return
                if (!result) { notice='Цепочка остановлена. Следующие этапы не запущены; предыдущий итог не заменён.'; return }
                content=result.content; subject=result.subject || ''; body=result.body || ''
                resultKind=currentScenario.mail?'mail':'documents'; resultModel=`${result.provider} · ${result.model}`
                resultScenario=currentScenario.title; resultReview=result.review || ''; resultReport=result.report || ''
                overwrite=false; notice='Цепочка завершена. Просмотрите этапы, проверьте факты и итог перед использованием.'
                return
            }
            const result = await invoke<{content:string; subject:string|null; body:string|null; provider:string; model:string}>('agent_run', {kind:selected, task, context:combinedContext})
            if (disposed) return
            if (!result.content?.trim()) throw new Error('Модель вернула пустой результат.')
            if (selected === 'mail' && (!result.subject?.trim() || !result.body?.trim())) throw new Error('Модель не подготовила корректное письмо.')
            content = result.content; subject = result.subject || ''; body = result.body || ''
            resultKind = selected; resultModel = `${result.provider} · ${result.model}`
            resultScenario=''; resultReview=''; resultReport=''
            overwrite = false; notice = 'Готово. Проверьте факты и отредактируйте результат.'
        } catch (e) { if (!disposed) error = `${String(e)}${selected==='chains'?' Цепочка остановлена; предыдущий итог оставлен.':''}` }
        finally { busy = false }
    }
    async function attachFiles(event: Event) {
        const input = event.currentTarget as HTMLInputElement
        const files = Array.from(input.files || [])
        input.value = ''
        if (!files.length || reading || busy) return
        if (attachments.length + files.length > 5) { error = 'Можно прикрепить до пяти файлов. Удалите лишние.'; return }
        reading = true; error = ''; notice = ''; acceptTruncated = false
        const errors: string[] = []
        try {
            for (const file of files) {
                try {
                    if (!/\.(docx|txt|md|csv|json|log)$/i.test(file.name)) throw Error('Этот формат пока не поддерживается. Используйте DOCX или текстовый файл.')
                    if (!file.size || file.size > 5 * 1024 * 1024) throw Error('Размер файла должен быть от 1 байта до 5 МБ.')
                    const result = await invoke<Omit<Attachment,'id'>>('agent_read_attachment', {name:file.name, data:Array.from(new Uint8Array(await file.arrayBuffer()))})
                    if (!result.text?.trim()) throw Error('В файле не найден текст.')
                    attachments = [...attachments, {...result, id:++nextAttachmentId}]
                } catch(e) { errors.push(`${file.name}: ${String(e)}`) }
            }
            error = errors.join('\n')
            if (!errors.length) notice = 'Документы прочитаны локально. Проверьте извлечённый текст перед запуском агента.'
        } finally { reading = false }
    }
    async function transfer() {
        if (!canTransfer) return
        editDraft({to:to.trim(), subject, body})
        busy=true; error=''
        try { await openMailDraft() }
        catch(e) { error=String(e); return }
        finally { busy=false }
        centerSection.set('outlook'); $goto('/center')
    }
    function stepLabel(step: ChainStep) {
        if (step.status==='running' && stopRequested) return 'Остановка запрошена'
        if (step.status==='waiting' && !busy && chainSteps.some(s=>s.status==='error' || s.status==='stopped')) return 'Не запущен'
        return {waiting:'Ожидает',running:'Работает',done:'Готово',error:'Ошибка',stopped:'Остановлен'}[step.status]
    }
    async function exportSteps() {
        exporting=true; error=''
        try {
            const path=await save({defaultPath:'jarvis-subagents-steps.txt',filters:[{name:'Текст',extensions:['txt']}]})
            if(path) await writeTextFile(path,chainSteps.map((step,i)=>`${i+1}. ${step.title} [${step.status}]\n${step.result?.content || step.error || ''}`).join('\n\n'))
        } catch(e) {error=`Не удалось экспортировать этапы: ${String(e)}`}
        finally {exporting=false}
    }
    async function exportResult() {
        exporting = true; error = ''; notice = ''
        exportStatus='';exportError=''
        try {
            const format=exportFormat
            const text=resultKind === 'mail' ? `Кому: ${to}\nТема: ${subject}\n\n${body}${resultReport?`\n\nНедельный отчёт:\n${resultReport}`:''}${resultReview?`\n\nЗамечания проверяющего:\n${resultReview}`:''}` : content
            const path = await save({defaultPath:`jarvis-agent-${resultKind}.${format}`, filters:[{name:exportFormats.find(f=>f.id===format)?.label || 'Текст',extensions:[format]}]})
            if (!path) return
            if(!path.toLowerCase().endsWith(`.${format}`)) throw Error(`Имя файла должно заканчиваться на .${format}. Выберите соответствующий формат.`)
            if(format==='docx') {
                const bytes=await invoke<number[]>('agent_export_docx',{blocks:exportBlocks(text)})
                await writeFile(path,new Uint8Array(bytes))
            } else await writeTextFile(path,format==='html'?exportHtml(text):text)
            notice = 'Результат экспортирован.'
            exportStatus=`Файл сохранён: ${path}`
        } catch(e) { error = `Не удалось экспортировать: ${String(e)}`;exportError=error }
        finally { exporting = false }
    }
</script>

<section class="agents-workspace">
    <header><div><span class="eyebrow">ОФИСНАЯ ЛАБОРАТОРИЯ</span><h1>Агенты <span class="badge">Тестовая функция</span></h1><p>Поручение → подготовка → ваша проверка. Без автономной отправки и скрытых действий.</p></div><button on:click={() => $goto('/settings')}>Настроить модель ↗</button></header>
    <div class="privacy"><strong>Перед запуском:</strong> агент получит поручение, вставленный контекст и извлечённый текст выбранных документов. При выборе OpenAI или DeepSeek этот текст передаётся внешнему сервису. Не передавайте пароли и конфиденциальные данные без разрешения. Прикрепление здесь — исходные данные для модели, не вложение в письмо.</div>
    <div class="workspace">
        <nav aria-label="Подразделы агентов">
            {#each agents as item}<button class:active={selected === item.id} aria-pressed={selected === item.id} disabled={busy || reading} on:click={() => choose(item.id)}><span class="agent-icon" aria-hidden="true">{item.icon}</span><span><strong>{item.title}</strong><small>{item.description}</small></span></button>{/each}
            <div class="model"><span class="eyebrow">МОДЕЛЬ ИЗ НАСТРОЕК</span><p>{model || 'Загрузка…'}</p><small>{selected==='chains'?'Саб-агенты: до трёх последовательных запросов к одной выбранной модели.':'Тестовый режим: один запрос.'} Без фоновых запусков и доступа ко всей почте.</small></div>
        </nav>
        <div class="panels">
            <form class="panel" on:submit|preventDefault={run}>
                <div class="panel-head"><span class="step">01</span><div><h2>Агент · {agent.title}</h2><p>{agent.hint}</p></div></div>
                {#if selected==='chains'}<label for="agent-scenario">Сценарий саб-агентов</label><select id="agent-scenario" bind:value={scenarioId} disabled={busy || reading} on:change={() => { task=''; allowChain=false; chainSteps=[]; notice=''; error='' }}>{#each agentScenarios as item}<option value={item.id}>{item.title}</option>{/each}</select><p class="hint">Добавьте исходное письмо, протокол, заметки или предложения поставщиков в контекст. Проверка модели — предварительная, не гарантия точности.</p>{/if}
                <label for="agent-task">Что нужно сделать?</label><textarea id="agent-task" rows="3" bind:value={task} maxlength="1000" required disabled={busy} placeholder={selected==='chains'?scenario.example:agent.example}></textarea>
                <button type="button" class="example" disabled={busy} on:click={() => task = selected==='chains'?scenario.example:agent.example}>Вставить пример поручения</button>
                <label for="agent-context">Контекст и исходные данные <span>необязательно</span></label><textarea id="agent-context" rows="5" bind:value={context} maxlength="12000" disabled={busy || reading} placeholder="Вставьте текст или прикрепите документы ниже."></textarea>
                <div class="file-picker"><label for="agent-files">Прикрепить документы</label><input id="agent-files" type="file" multiple accept=".docx,.txt,.md,.csv,.json,.log" disabled={busy || reading || attachments.length >= 5} on:change={attachFiles}/><small>{reading ? 'Читаю документы…' : 'DOCX, TXT, MD, CSV, JSON, LOG · до 5 файлов по 5 МБ. PDF, сканы и XLSX пока не поддерживаются.'}</small></div>
                {#each attachments as file (file.id)}<article class="attachment"><div class="attachment-head"><strong>{file.name}</strong><button type="button" aria-label={`Удалить ${file.name}`} disabled={busy || reading} on:click={() => {attachments=attachments.filter(a => a.id !== file.id); acceptTruncated=false}}>Удалить</button></div>{#if file.warning}<p class="hint">{file.warning}</p>{/if}{#if file.truncated}<p class="cut-warning">Документ сокращён: извлечены первые 8000 символов. Это не весь документ.</p>{/if}<details><summary>Просмотреть и сократить текст</summary><label for={`attachment-${file.id}`}>Текст документа {file.name}</label><textarea id={`attachment-${file.id}`} rows="6" bind:value={file.text} maxlength="8000" disabled={busy || reading}></textarea></details></article>{/each}
                <p class:cut-warning={contextLength > 12000} class="hint">Общий контекст: {contextLength.toLocaleString('ru-RU')} / 12 000 символов. {contextLength > 12000 ? 'Сократите текст или удалите часть документов перед запуском.' : ''}</p>
                {#if hasTruncated}<label class="overwrite"><input type="checkbox" bind:checked={acceptTruncated} disabled={busy || reading}/> Использовать только извлечённые фрагменты сокращённых документов</label>{/if}
                {#if selected==='chains'}<label class="overwrite"><input type="checkbox" bind:checked={allowChain} disabled={busy || reading}/> Разрешаю до трёх запросов с этими данными; в облаке возможны расходы</label>{/if}
                <div class="actions"><button class="primary" disabled={!canRun}>{busy ? 'Агент готовит результат…' : 'Подготовить результат →'}</button><small>Не отправляет письма и не выполняет команды.</small></div>
                {#if selected==='chains'}<div class="actions"><button type="button" class="stop" disabled={!busy || stopRequested} on:click={() => {stopRequested=true; notice='Остановка запрошена. Ожидаем завершения уже отправленного запроса; новые этапы не запустятся.'}}>{busy && stopRequested?'Останавливаем цепочку…':'Остановить цепочку'}</button><small>{!busy ? (chainSteps.length ? 'Работа завершена или остановлена. Можно запустить заново.' : 'Кнопка станет доступна во время работы.') : ''}</small></div><small class="hint">Уже отправленный запрос может завершиться и тарифицироваться. Его ответ после остановки не станет итогом.</small>{/if}
            </form>
            {#if error}<p class="error" role="alert">{error}</p>{/if}
            {#if notice}<p class="notice" role="status">{notice}</p>{/if}
            {#if chainSteps.length}
                <section class="panel chain-progress" aria-label="Этапы саб-агентов">
                    <h2>Ход работы команды</h2>
                    <ol>{#each chainSteps as step,index}
                        <li class:running={step.status==='running'} class:failed={step.status==='error'}>
                            <div><span class="step">{index+1}</span><strong>{step.title}</strong><span class="step-status">{stepLabel(step)}</span></div>
                            {#if step.result}<details><summary>Результат этапа {index+1}</summary><div class="formatted stage-text">{@html renderNote(step.result.subject?`Тема: ${step.result.subject}\n\n${step.result.body}${step.result.report?`\n\nОтчёт:\n${step.result.report}`:''}${step.result.review?`\n\nПроверка:\n${step.result.review}`:''}`:step.result.content,true)}</div></details>{/if}
                            {#if step.error}<p class="error">{step.error}</p>{/if}
                        </li>
                    {/each}</ol>
                    <button disabled={busy || exporting || !chainSteps.some(s=>s.result)} on:click={exportSteps}>Экспортировать этапы .txt</button>
                    <p class="hint">На ошибке дальнейшие этапы не запускаются. Промежуточные результаты не являются готовым письмом.</p>
                </section>
            {/if}
            <section class="panel" aria-label="Результат агента" aria-busy={busy}>
                <div class="panel-head"><span class="step">02</span><div><h2>Проверьте результат</h2><p>{resultKind ? `${resultScenario || agents.find(a => a.id === resultKind)?.title} · ${resultModel}` : 'Здесь появится редактируемый черновик, документ или план.'}</p></div></div>
                {#if resultReview}<div class="review"><h3>Замечания проверяющего</h3><div class="formatted">{@html renderNote(resultReview,true)}</div><small>Это вывод модели, а не гарантия отсутствия ошибок. Уточните недостающие сведения до отправки.</small></div>{/if}
                {#if resultReport}<label for="agent-report">Недельный отчёт</label><textarea id="agent-report" rows="8" bind:value={resultReport} disabled={busy} on:input={() => approved=false}></textarea><p class="hint">Отчёт экспортируется отдельно в общем текстовом результате. Вложением в письмо он автоматически не становится.</p>{/if}
                {#if resultKind === 'mail'}
                    <label for="agent-to">Получатели</label><input id="agent-to" bind:value={to} maxlength="2000" disabled={busy} placeholder="test@jarvis.test; коллега@компания.ru" on:input={() => approved=false}/>
                    <label for="agent-subject">Тема письма</label><input id="agent-subject" bind:value={subject} maxlength="250" disabled={busy} on:input={() => approved=false}/>
                    <label for="agent-body">Текст письма</label><textarea id="agent-body" rows="9" bind:value={body} maxlength="100000" disabled={busy} on:input={() => approved=false}></textarea>
                    <details class="preview" open><summary>Форматированный просмотр письма</summary><div class="formatted">{@html renderNote(body,true)}</div></details>
                    {#if resultReport}<details class="preview" open><summary>Форматированный просмотр отчёта</summary><div class="formatted">{@html renderNote(resultReport,true)}</div></details>{/if}
                    {#if resultScenario}<label class="overwrite"><input type="checkbox" bind:checked={approved} disabled={busy}/> Я проверил итог, замечания и адресатов перед передачей в Outlook</label>{/if}
                    {#if hasMailDraft}<label class="overwrite"><input type="checkbox" bind:checked={overwrite} disabled={busy}/> Заменить существующий черновик JARVIS этим письмом</label>{/if}
                    <button class="primary" disabled={busy || !canTransfer} on:click={transfer}>Передать черновик в Outlook →</button><p class="hint">Откроется раздел почты JARVIS. Подключите Outlook, проверьте адресатов и подтвердите отправку там. Сейчас письмо не отправляется.</p>
                {:else if resultKind}
                    <label for="agent-result">Редактируемый текст</label><textarea id="agent-result" rows="12" bind:value={content} disabled={busy}></textarea>
                    <details class="preview" open><summary>Форматированный просмотр результата</summary><div class="formatted">{@html renderNote(content,true)}</div></details>
                {:else}<div class="empty"><span aria-hidden="true">✧</span><p>Дайте агенту небольшое поручение.<br/>Начните с примера выше.</p></div>{/if}
                {#if resultKind}<label for="agent-export-format">Формат экспорта</label><select id="agent-export-format" bind:value={exportFormat} disabled={busy || exporting}>{#each exportFormats as format}<option value={format.id}>{format.label}</option>{/each}</select><div class="actions"><button disabled={busy || exporting} on:click={exportResult}>{exporting ? 'Экспорт…' : `Экспортировать .${exportFormat}`}</button><button disabled={busy || exporting} on:click={() => { resultKind=''; content=''; subject=''; body=''; notice=''; resultScenario=''; resultReview=''; resultReport=''; approved=false }}>Очистить результат</button></div><small class="hint">Word и HTML сохраняют таблицы и оформление. TXT и Markdown — текстовые форматы. Экспортируйте результат до ухода со страницы или закрытия приложения.</small>{/if}
                {#if exportError}<p class="error" role="alert">{exportError}</p>{/if}
                {#if exportStatus}<p class="notice" role="status">{exportStatus}</p>{/if}
            </section>
        </div>
    </div>
</section>

<style>
    .formatted{overflow-x:auto}.formatted :global(table){border-collapse:collapse;width:100%;min-width:480px;margin:16px 0;font-size:12px}.formatted :global(th),.formatted :global(td){border:1px solid var(--line);padding:10px 12px;text-align:left;vertical-align:top;min-width:70px}.formatted :global(th){background:#17383d;color:#eaf6f8;font-weight:800}.formatted :global(tbody tr:nth-child(even)){background:#102c32}
    .formatted{font-size:13px;line-height:1.8;color:#c5dde4;overflow-wrap:anywhere;white-space:normal}.formatted :global(p){color:inherit;margin:9px 0}.formatted :global(h2),.formatted :global(h3),.formatted :global(h4){color:#eaf6f8;line-height:1.4;margin:20px 0 10px;font-size:16px}.formatted :global(strong){color:#eef9fa;font-weight:800}.formatted :global(em){font-style:italic}.stage-text{max-height:420px;overflow:auto;padding-right:8px}.review .formatted{color:#e0d6bd;margin:12px 0}.review .formatted :global(strong){color:#ffe1a7}.preview{margin-top:16px;padding:14px 16px;border:1px solid var(--line);border-radius:12px;background:#0b2026}.preview summary{cursor:pointer;font-size:12px;color:var(--accent);font-weight:700}
    select{width:100%;background:#091b20;color:#eef9fa;border:1px solid var(--line);border-radius:10px;padding:12px;font:inherit;font-size:13px}.chain-progress ol{list-style:none;padding:0;margin:18px 0}.chain-progress li{border:1px solid var(--line);border-radius:12px;padding:14px;margin:10px 0;background:#0b2026}.chain-progress li>div{display:flex;align-items:center;gap:12px}.chain-progress strong{font-size:12px}.step-status{margin-left:auto;font-size:11px;color:#9fbcc5}.chain-progress li.running{border-color:var(--accent)}.chain-progress li.failed{border-color:#bc6978}.chain-progress summary{color:var(--accent);font-size:12px;margin-top:12px;cursor:pointer}.chain-progress pre{white-space:pre-wrap;overflow-wrap:anywhere;max-height:320px;overflow:auto;font:inherit;font-size:12px;line-height:1.7;color:#c5dde4}.stop{color:#ffd19e;border-color:#8f633b;background:#392e24}.review{border:1px solid #7b6840;background:#302c23;border-radius:12px;padding:16px}.review h3{font-size:14px;color:#f8d391;margin:0}.review p{white-space:pre-wrap;color:#e0d6bd;font-size:13px}.review small{font-size:11px;color:#c9b98f}
    .file-picker{padding:12px 14px;border:1px dashed #39727a;border-radius:10px;margin-top:12px;background:#0b2026}.file-picker label{margin-top:0;color:var(--accent)}.file-picker input{font-size:12px;max-width:100%}.file-picker small{display:block;color:#9dbcc5;font-size:11px;line-height:1.7;margin-top:9px}.file-picker input::file-selector-button{background:#1b484e;color:#d6f9f5;border:1px solid #39727a;border-radius:7px;padding:8px 12px;margin-right:10px;cursor:pointer}.attachment{padding:14px;background:#0b2026;border:1px solid var(--line);border-radius:10px;margin-top:12px}.attachment-head{display:flex;justify-content:space-between;gap:12px;align-items:center}.attachment strong{font-size:12px;overflow-wrap:anywhere}.attachment summary{font-size:12px;color:var(--accent);cursor:pointer;margin-top:12px}.cut-warning{color:#f7d18d!important;font-size:12px;line-height:1.7}.error{white-space:pre-wrap}
    .agents-workspace{--accent:#45dfdc;--line:#294c52;--panel:#10262b;max-width:1180px;margin:24px auto;padding:0 24px 36px;color:#eaf6f8;font-family:'Manrope Variable',sans-serif}header{display:flex;justify-content:space-between;align-items:center;gap:20px;margin-bottom:20px}h1{font-size:30px;margin:8px 0}h2{font-size:18px;margin:0}p{line-height:1.65;margin:8px 0;color:#9fbcc5}.eyebrow{font-size:10px;letter-spacing:2px;font-weight:800;color:var(--accent)}.badge{font-size:11px;vertical-align:middle;margin-left:8px;background:#453923;color:#ffd991;padding:6px 10px;border-radius:20px}.privacy{border:1px solid #615231;background:#272a24;color:#dacba8;border-radius:12px;padding:14px 18px;font-size:12px;line-height:1.7;margin-bottom:22px}.workspace{display:grid;grid-template-columns:230px minmax(0,1fr);gap:22px}nav{display:flex;flex-direction:column;gap:10px}nav button{display:flex;gap:14px;text-align:left;padding:18px 14px}nav button.active{border-color:var(--accent);background:#163d41}nav strong{display:block;font-size:15px}nav small{display:block;line-height:1.6;margin-top:5px;color:#9fbcc5}.agent-icon{font-size:23px;color:var(--accent)}.model{padding:20px 12px;font-size:12px;overflow-wrap:anywhere}.model small{color:#86a7b2;line-height:1.8}.panels{display:grid;gap:18px}.panel{background:var(--panel);border:1px solid var(--line);border-radius:18px;padding:24px;min-width:0}.panel-head{display:flex;align-items:flex-start;gap:14px;margin-bottom:20px}.panel-head p{font-size:12px;margin:6px 0 0}.step{color:var(--accent);font-size:13px;border:1px solid var(--line);padding:9px;border-radius:10px}label{display:block;font-size:12px;margin:16px 0 8px;font-weight:700}label span{font-weight:400;color:#8baab5}input:not([type=checkbox]),textarea{box-sizing:border-box;width:100%;background:#091b20;color:#eef9fa;border:1px solid var(--line);border-radius:10px;padding:12px 14px;font:inherit;font-size:13px;line-height:1.65}textarea{resize:vertical}button{font:inherit;font-size:12px;color:#d2e6eb;background:#163238;border:1px solid var(--line);border-radius:10px;padding:11px 15px;cursor:pointer}button:disabled{opacity:.5;cursor:not-allowed}.primary{background:var(--accent);color:#082227;font-weight:800;border-color:var(--accent)}.example{margin-top:9px;color:var(--accent);background:transparent;padding:7px 10px}.actions{display:flex;align-items:center;flex-wrap:wrap;gap:12px;margin-top:18px}.actions small,.hint{font-size:11px;color:#9dbcc5;line-height:1.7}.overwrite{display:flex;align-items:center;gap:9px}.error,.notice{padding:12px 16px;border-radius:10px;margin:0}.error{background:#402b30;color:#ffc1c8}.notice{background:#173b39;color:#a1ece4}.empty{text-align:center;padding:30px}.empty span{font-size:36px;color:var(--accent)}:is(button,input,textarea):focus-visible{outline:2px solid var(--accent);outline-offset:3px}@media(max-width:800px){.workspace{grid-template-columns:1fr}nav{display:grid;grid-template-columns:repeat(3,1fr)}nav button{padding:12px;flex-direction:column}.model{grid-column:1/-1;padding:10px}.agents-workspace{padding:0 14px 24px}header{align-items:flex-start;flex-direction:column}h1{font-size:25px}.panel{padding:18px}}
</style>

<script lang="ts">
    import CenterTasks from "@/components/CenterTasks.svelte"
    import "@fontsource-variable/manrope/wght.css"
    import { onMount, onDestroy } from "svelte"
    import WeekWeather from "@/components/WeekWeather.svelte"
    import CenterTimer from "@/components/CenterTimer.svelte"
    import CenterNotes from "@/components/CenterNotes.svelte"
    import CenterVoiceText from "@/components/CenterVoiceText.svelte"
    import CenterHabits from "@/components/CenterHabits.svelte"
    import CenterTraining from "@/components/CenterTraining.svelte"
    import CenterNews from "@/components/CenterNews.svelte"
    import CenterRequestHistory from '@/components/CenterRequestHistory.svelte'
    import CenterPasswords from '@/components/CenterPasswords.svelte'
    import CenterOutlook from '@/components/CenterOutlook.svelte'
    import ReminderSchedule from '@/components/ReminderSchedule.svelte'
    import { sendAction } from '@/lib/ipc'
    import type { TrainingData } from "@/lib/training"
    import { centerSection, centerRevision, centerVoiceHint } from '@/lib/centerVoice'
    import { birthdayDate, birthdayOccursInYear, dayKey, emptyCenterData, loadCenterData, makeId, nextBirthday, saveCenterData, type CenterData, type Reminder, type Note } from "@/lib/center"

    const weekdays = ["Пн", "Вт", "Ср", "Чт", "Пт", "Сб", "Вс"]
    const today = new Date()
    let month = new Date(today.getFullYear(), today.getMonth(), 1)
    let selectedDay = dayKey(today)
    let data: CenterData = emptyCenterData()
    let loading = true, saving = false, error = "", notice = ""
    let noticeTimer: ReturnType<typeof setTimeout> | undefined
    let disposed = false
    onDestroy(() => { disposed = true; clearTimeout(noticeTimer) })
    let reminderTitle = "", reminderDue = defaultDue(selectedDay)
    let reminderAdvance: number[] = [], editingReminder: string | null = null
    function editReminder(item: Reminder) {
        editingReminder=item.id;reminderTitle=item.title
        const due=new Date(item.dueAt)
        reminderDue=`${dayKey(due)}T${String(due.getHours()).padStart(2,'0')}:${String(due.getMinutes()).padStart(2,'0')}`
        reminderAdvance=[...(item.advanceDays || [])]
    }
    let birthdayName = "", birthdayInput = ""
    const sections = [
        { id: "news", label: "Новости", icon: "◈", ready: true },
        { id: "outlook", label: "Почта Outlook", icon: "✉", ready: true },
        { id: "calendar", label: "Календарь", icon: "▦", ready: true },
        { id: "tasks", label: "Задачи", icon: "✓", ready: true },
        { id: "notes", label: "Заметки", icon: "✎", ready: true },
        { id: "voice-text", label: "Голос в текст", icon: "🎙", ready: true },
        { id: "timer", label: "Таймер", icon: "◷", ready: true },
        { id: "reminders", label: "Напоминания", icon: "◉", ready: true },
        { id: "birthdays", label: "Дни рождения", icon: "◆", ready: true },
        { id: "weather", label: "Погода на неделю", icon: "☁", ready: true },
        { id: "habits", label: "Привычки", icon: "◎", ready: true },
        { id: "workouts", label: "Тренировки", icon: "⚡", ready: true },
        { id: "passwords", label: "Пароли", icon: "◇", ready: true },
        { id: "request-history", label: "История запросов", icon: "↺", ready: true },
    ] as const
    let selectedSection: typeof sections[number]["id"] = "calendar"
    $: if (sections.some(section => section.id === $centerSection)) selectedSection = $centerSection as typeof selectedSection
    const unsubscribeRevision = centerRevision.subscribe(async revision => { if(revision) { try {data = await loadCenterData()} catch(e) {error=String(e)} } })
    onDestroy(unsubscribeRevision)

    $: monthLabel = new Intl.DateTimeFormat("ru-RU", { month: "long", year: "numeric" }).format(month)
    $: calendarDays = buildDays(month)
    $: reminders = [...data.reminders].sort((a, b) => a.dueAt.localeCompare(b.dueAt))
    $: birthdays = [...data.birthdays].sort((a, b) => nextBirthday(a).getTime() - nextBirthday(b).getTime())
    $: currentSection = sections.find(section => section.id === selectedSection) ?? sections[0]
    $: selectedReminders = reminders.filter(item => dayKey(new Date(item.dueAt)) === selectedDay)
    $: selectedBirthdays = data.birthdays.filter(item => birthdayOccursInYear(item, Number(selectedDay.slice(0, 4))) && dayKey(birthdayDate(item, Number(selectedDay.slice(0, 4)))) === selectedDay)
    $: pendingCount = data.reminders.filter(item => !item.done).length

    onMount(async () => {
        try { data = await loadCenterData() }
        catch (e) { error = `Не удалось открыть Центр: ${String(e)}` }
        finally { loading = false }
    })

    function buildDays(current: Date): (number | null)[] {
        const offset = (current.getDay() + 6) % 7
        const total = new Date(current.getFullYear(), current.getMonth() + 1, 0).getDate()
        return [...Array(offset).fill(null), ...Array.from({ length: total }, (_, i) => i + 1)]
    }
    function defaultDue(day: string) {
        if (day === dayKey(new Date())) {
            const nextHour = new Date(Date.now() + 60 * 60 * 1000)
            return `${dayKey(nextHour)}T${String(nextHour.getHours()).padStart(2, "0")}:00`
        }
        return `${day}T09:00`
    }
    function selectDay(day: number) {
        selectedDay = dayKey(new Date(month.getFullYear(), month.getMonth(), day))
        reminderDue = defaultDue(selectedDay)
    }
    function shiftMonth(delta: number) { month = new Date(month.getFullYear(), month.getMonth() + delta, 1) }
    function cellKey(day: number) { return dayKey(new Date(month.getFullYear(), month.getMonth(), day)) }
    function hasReminder(day: number) { return data.reminders.some(item => !item.done && dayKey(new Date(item.dueAt)) === cellKey(day)) }
    function hasBirthday(day: number) { return data.birthdays.some(item => birthdayOccursInYear(item, month.getFullYear()) && dayKey(birthdayDate(item, month.getFullYear())) === cellKey(day)) }
    function formatDue(value: string) { return new Date(value).toLocaleString("ru-RU", { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" }) }
    function birthdayCaption(item: CenterData["birthdays"][number]) {
        const date = nextBirthday(item)
        const when = date.toLocaleDateString("ru-RU", { day: "numeric", month: "long", year: "numeric" })
        const age = item.year ? date.getFullYear() - item.year : null
        return age === 0 ? `${when} · первая дата` : age !== null ? `${when} · исполнится ${age}` : when
    }
    async function persist(next: CenterData, message: string) {
        clearTimeout(noticeTimer)
        saving = true; error = ""; notice = ""
        try {
            await saveCenterData(next,data); data = next; notice = message
            if (!disposed && message.startsWith("Заметка")) noticeTimer = setTimeout(() => { notice = "" }, 2000)
        }
        catch (e) { error = String(e) }
        finally { saving = false }
        return !error
    }
    async function addReminder() {
        const title = reminderTitle.trim()
        if (!title || !reminderDue || saving) return
        if (Number.isNaN(new Date(reminderDue).getTime())) { error = "Укажите корректную дату напоминания"; return }
        const item:Reminder={id:editingReminder||makeId(),title,dueAt:new Date(reminderDue).toISOString(),done:false,advanceDays:reminderAdvance,announced:[],createdAt:new Date().toISOString(),silentPast:new Date(reminderDue).getTime()<=Date.now()}
        const edited=!!editingReminder
        if(edited&&!data.reminders.some(existing=>existing.id===editingReminder)){error='Напоминание уже удалено';return}
        await persist({ ...data, reminders: edited?data.reminders.map(existing=>existing.id===editingReminder?item:existing):[...data.reminders,item] }, edited?'Напоминание изменено':'Напоминание сохранено')
        if (!error) {reminderTitle = '';editingReminder=null;sendAction('center_reply',{text:edited?'Напоминание изменено, сэр.':'Напоминание сохранено, сэр.',reply_id:edited?'':'reminder_saved',follow_up:false})}
    }
    async function toggleReminder(item: Reminder) {
        await persist({ ...data, reminders: data.reminders.map(reminder => reminder.id === item.id ? { ...reminder, done: !reminder.done } : reminder) }, item.done ? "Напоминание возвращено" : "Готово")
    }
    async function deleteReminder(id: string) { await persist({ ...data, reminders: data.reminders.filter(item => item.id !== id) }, "Напоминание удалено") }
    async function addBirthday() {
        const name = birthdayName.trim()
        if (!name || !birthdayInput || saving) return
        const date = new Date(`${birthdayInput}T12:00`)
        if (Number.isNaN(date.getTime()) || dayKey(date) !== birthdayInput) { error = "Укажите корректную дату рождения"; return }
        await persist({ ...data, birthdays: [...data.birthdays, { id: makeId(), name, day: date.getDate(), month: date.getMonth() + 1, year: date.getFullYear() }] }, "День рождения сохранён")
        if (!error) { birthdayName = ""; birthdayInput = "" }
    }
    async function deleteBirthday(id: string) { await persist({ ...data, birthdays: data.birthdays.filter(item => item.id !== id) }, "Дата удалена") }
    async function saveNote(note: Note) {
        if (saving || loading) return false
        const exists = data.notes.some(item => item.id === note.id)
        const next = exists ? data.notes.map(item => item.id === note.id ? note : item) : [...data.notes, note]
        return persist({ ...data, notes: next }, exists ? "Заметка обновлена" : "Заметка сохранена")
    }
    async function deleteNote(id: string) {
        if (saving || loading) return false
        return persist({ ...data, notes: data.notes.filter(item => item.id !== id) }, "Заметка удалена")
    }
    async function saveTraining(training: TrainingData, habit?: string) {
        if (saving || loading) return false
        const today = dayKey(new Date())
        const habits = habit ? data.habits.map(item => item.title === habit && item.createdOn <= today && !item.archivedOn ? { ...item, entries: { ...item.entries, [today]: item.target } } : item) : data.habits
        return persist({ ...data, training, habits }, "")
    }
</script>

<svelte:head><title>Центр — JARVIS</title></svelte:head>

<section class="center-shell">
    {#if $centerVoiceHint}<p class="voice-hint" role="status">{$centerVoiceHint}</p>{/if}
    <div class="center-inner">
        <header class="center-heading"><div><p class="eyebrow"><span class="signal"></span> ЛИЧНЫЙ ЦЕНТР JARVIS</p><h1>Всё важное — рядом.</h1><p class="subtitle">Выберите нужный раздел слева.</p></div><div class="today-pill"><span>СЕГОДНЯ</span><strong>{today.toLocaleDateString("ru-RU", { day: "numeric", month: "long" })}</strong></div></header>
        <div class="summary-bar"><div><span>●</span><b>{pendingCount}</b><small>активных напоминаний</small></div><div><span>◆</span><b>{data.birthdays.length}</b><small>важных дат</small></div><div><span>◈</span><b>{selectedReminders.length + selectedBirthdays.length}</b><small>событий в выбранный день</small></div></div>
        {#if error}<p class="status error" role="alert">{error}</p>{/if}
        {#if notice}<p class="status success" role="status">{notice}</p>{/if}
        <div class="center-layout">
            <nav class="section-list" aria-label="Разделы Центра">
                {#each sections as section}<button class:active={selectedSection === section.id} aria-pressed={selectedSection === section.id} on:click={() => { selectedSection = section.id; error = ""; notice = "" }}><span class="section-icon">{section.icon}</span><span>{section.label}</span>{#if !section.ready}<small>скоро</small>{/if}</button>{/each}
            </nav>
            <div class="section-content">
            {#if selectedSection === "calendar"}
            <section class="panel calendar-panel" aria-label="Календарь">
                <div class="panel-top"><div><p class="section-index">КАЛЕНДАРЬ</p><h2>{monthLabel}</h2></div><div class="month-controls"><button aria-label="Предыдущий месяц" on:click={() => shiftMonth(-1)}>‹</button><button aria-label="Следующий месяц" on:click={() => shiftMonth(1)}>›</button></div></div>
                <div class="calendar-grid">{#each weekdays as name}<span class="weekday">{name}</span>{/each}{#each calendarDays as day}{#if day}<button class="day" class:today={cellKey(day) === dayKey(today)} class:selected={cellKey(day) === selectedDay} aria-pressed={cellKey(day) === selectedDay} on:click={() => selectDay(day)}><span>{day}</span><i class="dots">{#if hasReminder(day)}<em class="reminder-dot"></em>{/if}{#if hasBirthday(day)}<em class="birthday-dot"></em>{/if}</i></button>{:else}<span class="day-spacer"></span>{/if}{/each}</div>
                <div class="calendar-legend"><span><i class="reminder-dot"></i> Напоминание</span><span><i class="birthday-dot"></i> День рождения</span></div>
                <div class="day-agenda"><div class="agenda-label">{new Date(`${selectedDay}T12:00`).toLocaleDateString("ru-RU", { day: "numeric", month: "long", weekday: "long" })}</div>{#if selectedReminders.length === 0 && selectedBirthdays.length === 0}<p>На этот день ничего не запланировано.</p>{/if}{#each selectedReminders as item}<p>◉ {item.title} · {formatDue(item.dueAt)}</p>{/each}{#each selectedBirthdays as item}<p>◆ День рождения: {item.name}</p>{/each}</div>
            </section>
            {:else if selectedSection === "news"}
                <CenterNews />
            {:else if selectedSection === "outlook"}
                <CenterOutlook />
            {:else if selectedSection === "tasks"}
                <CenterTasks tasks={data.tasks||[]} disabled={saving} onSave={(tasks)=>persist({...data,tasks},"Задачи сохранены")}/>
            {:else if selectedSection === "notes"}
                <CenterNotes notes={data.notes} disabled={saving || loading} onSave={saveNote} onDelete={deleteNote} />
            {:else if selectedSection === "voice-text"}
                <CenterVoiceText disabled={saving || loading} onSave={saveNote} />
            {:else if selectedSection === "reminders"}
                <section class="panel reminders-panel" aria-label="Напоминания">
                    <div class="panel-top"><div><p class="section-index">НЕ ЗАБЫТЬ</p><h2>Напоминания</h2></div><span class="panel-count">{pendingCount} активных</span></div>
                    <form class="entry-form" on:submit|preventDefault={addReminder}>
                        <input aria-label="Текст напоминания" maxlength="140" placeholder="Что нужно сделать?" bind:value={reminderTitle} required />
                        <ReminderSchedule bind:value={reminderDue}/>
                        <button class="add-btn reminder-submit" disabled={saving || loading}>{editingReminder?'Сохранить изменения':'+ Создать напоминание'}</button>
                        <fieldset class="advance-options"><legend>Предупредить заранее</legend><label><input type="checkbox" bind:group={reminderAdvance} value={1}/> За день</label><label><input type="checkbox" bind:group={reminderAdvance} value={2}/> За два дня</label><small>В назначенное время — всегда. Озвучка работает, пока JARVIS запущен.</small></fieldset>
                        {#if editingReminder}<button type="button" class="edit-reminder" on:click={()=>{editingReminder=null;reminderTitle='';reminderAdvance=[]}}>Отменить редактирование</button>{/if}
                    </form>
                    <div class="entry-list">{#if loading}<p class="empty">Загружаю…</p>{:else if reminders.length === 0}<p class="empty">Пока пусто. Добавьте первое напоминание.</p>{:else}{#each reminders as item,index}<div class="entry" class:done={item.done}><button class="check" aria-label={item.done ? "Вернуть напоминание" : "Отметить выполненным"} aria-pressed={item.done} on:click={() => toggleReminder(item)} disabled={saving}>{item.done ? "✓" : ""}</button><div class="entry-copy"><strong>{index+1}. {item.title}</strong><small>{formatDue(item.dueAt)}{item.advanceDays?.length?' · заранее за '+item.advanceDays.join(' и ')+' дн.':''}</small></div><button class="edit-reminder" aria-label={`Изменить напоминание ${item.title}`} on:click={()=>editReminder(item)} disabled={saving}>✎</button><button class="remove" aria-label="Удалить напоминание {item.title}" title="Удалить" on:click={() => deleteReminder(item.id)} disabled={saving}>×</button></div>{/each}{/if}</div>
                </section>
            {:else if selectedSection === "birthdays"}
                <section class="panel birthdays-panel" aria-label="Дни рождения">
                    <div class="panel-top"><div><p class="section-index">ВАЖНЫЕ ЛЮДИ</p><h2>Дни рождения</h2></div><span class="panel-count">{data.birthdays.length} дат</span></div>
                    <form class="entry-form" on:submit|preventDefault={addBirthday}>
                        <div class="form-row"><input aria-label="Имя именинника" maxlength="100" placeholder="Имя" bind:value={birthdayName} required /><input aria-label="Дата рождения" aria-describedby="birthday-date-hint" type="date" bind:value={birthdayInput} max="9999-12-31" required /></div>
                        <small id="birthday-date-hint" class="empty">Можно выбрать прошлую или будущую дату. Для будущего года запись появится в календаре начиная с указанного года, затем будет повторяться ежегодно.</small>
                        <button class="add-btn" disabled={saving || loading}>+ Сохранить дату</button>
                    </form>
                    <div class="entry-list">{#if loading}<p class="empty">Загружаю…</p>{:else if birthdays.length === 0}<p class="empty">Добавьте важную дату — JARVIS её запомнит.</p>{:else}{#each birthdays as item}<div class="entry"><span class="birthday-icon">◆</span><div class="entry-copy"><strong>{item.name}</strong><small>{birthdayCaption(item)}</small></div><button class="remove" aria-label="Удалить день рождения {item.name}" title="Удалить" on:click={() => deleteBirthday(item.id)} disabled={saving}>×</button></div>{/each}{/if}</div>
                </section>
            {:else if selectedSection === "weather"}
                <WeekWeather />
            {:else if selectedSection === "timer"}
                <CenterTimer />
            {:else if selectedSection === "request-history"}
                <CenterRequestHistory />
            {:else if selectedSection === "passwords"}
                <CenterPasswords />
            {:else if selectedSection === "habits"}
                <CenterHabits habits={data.habits} disabled={saving || loading} onSave={(habits) => persist({ ...data, habits }, "")} />
            {:else if selectedSection === "workouts"}
                {#if loading}<p class="empty">Загружаю тренировки…</p>{:else}<CenterTraining data={data.training} disabled={saving} onSave={saveTraining} />{/if}
            {:else}
                <section class="panel upcoming-panel"><p class="section-index">В РАЗРАБОТКЕ</p><h2>{currentSection.label}</h2><p>Раздел появится на одном из следующих этапов. Календарь, заметки, напоминания и дни рождения уже доступны в списке слева.</p>{#if selectedSection === "passwords"}<p>Менеджер паролей добавим отдельно, когда определим защиту мастер-паролем и надёжное шифрование.</p>{/if}</section>
            {/if}
            </div>
        </div>
        <p class="stage-note">Календарь, заметки, напоминания, дни рождения и прогноз погоды — в одном месте.</p>
    </div>
</section>

<style lang="scss">
    .advance-options label{white-space:nowrap;flex:none;min-width:125px;border:1px solid #31545a;border-radius:20px;padding:8px 12px;background:#143037;cursor:pointer}.advance-options label input{flex:none;width:16px;height:16px;margin:0;accent-color:#60f3e9}.advance-options small{flex-basis:100%}.reminder-submit{align-self:flex-end;padding:10px 16px}.advance-options{align-items:center}
    .advance-options{border:1px solid #31545a;border-radius:9px;padding:10px;display:flex;gap:12px;flex-wrap:wrap;color:#b8d5d6;font-size:12px}.advance-options legend{color:#60f3e9}.advance-options label{display:flex;align-items:center;gap:6px}.advance-options input{width:auto;accent-color:#60f3e9}.advance-options small{width:100%;color:#91abb1}.edit-reminder{border:1px solid #31545a;border-radius:8px;background:#122b30;color:#a1eee7;padding:5px 9px;cursor:pointer}
    .voice-hint{position:sticky;top:0;z-index:2;margin:0;padding:.7rem 1.5rem;border-bottom:1px solid #60f3e944;background:#103237;color:#bdfcf6;font-size:.75rem;line-height:1.6}
    .center-shell{--cyan:#60f3e9;--muted:#91abb1;--line:#25464c;--panel:#0d1b20;height:calc(100vh - 82px);overflow-y:auto;color:#eafafa;font-family:"Manrope Variable",sans-serif;background:radial-gradient(ellipse at 85% 0%,rgba(35,112,117,.2),transparent 46%),#091216}.center-inner{max-width:1120px;margin:0 auto;padding:1.6rem 1.65rem 2rem}.center-heading{display:flex;align-items:flex-end;justify-content:space-between;gap:1rem;margin-bottom:1.15rem}.eyebrow,.section-index{margin:0;color:var(--cyan);font-size:.68rem;font-weight:800;letter-spacing:.16em}.signal{display:inline-block;width:7px;height:7px;margin-right:.4rem;border-radius:50%;background:var(--cyan);box-shadow:0 0 10px var(--cyan)}h1{margin:.45rem 0 .2rem;font-size:clamp(1.4rem,3vw,2rem);font-weight:700;letter-spacing:-.035em;line-height:1.12}.subtitle{margin:0;color:var(--muted);font-size:.8rem}.today-pill{display:flex;flex-direction:column;gap:.1rem;flex:none;padding:.65rem .85rem;border:1px solid var(--line);border-radius:9px;background:#11262b}.today-pill span{color:var(--cyan);font-size:.55rem;font-weight:800;letter-spacing:.17em}.today-pill strong{font-size:.82rem}.summary-bar{display:grid;grid-template-columns:repeat(3,1fr);gap:.6rem;margin-bottom:1rem}.summary-bar>div{display:flex;align-items:baseline;gap:.55rem;padding:.7rem .85rem;border:1px solid #24454b;border-radius:8px;background:linear-gradient(120deg,#10252a,#0d1b20)}.summary-bar span{color:var(--cyan);font-size:.7rem}.summary-bar b{font-size:1rem}.summary-bar small{color:var(--muted);font-size:.67rem}.center-grid{display:grid;grid-template-columns:minmax(0,1.1fr) minmax(0,.9fr);gap:1rem}.right-stack{display:flex;flex-direction:column;gap:1rem;min-width:0}.panel{min-width:0;padding:1rem;border:1px solid var(--line);border-radius:11px;background:linear-gradient(140deg,rgba(20,48,54,.7),transparent 55%),var(--panel);box-shadow:0 12px 28px #0002}.panel-top{display:flex;align-items:center;justify-content:space-between;gap:.5rem;margin-bottom:.85rem}.panel-top h2{margin:.18rem 0 0;font-size:1.08rem;font-weight:700;letter-spacing:-.02em;text-transform:capitalize}.month-controls{display:flex;gap:.3rem}.month-controls button{display:grid;place-items:center;width:29px;height:29px;border:1px solid #376269;border-radius:5px;background:#18353b;color:#d8fbfa;font-size:1.3rem;line-height:1;cursor:pointer}.month-controls button:hover,.month-controls button:focus-visible{border-color:var(--cyan)}.calendar-grid{display:grid;grid-template-columns:repeat(7,minmax(0,1fr));gap:.18rem}.weekday{padding:.35rem 0;text-align:center;color:#83a9af;font-size:.63rem;font-weight:800;text-transform:uppercase}.day,.day-spacer{display:flex;flex-direction:column;align-items:center;justify-content:center;min-width:0;height:47px;border:1px solid transparent;border-radius:7px}.day{background:#112329;color:#dceced;font:600 .85rem "Manrope Variable",sans-serif;cursor:pointer;transition:background .16s,border-color .16s,transform .16s}.day:hover{transform:translateY(-2px);background:#1b3a41}.day.today{border-color:#52999c}.day.selected{background:#147b81;border-color:var(--cyan);color:#fff;box-shadow:0 0 14px #2ee8e82b}.dots{display:flex;gap:3px;height:4px;margin-top:2px}.dots em,.calendar-legend i{display:block;width:4px;height:4px;border-radius:50%}.reminder-dot{background:#60f3e9}.birthday-dot{background:#d9a8ff}.calendar-legend{display:flex;gap:1rem;margin:.8rem 0;color:#9bb8bb;font-size:.62rem}.calendar-legend span{display:flex;align-items:center;gap:.3rem}.day-agenda{padding:.8rem;border:1px solid #24454b;border-radius:8px;background:#0b191d}.agenda-label{margin-bottom:.35rem;color:var(--cyan);font-size:.72rem;font-weight:800;text-transform:capitalize}.day-agenda p{margin:.25rem 0;color:#b8cdcf;font-size:.7rem}.entry-form{display:flex;flex-direction:column;gap:.4rem;margin-bottom:.8rem}.entry-form input{min-width:0;width:100%;padding:.5rem .6rem;border:1px solid #31545a;border-radius:6px;outline:none;background:#09171b;color:#e7ffff;font:500 .72rem "Manrope Variable",sans-serif;color-scheme:dark}.entry-form input:focus{border-color:var(--cyan)}.form-row{display:flex;gap:.4rem}.form-row>*{min-width:0}.add-btn{flex:none;padding:.5rem .7rem;border:1px solid #65f2e9;border-radius:6px;background:#1aa7a8;color:#061617;font:800 .7rem "Manrope Variable",sans-serif;cursor:pointer}.add-btn:hover{background:#65f2e9}.add-btn:disabled{opacity:.55;cursor:wait}.entry-list{max-height:185px;overflow-y:auto}.entry{display:flex;align-items:center;gap:.55rem;padding:.54rem .2rem;border-top:1px solid #20363b}.entry-copy{display:flex;flex:1;min-width:0;flex-direction:column;gap:.1rem}.entry-copy strong{overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-size:.75rem;font-weight:700}.entry-copy small{color:#8ca8ac;font-size:.65rem}.entry.done .entry-copy{opacity:.53;text-decoration:line-through}.check{width:19px;height:19px;flex:none;border:1px solid #5aaeb0;border-radius:5px;background:transparent;color:var(--cyan);cursor:pointer}.check[aria-pressed="true"]{background:#164f52}.birthday-icon{color:#d9a8ff;font-size:.7rem}.remove{border:0;background:transparent;color:#8aa2a6;font-size:1.2rem;cursor:pointer}.remove:hover{color:#ff9c9c}.empty{margin:.45rem 0;color:#91acaf;font-size:.7rem}.panel-count{color:#a2c8ca;font-size:.63rem;white-space:nowrap}.status{margin:0 0 .8rem;padding:.55rem .7rem;border-radius:6px;font-size:.72rem}.error{border:1px solid #a55c5c;background:#321b20;color:#ffd2d2}.success{border:1px solid #3c8f87;background:#123d3b;color:#b6f7ea}.stage-note{margin:1rem 0 0;color:#6f989b;font-size:.62rem;letter-spacing:.08em}@media(max-width:720px){.center-grid{grid-template-columns:1fr}.center-inner{padding:1rem}.today-pill{display:none}.summary-bar small{display:none}.summary-bar>div{justify-content:center}.center-shell{height:calc(100vh - 75px)}}
    .center-layout{display:grid;grid-template-columns:218px minmax(0,1fr);align-items:start;gap:1rem}.section-list{display:flex;flex-direction:column;gap:.34rem;max-height:calc(100vh - 278px);overflow-y:auto;padding:.45rem;border:1px solid var(--line);border-radius:10px;background:#0d1b20}.section-list button{display:flex;align-items:center;gap:.65rem;min-width:0;width:100%;padding:.7rem .6rem;border:1px solid transparent;border-radius:7px;background:transparent;color:#b4ced0;font:700 .73rem "Manrope Variable",sans-serif;text-align:left;cursor:pointer;transition:background .15s,border-color .15s,color .15s}.section-list button:hover{background:#183138;color:#fff}.section-list button.active{border-color:#5ae3dc;background:linear-gradient(95deg,#15575c,#153139);color:#fff;box-shadow:0 0 18px #36d9ce1c}.section-list button:focus-visible{outline:2px solid #fff;outline-offset:2px}.section-icon{display:grid;place-items:center;flex:none;width:20px;color:var(--cyan);font-size:.9rem}.section-list small{margin-left:auto;color:#779599;font-size:.58rem;font-weight:500}.section-content{min-width:0}.section-content>.panel{min-height:400px}.section-content .entry-list{max-height:340px}.notes-layout{display:grid;grid-template-columns:180px minmax(0,1fr);gap:.8rem}.note-list{display:flex;flex-direction:column;gap:.3rem;max-height:320px;overflow-y:auto}.note-list button{display:flex;flex-direction:column;gap:.16rem;padding:.58rem;border:1px solid #28494f;border-radius:6px;background:#11262b;color:#e9fafa;text-align:left;cursor:pointer}.note-list button.active{border-color:var(--cyan);background:#164147}.note-list strong{overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-size:.72rem}.note-list small{color:#8eb0b3;font-size:.62rem}.note-editor{display:flex;flex-direction:column;gap:.45rem;min-width:0}.note-editor input,.note-editor textarea{width:100%;padding:.6rem;border:1px solid #31545a;border-radius:6px;outline:none;background:#09171b;color:#e7ffff;font:500 .73rem "Manrope Variable",sans-serif}.note-editor textarea{min-height:205px;resize:vertical;line-height:1.5}.note-editor input:focus,.note-editor textarea:focus{border-color:var(--cyan)}.note-actions{display:flex;gap:.5rem}.delete-note{padding:.5rem .7rem;border:1px solid #7c4549;border-radius:6px;background:#301b20;color:#ffb3b3;font:700 .7rem "Manrope Variable",sans-serif;cursor:pointer}.notes-hint{margin:.8rem 0 0;color:#7fa5a9;font-size:.66rem}.upcoming-panel h2{margin:.5rem 0 1rem;font-size:1.4rem}.upcoming-panel p:not(.section-index){max-width:45ch;color:#a8c1c3;font-size:.78rem;line-height:1.6}.upcoming-panel p+p{margin-top:1rem}@media(max-width:720px){.center-layout{grid-template-columns:1fr}.section-list{flex-direction:row;overflow-x:auto;max-height:none}.section-list button{flex:none;width:auto;white-space:nowrap}.section-list small{display:none}}@media(max-width:520px){.notes-layout{grid-template-columns:1fr}.note-list{flex-direction:row;overflow-x:auto}.note-list button{min-width:130px}}
</style>

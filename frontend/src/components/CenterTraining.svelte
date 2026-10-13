<script lang="ts">
    import { onMount } from "svelte"
    import { trainingVoiceTab } from '@/lib/centerVoice'
    import TrainingStats from "./TrainingStats.svelte"
    import TrainingBody from "./TrainingBody.svelte"
    import TrainingPreferences from "./TrainingPreferences.svelte"
    import {dailyProposal,chargeProposal,plannerLevels,type Proposal} from "@/lib/trainingPlanner"
    import { moodLabels } from "@/lib/trainingStats"
    import { dayKey } from "@/lib/center"
    import { timerState, selectTimer, startTimer, pauseTimer, resetTimer, formatTimer } from "@/lib/timer"
    import { defaultTraining, newTraining, changeTrainingStatus, trainingDuration, workingSets, trainingVolume, estimatedMax, previousTraining, progression, validateSet, trainingId, type TrainingData, type TrainingSession, type TrainingPlan, type PlanItem } from "@/lib/training"
    export let data: TrainingData | undefined
    export let disabled = false
    export let mobile = false
    export let beforeTabChange: () => boolean = () => true
    export let onSave: (data: TrainingData, habit?: string) => Promise<boolean>
    const defaults = defaultTraining()
    const tabs = [{ id: "today", label: "Сегодня" }, { id: "charge", label: "Зарядка" }, { id: "profile", label: "Мои параметры" }, { id: "plans", label: "Программы" }, { id: "exercises", label: "Упражнения" }, { id: "history", label: "История" }, { id: "stats", label: "Статистика" }, { id: "records", label: "Рекорды" }]
    const chargeSteps: import("@/lib/training").ChargeStep[] = [
        {id:"walk",name:"Шаги на месте",hint:"1 минута · спокойный темп"},
        {id:"shoulders",name:"Круговые движения плечами",hint:"8–10 раз в каждую сторону · без рывков"},
        {id:"arms",name:"Мягкие махи руками",hint:"1 минута · комфортная амплитуда"},
        {id:"squat",name:"Приседания или вставания со стула",hint:"8–10 раз · выберите удобный вариант"},
        {id:"heels",name:"Подъёмы на носки",hint:"10–15 раз · держитесь за опору при необходимости"},
        {id:"finish",name:"Спокойные шаги и дыхание",hint:"1 минута · плавное завершение"},
    ]
    $: chargeDate = dayKey(new Date(now))
    $: chargeDone = training.morningExercise?.[chargeDate] || []
    $: visibleChargeSteps = (training.chargeSettings?.steps || chargeSteps).filter(item=>item.enabled!==false)
    async function toggleCharge(id:string) {
        const entries = chargeDone.includes(id) ? chargeDone.filter(item=>item!==id) : [...chargeDone,id]
        await persist({...training,morningExercise:{...training.morningExercise,[chargeDate]:entries},chargeLevels:{...training.chargeLevels,[chargeDate]:Math.max(training.chargeLevels?.[chargeDate]??0,training.chargeSettings?.level??1)}})
    }
    const warmups = ["Общая разминка", "Подготовка суставов и движений", "Разминочные подходы"]
    let tab = "today", planId = "push", exerciseIndex = 0, viewId = "", selectedKey = ""
    export function currentTab() { return tab }
    $: tab = $trainingVoiceTab
    let weight = 0, reps = 8, rpe: number | null = 8, isWarmup = false
    let busy = false, error = "", now = Date.now(), note = ""
    let mood: number | null = null
    let fatigue=2, pain=false, suggested:Proposal|null=null
    let chargeSuggested:ReturnType<typeof chargeProposal>|null=null
    function recommend(random=false){
        if(tab==="charge")chargeSuggested=chargeProposal(training.chargeSettings?.steps||chargeSteps,training,{fatigue,pain,mood},random)
        else suggested=dailyProposal(training,{fatigue,pain,mood},random)
    }
    async function applyCharge(){
        if(!chargeSuggested||chargeSuggested.blocked)return
        if(!confirm("Применить предложенную зарядку? Сохранённые настройки комплекса будут заменены, отметки выполнения останутся."))return
        if(await persist({...training,chargeSettings:{level:chargeSuggested.level,steps:chargeSuggested.steps}}))chargeSuggested=null
    }
    let bodyGroup = ""
    let exerciseName = "", muscle = "Грудь", equipment = "Штанга", libraryFilter = "Все"
    let editing: TrainingPlan | null = null, addExerciseId = "bench"
    let planLevel = data?.profile?.preferredLevel ?? 1
    const levelNames = ["Очень лёгкий","Лёгкий","Умеренно лёгкий","Средний","Выше среднего","Интенсивный","Продвинутый"]
    function levelPlan() {
        const source = training.plans.find(item=>item.id===planId) || training.plans[0]
        if (!source) {editPlan();return}
        editing = {...source,id:trainingId(),name:`${source.name} · ${levelNames[planLevel]}`,items:source.items.map(item=>({...item,sets:[1,1,2,3,3,4,4][planLevel]}))}
    }
    $: training = data || defaults
    $: active = [...training.sessions].reverse().find(session => session.status === "active" || session.status === "paused")
    $: session = viewId ? training.sessions.find(item => item.id === viewId) : active
    $: current = session?.items[exerciseIndex]
    $: previous = current ? previousTraining(training, current.exerciseId, session?.id) : undefined
    $: previousSets = previous && current ? workingSets(previous, current.exerciseId) : []
    $: sets = session && current ? session.sets.filter(set => set.exerciseId === current.exerciseId) : []
    $: done = session ? workingSets(session).length : 0
    $: total = session ? session.items.reduce((sum, item) => sum + item.sets, 0) : 0
    $: restLabel = session ? `Отдых · ${session.name}` : ""
    $: ownRest = Boolean(session && $timerState.owner === session.id)
    $: locked = disabled || busy
    $: canRecord = session?.status === "active" && !viewId
    $: if (session && current && selectedKey !== `${session.id}/${current.exerciseId}`) {
        selectedKey = `${session.id}/${current.exerciseId}`
        weight = previousSets[0]?.weight ?? 0; reps = current.maxReps; rpe = 8; note = session.note; mood = session.mood ?? null
    }
    $: history = [...training.sessions].sort((a, b) => b.startedAt.localeCompare(a.startedAt))
    $: completed = training.sessions.filter(session => session.status === "completed")
    $: groups = [...new Set(training.exercises.map(exercise => exercise.muscle))]
    $: muscleCounts = groups.map(group => ({ group, sets: completed.reduce((sum, session) => sum + workingSets(session).filter(set => session.items.find(item => item.exerciseId === set.exerciseId)?.muscle === group).length, 0) }))
    $: maxCount = Math.max(1, ...muscleCounts.map(item => item.sets))
    $: bodyCounts = tab === "today" && session ? groups.map(group => ({group, sets: workingSets(session).filter(set => session.items.find(item => item.exerciseId === set.exerciseId)?.muscle === group).length})) : muscleCounts
    function chooseBody(group: string) {
        if (tab === "exercises") libraryFilter = group || "Все"
        if (tab === "today" && session && group) {
            const index = session.items.findIndex(item => item.muscle === group)
            if (index >= 0) exerciseIndex = index
        }
        if (tab === "plans" && group) addExerciseId = training.exercises.find(item => item.muscle === group)?.id || addExerciseId
    }
    function chooseExercise(exercise: {id: string; muscle: string}) {
        const index = tab === "today" ? session?.items.findIndex(item => item.exerciseId === exercise.id) ?? -1 : -1
        if (index >= 0) exerciseIndex = index
        else { tab = "exercises"; libraryFilter = exercise.muscle }
    }
    $: records = training.exercises.map(exercise => {
        const sets = completed.flatMap(session => workingSets(session, exercise.id))
        return { exercise, best: sets.reduce<(typeof sets)[number] | undefined>((best, set) => !best || set.weight > best.weight || (set.weight === best.weight && set.reps > best.reps) ? set : best, undefined), estimate: Math.max(0, ...sets.map(set => estimatedMax(set) || 0)) }
    }).filter(item => item.best)
    onMount(() => { const timer = setInterval(() => now = Date.now(), 1000); return () => clearInterval(timer) })
    async function persist(next: TrainingData, habit?: string) {
        if (locked) return false
        busy = true; error = ""
        try { return await onSave(next, habit) }
        catch (e) { error = `Не удалось сохранить тренировку: ${String(e)}`; return false }
        finally { busy = false }
    }
    async function saveSession(next: TrainingSession, habit?: string) {
        return persist({ ...training, sessions: training.sessions.map(item => item.id === next.id ? next : item) }, habit)
    }
    async function begin() {
        if (active) { viewId = ""; tab = "today"; return }
        if(suggested && (pain||fatigue>=4||dailyProposal(training,{fatigue,pain,mood}).rest)){suggested=dailyProposal(training,{fatigue,pain,mood});error="Условия изменились. Посмотрите обновлённое предложение.";return}
        const plan = suggested?.plan || training.plans.find(plan => plan.id === planId)
        if (!plan) { error = "Выберите программу"; return }
        const latestWeight = [...(training.profile?.measurements || [])].filter(item => item.date <= dayKey(new Date())).sort((a,b) => a.date.localeCompare(b.date)).at(-1)
        const next = newTraining(plan, training.exercises, Date.now(), { mood, heightCm: training.profile?.heightCm ?? null, bodyWeight: latestWeight?.weight ?? null, ...(suggested?.plan?{plannedLevel:suggested.level}:{}) })
        if (await persist({ ...training, sessions: [...training.sessions, next] })) { exerciseIndex = 0; viewId = "" }
    }
    async function status(next: TrainingSession["status"]) {
        if (!session || viewId) return
        if (next === "completed") {
            if (!done) { error = "Сначала запишите хотя бы один рабочий подход."; return }
            if (done < total && !confirm("Завершить тренировку с неполным планом? Сохранятся только записанные подходы.")) return
        }
        if (next === "cancelled" && !confirm("Отменить тренировку? Подходы останутся в истории, но не попадут в рекорды.")) return
        const id = session.id
        const saved = await saveSession(changeTrainingStatus(session, next), next === "completed" ? "Сходил в зал" : undefined)
        if (saved && (next === "completed" || next === "cancelled")) {
            if (ownRest) { pauseTimer(); resetTimer() }
            viewId = id
        }
    }
    async function record() {
        if (!session || !current || !canRecord || locked) return
        if (!validateSet(Number(weight), Number(reps), rpe)) { error = "Проверьте вес (0–1000 кг), повторы (1–500) и RPE (6–10 или не указан)."; return }
        if (!isWarmup && workingSets(session, current.exerciseId).length >= current.sets) { error = "Целевые подходы уже записаны. Выберите следующее упражнение."; return }
        const warmup = isWarmup
        const restSeconds = current.rest, timerLabel = restLabel, timerOwner = session.id
        const next = { ...session, sets: [...session.sets, { id: trainingId(), exerciseId: current.exerciseId, weight: Number(weight), reps: Number(reps), rpe, warmup, at: new Date().toISOString() }] }
        if (await saveSession(next)) {
            if (!warmup && ($timerState.phase !== "running" || ownRest || confirm("Другой таймер уже идёт. Заменить его отдыхом после подхода?"))) {
                selectTimer({ id: "training-rest", label: timerLabel, seconds: restSeconds, icon: "◷", owner: timerOwner }); startTimer()
            }
        }
    }
    async function removeSet(id: string) {
        if (!session || !canRecord || !confirm("Удалить этот подход из тренировки?")) return
        await saveSession({ ...session, sets: session.sets.filter(set => set.id !== id) })
    }
    async function markWarmup(index: number) {
        if (!session || !canRecord) return
        await saveSession({ ...session, warmup: session.warmup.map((done, i) => i === index ? !done : done) })
    }
    async function addExercise() {
        if (!exerciseName.trim()) return
        if (await persist({ ...training, exercises: [...training.exercises, { id: trainingId(), name: exerciseName.trim(), muscle: muscle.trim(), equipment: equipment.trim() }] })) exerciseName = ""
    }
    function editPlan(plan?: TrainingPlan) {
        editing = plan ? JSON.parse(JSON.stringify(plan)) : { id: trainingId(), name: "Моя программа", items: [] }
    }
    function appendExercise() {
        if (!editing || editing.items.some(item => item.exerciseId === addExerciseId)) return
        editing = { ...editing, items: [...editing.items, { exerciseId: addExerciseId, sets: 3, minReps: 8, maxReps: 12, rest: 120 }] }
    }
    async function savePlan() {
        if (!editing || !editing.name.trim() || !editing.items.length) { error = "Укажите название и добавьте упражнения."; return }
        const valid = editing.items.every(item => [item.sets, item.minReps, item.maxReps, item.rest].every(Number.isInteger) && item.sets >= 1 && item.sets <= 20 && item.minReps >= 1 && item.maxReps >= item.minReps && item.maxReps <= 500 && item.rest >= 10 && item.rest <= 1800)
        if (!valid) { error = "Проверьте цели: 1–20 подходов, 1–500 повторов, отдых 10–1800 секунд."; return }
        const plan = { ...editing, name: editing.name.trim() }
        const plans = training.plans.some(item => item.id === plan.id) ? training.plans.map(item => item.id === plan.id ? plan : item) : [...training.plans, plan]
        if (await persist({ ...training, plans })) { planId = plan.id; editing = null }
    }
    function date(value: string) { return new Date(value).toLocaleString("ru-RU", { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" }) }
    function sessionState(value: TrainingSession["status"]) { return ({ active: "В процессе", paused: "Пауза", completed: "Завершена", cancelled: "Отменена" })[value] }
</script>

<section class="training" aria-label="Тренировки JARVIS">
    <header><div><p class="kicker">JARVIS / TRAINING</p><h2>Твой следующий шаг.</h2></div><span class="date">{new Date(now).toLocaleDateString("ru-RU", { day: "numeric", month: "long" })}</span></header>
    <nav class="tabs" aria-label="Разделы тренировок">{#each tabs as item}<button class:active={tab === item.id} aria-pressed={tab === item.id} on:click={() => {if(beforeTabChange())tab = item.id}}>{item.label}</button>{/each}</nav>
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    {#if (tab === "today" && !session) || tab === "charge"}
        <div class="panel daily-planner"><p class="kicker">JARVIS · ПОДБОР НА СЕГОДНЯ</p><h3>План с учётом последних дней</h3><div class="form-row"><label>Усталость сегодня<select bind:value={fatigue} on:change={()=>{suggested=null;chargeSuggested=null}}><option value={1}>1 · Отдохнул</option><option value={2}>2 · Обычная</option><option value={3}>3 · Устал</option><option value={4}>4 · Сильно устал</option><option value={5}>5 · Нужен отдых</option></select></label><label class="checkbox"><input type="checkbox" bind:checked={pain} on:change={()=>{suggested=null;chargeSuggested=null}}/>Есть боль или плохое самочувствие</label></div><small>Профиль: {training.profile?.experience||"опыт не указан"} · {training.profile?.heightCm??"—"} см · жир {training.profile?.bodyFatPercent??"—"}%. Учитываются записанные подходы, RPE и выполненная зарядка. Незаписанные занятия неизвестны.</small><div class="form-row"><button class="primary" disabled={locked} on:click={()=>recommend(false)}>Подобрать на сегодня</button><button disabled={locked} on:click={()=>recommend(true)}>🎲 Случайный подходящий вариант</button></div>
        {#if tab === "today" && suggested}<div class="proposal"><h4>{suggested.rest?"Сегодня — отдых / лёгкая активность":suggested.plan?.name}</h4>{#if suggested.plan}<p class="empty">{suggested.plan.items.map(item=>`${training.exercises.find(e=>e.id===item.exerciseId)?.name}: ${item.sets} × ${item.minReps}–${item.maxReps}`).join(" · ")}</p>{/if}<ul>{#each suggested.reasons as reason}<li>{reason}</li>{/each}</ul><button on:click={()=>suggested=null}>Вернуться к ручному выбору</button></div>{/if}
        {#if tab === "charge" && chargeSuggested}<div class="proposal"><h4>{chargeSuggested.blocked?"Сегодня лучше пропустить комплекс":`Зарядка · ${plannerLevels[chargeSuggested.level]}`}</h4><ul>{#each chargeSuggested.reasons as reason}<li>{reason}</li>{/each}</ul>{#each chargeSuggested.steps as step}<p class="empty">{step.name} · {step.hint}</p>{/each}<button class="primary" disabled={locked||chargeSuggested.blocked} on:click={applyCharge}>Применить зарядку на сегодня</button></div>{/if}
        <small>Подбор по правилам, не AI-диагноз и не гарантия восстановления. <a href="https://www.niddk.nih.gov/health-information/weight-management/healthy-eating-physical-activity-for-life/health-tips-for-adults" target="_blank" rel="noopener noreferrer">О постепенной нагрузке и отдыхе</a>.</small></div>
    {/if}
    {#if tab !== "stats" && (!mobile || tab === "today" || tab === "exercises")}<div class="body-navigation"><TrainingBody bind:selected={bodyGroup} counts={bodyCounts} exercises={tab === "today" && session ? session.items : training.exercises} metric={tab === "today" && session ? "Рабочие подходы этой тренировки" : "Рабочие подходы завершённых тренировок"} caption="Только записанные рабочие подходы, без разминки." onSelect={chooseBody} onExercise={chooseExercise}/></div>{/if}
    {#if tab === "today"}
        {#if !session}<div class="panel intro"><p class="kicker">ГОТОВ К ТРЕНИРОВКЕ</p><h3>Выбери план на сегодня</h3><p>Записывай реальные подходы. JARVIS сохранит результаты и покажет прошлую тренировку.</p><select bind:value={planId} on:change={()=>suggested=null} aria-label="Программа тренировки">{#each training.plans as plan}<option value={plan.id}>{plan.name}</option>{/each}</select><label>Настроение перед тренировкой<select bind:value={mood} on:change={()=>suggested=null}><option value={null}>Не указано</option>{#each [1,2,3,4,5] as value}<option value={value}>{value} · {moodLabels[value]}</option>{/each}</select></label><small>Рост и вес можно записать во вкладке «Статистика». Они сохранятся вместе с новой тренировкой.</small><button class="primary" disabled={locked || suggested?.rest} on:click={begin}>Начать тренировку</button><small>Шаблоны — примеры, а не персональное назначение. Подстройте упражнения, цели и отдых под свой план.</small></div>
        {:else}<div class="session-header"><div><h3>{session.name}</h3><small>{sessionState(session.status)} · {date(session.startedAt)}</small></div><div class="session-actions">{#if !viewId}<button disabled={locked} on:click={() => status(session?.status === "active" ? "paused" : "active")}>{session.status === "active" ? "Пауза" : "Продолжить"}</button><button disabled={locked} on:click={() => status("completed")}>Завершить</button><button disabled={locked} on:click={() => status("cancelled")}>Отменить</button>{:else}<button on:click={() => { viewId = ""; exerciseIndex = 0 }}>{active ? "К текущей" : "Новая тренировка"}</button>{/if}</div></div>
        <div class="session-context"><span>Рост: {session.heightCm ?? "—"} см</span><span>Вес тела: {session.bodyWeight ?? "—"} кг</span><span>Настроение: {moodLabels[session.mood || 0] || "не указано"}</span>{#if !viewId}<label>Настроение перед тренировкой<select bind:value={mood}><option value={null}>Не указано</option>{#each [1,2,3,4,5] as value}<option value={value}>{value} · {moodLabels[value]}</option>{/each}</select></label><button disabled={locked} on:click={() => session && saveSession({ ...session, mood })}>Сохранить оценку</button>{/if}</div>
        <div class="training-grid"><aside class="panel route"><p class="kicker">МАРШРУТ НА СЕГОДНЯ</p><div class="progress"><span style={`width:${Math.min(100,done / total * 100)}%`}></span></div><small>{done} / {total} рабочих подходов</small><div class="warmups"><h4>Подготовка</h4>{#each warmups as text, index}<label><input type="checkbox" checked={session.warmup[index]} disabled={locked || !canRecord} on:change={() => markWarmup(index)}/>{text}</label>{/each}</div><div class="exercise-list">{#each session.items as item, index}<button class:selected={exerciseIndex === index} aria-pressed={exerciseIndex === index} on:click={() => exerciseIndex = index}><span class="number">{workingSets(session, item.exerciseId).length >= item.sets ? "✓" : index + 1}</span><span><strong>{item.name}</strong><small>{item.sets} × {item.minReps}–{item.maxReps} · {workingSets(session, item.exerciseId).length}/{item.sets}</small></span></button>{/each}</div></aside>
        <main class="panel current">{#if current}<p class="kicker">ТЕКУЩЕЕ УПРАЖНЕНИЕ · {exerciseIndex + 1}/{session.items.length}</p><h3>{current.name}</h3><div class="tags"><span>{current.muscle}</span><span>{current.equipment}</span></div><div class="context"><div><small>Цель</small><strong>{current.sets} × {current.minReps}–{current.maxReps}</strong></div><div><small>Отдых</small><strong>{formatTimer(current.rest)}</strong></div></div><div class="previous"><small>Предыдущий раз · {previous ? date(previous.startedAt) : "ещё нет"}</small><p>{previousSets.length ? previousSets.map(set => `${set.weight} кг × ${set.reps}`).join(" / ") : "Первая запись. Укажите подходящий вам рабочий вес."}</p></div>
        <div class="sets">{#each sets as set, index}<div class="set"><span>{set.warmup ? "Р" : index + 1}</span><strong>{set.weight} кг × {set.reps}</strong><small>RPE {set.rpe ?? "—"}{set.warmup ? " · разминка" : ""}</small>{#if canRecord}<button disabled={locked} aria-label={`Удалить подход ${index + 1}`} on:click={() => removeSet(set.id)}>×</button>{/if}</div>{/each}{#if !sets.length}<p class="empty">Подходов пока нет.</p>{/if}</div>
        {#if canRecord}
            <form class="set-form" on:submit|preventDefault={record}>
                <div class="inputs">
                    <div class="input-group"><label for="training-weight">Вес, кг</label><div class="stepper">
                        <button type="button" aria-label="Уменьшить вес" on:click={() => weight = Math.max(0, Number(weight) - 2.5)}>−</button>
                        <input id="training-weight" type="number" min="0" max="1000" step="0.25" bind:value={weight} required/>
                        <button type="button" aria-label="Увеличить вес" on:click={() => weight = Math.min(1000, Number(weight) + 2.5)}>+</button>
                    </div></div>
                    <div class="input-group"><label for="training-reps">Повторы</label><div class="stepper">
                        <button type="button" aria-label="Уменьшить повторы" on:click={() => reps = Math.max(1, Number(reps) - 1)}>−</button>
                        <input id="training-reps" type="number" min="1" max="500" step="1" bind:value={reps} required/>
                        <button type="button" aria-label="Увеличить повторы" on:click={() => reps = Math.min(500, Number(reps) + 1)}>+</button>
                    </div></div>
                </div>
                <small>Для собственного веса без отягощения — 0 кг. Вес гантелей записывайте каждый раз одинаковым способом.</small>
                <label>Сложность (RPE)<select bind:value={rpe}><option value={null}>Не указана</option>{#each [6,7,8,9,10] as value}<option value={value}>{value} · {({6:"Легко",7:"Нормально",8:"Тяжело",9:"Почти предел",10:"Предел"})[value]}</option>{/each}</select></label>
                <label class="checkbox"><input type="checkbox" bind:checked={isWarmup}/>Разминочный подход · не учитывать в рабочих метриках</label>
                <button class="primary" disabled={locked || (!isWarmup && workingSets(session,current.exerciseId).length >= current.sets)}>✓ Завершить подход</button>
            </form>
        {/if}
        <div class="exercise-nav"><button disabled={exerciseIndex === 0} on:click={() => exerciseIndex--}>← Назад</button><button disabled={exerciseIndex >= session.items.length - 1} on:click={() => exerciseIndex++}>Далее →</button></div>{/if}</main>
        <aside class="right"><div class="panel rest"><p class="kicker">ТАЙМЕР ОТДЫХА</p><div class="rest-dial" style={`--progress:${ownRest ? $timerState.remaining / $timerState.total * 100 : 0}%`}><div><time>{ownRest ? formatTimer($timerState.remaining) : "—:—"}</time><small>{ownRest ? ({idle:"Готов",running:"Отдых идёт",paused:"Пауза",finished:"Можно продолжить"})[$timerState.phase] : "После подхода"}</small></div></div><div class="rest-actions"><button disabled={!ownRest || $timerState.phase === "finished"} on:click={() => $timerState.phase === "running" ? pauseTimer() : startTimer()}>{$timerState.phase === "running" ? "Пауза" : "Продолжить"}</button><button disabled={!ownRest} on:click={() => { pauseTimer(); resetTimer() }}>Сброс</button></div><small>Общий таймер JARVIS: отсчёт продолжается при смене раздела.</small></div>
        <div class="panel summary"><p class="kicker">СВОДКА ТРЕНИРОВКИ</p><div class="metrics"><div><strong>{formatTimer(Math.floor(trainingDuration(session,now) / 1000))}</strong><small>время по таймеру</small></div><div><strong>{done}/{total}</strong><small>рабочих подходов</small></div><div><strong>{trainingVolume(session).toLocaleString("ru-RU")}</strong><small>объём, кг · вес × повторы</small></div></div></div><div class="panel analysis"><p class="kicker">JARVIS · ПО ВАШИМ ДАННЫМ</p><p>{current ? progression(current, previousSets) : "Выберите упражнение"}</p><small>Правило по записанным подходам, не AI-прогноз. Смена веса только по вашему решению.</small><label>Заметка к тренировке<textarea bind:value={note} maxlength="2000" disabled={!canRecord || locked}/></label>{#if canRecord}<button disabled={locked} on:click={() => session && saveSession({ ...session, note })}>Сохранить заметку</button>{/if}</div></aside></div>{/if}
    {:else if tab === "charge"}
        <div class="panel charge-panel"><p class="kicker">МЯГКОЕ НАЧАЛО ДНЯ</p><h3>Зарядка · ваш комплекс</h3><p class="empty">Пример лёгкого комплекса без оборудования. Выполняйте в комфортном темпе; при боли остановитесь. Это не персональная программа.</p><div class="section-heading"><strong>{visibleChargeSteps.filter(step=>chargeDone.includes(step.id)).length} / {visibleChargeSteps.length} выполнено сегодня</strong><small>Отметки сохраняются по дням</small></div><div class="charge-list">{#each visibleChargeSteps as step,index}<label class:charge-complete={chargeDone.includes(step.id)}><input type="checkbox" checked={chargeDone.includes(step.id)} disabled={locked} on:change={()=>toggleCharge(step.id)}/><span class="number">{index+1}</span><span><strong>{step.name}</strong><small>{step.hint}</small></span></label>{/each}</div><small>Зарядка учитывается отдельно: её отметки не увеличивают рабочие подходы и силовые рекорды.</small></div>
        <TrainingPreferences mode="charge" data={training} defaults={chargeSteps} disabled={locked} onSave={persist}/>
    {:else if tab === "profile"}<TrainingPreferences data={training} disabled={locked} onSave={persist}/>
    {:else if tab === "plans"}<div class="panel"><h3>Вариант программы по уровню</h3><div class="form-row"><label>Базовая программа<select bind:value={planId}>{#each training.plans as plan}<option value={plan.id}>{plan.name}</option>{/each}</select></label><label>Уровень объёма<select bind:value={planLevel}>{#each levelNames as name,index}<option value={index}>{index+1} · {name}</option>{/each}</select></label></div><div class="form-row"><button disabled={locked} on:click={()=>planLevel=training.profile?.preferredLevel??1}>Уровень из профиля</button><button disabled={locked} on:click={levelPlan}>Создать вариант</button></div><small>Создаёт редактируемую копию с 1–4 подходами на упражнение. Уровень здесь регулирует объём, не рабочий вес. Проверьте упражнения, повторы и отдых в конструкторе перед сохранением. История и исходная программа не изменяются.</small></div><div class="panel"><div class="section-heading"><h3>Программы</h3><button class="primary" disabled={locked} on:click={() => editPlan()}>+ Создать</button></div>{#each training.plans as plan}<div class="library-row"><div><strong>{plan.name}</strong><small>{plan.items.length} упражнений · {plan.items.reduce((sum,item) => sum+item.sets,0)} подходов</small></div><button disabled={locked} on:click={() => editPlan(plan)}>Изменить</button></div>{/each}</div>
        {#if editing}<form class="panel plan-editor" on:submit|preventDefault={savePlan}><h3>Конструктор программы</h3><label>Название<input bind:value={editing.name} maxlength="100" required/></label>{#each editing.items as item,index}<div class="plan-item"><strong>{training.exercises.find(exercise => exercise.id === item.exerciseId)?.name}</strong><div class="plan-fields"><label>Подходы<input type="number" min="1" max="20" bind:value={item.sets} required/></label><label>Повторы от<input type="number" min="1" max="500" bind:value={item.minReps} required/></label><label>до<input type="number" min="1" max="500" bind:value={item.maxReps} required/></label><label>Отдых, сек<input type="number" min="10" max="1800" bind:value={item.rest} required/></label><button type="button" aria-label="Убрать упражнение из программы" on:click={() => editing && (editing = {...editing,items:editing.items.filter((_,i)=>i!==index)})}>×</button></div></div>{/each}<div class="form-row"><select bind:value={addExerciseId} aria-label="Упражнение для программы">{#each training.exercises as exercise}<option value={exercise.id}>{exercise.name}</option>{/each}</select><button type="button" on:click={appendExercise}>Добавить</button></div><div class="form-row"><button class="primary" disabled={locked}>Сохранить программу</button><button type="button" on:click={() => editing = null}>Закрыть</button></div><small>Изменения программы не меняют уже начатую тренировку и её историю.</small></form>{/if}
    {:else if tab === "exercises"}<div class="panel"><div class="section-heading"><h3>Библиотека упражнений</h3><select bind:value={libraryFilter} aria-label="Мышечная группа"><option>Все</option>{#each groups as group}<option>{group}</option>{/each}</select></div>{#each training.exercises.filter(exercise=>libraryFilter === "Все" || exercise.muscle===libraryFilter) as exercise}<div class="library-row"><div><strong>{exercise.name}</strong><small>{exercise.muscle} · {exercise.equipment}</small></div></div>{/each}<form class="exercise-form" on:submit|preventDefault={addExercise}><h4>Своё упражнение</h4><label>Название<input bind:value={exerciseName} maxlength="100" required/></label><div class="form-row"><label>Группа мышц<input bind:value={muscle} maxlength="40" required/></label><label>Оборудование<input bind:value={equipment} maxlength="40" required/></label></div><button class="primary" disabled={locked}>Добавить в библиотеку</button></form></div>
    {:else if tab === "history"}<div class="panel"><h3>История</h3>{#if !history.length}<p class="empty">Пока нет тренировок. Начните первую — здесь появятся ваши результаты.</p>{/if}{#each history as item}<div class="library-row"><div><strong>{item.name}</strong><small>{date(item.startedAt)} · {sessionState(item.status)} · {workingSets(item).length} подходов · {trainingVolume(item)} кг</small></div><button on:click={() => { viewId = item.id === active?.id ? "" : item.id; exerciseIndex = 0; tab = "today" }}>Открыть</button></div>{/each}</div>
    {:else if tab === "stats"}<TrainingStats bind:selected={bodyGroup} data={training} disabled={locked} onSave={persist} onProfile={()=>tab="profile"} />
    {:else}<div class="panel"><p class="kicker">ЛИЧНЫЕ РЕЗУЛЬТАТЫ</p><h3>Рекорды</h3>{#if !records.length}<p class="empty">Рекорды появятся после первой завершённой тренировки.</p>{/if}{#each records as record}<div class="library-row"><div><strong>{record.exercise.name}</strong><small>Лучший вес: {record.best?.weight} кг × {record.best?.reps}<br/>Расчётный 1RM: {record.estimate ? `${record.estimate.toFixed(1)} кг` : "нет подходов 1–10 повторов"}</small></div></div>{/each}<small>1RM — оценка по Epley для подходов 1–10 повторов, а не проверенный максимум и не рекомендация поднять этот вес.</small></div>{/if}
</section>

<style>
    .daily-planner{margin-bottom:1rem;display:flex;flex-direction:column;gap:.6rem}.proposal{padding:.8rem;border:1px solid #376e71;border-radius:9px;background:#123237}.proposal ul{padding-left:1.2rem;margin:.5rem 0;font-size:.68rem;color:#b4cdd0;line-height:1.7}.daily-planner a{color:var(--accent)}
    .body-navigation{padding:1rem;margin:0 0 1rem;border:1px solid #28494f;border-radius:13px;background:linear-gradient(120deg,#123a3c66,#0b1b20);width:100%;box-sizing:border-box;min-width:0}
    .charge-panel{max-width:580px;margin:auto}.charge-list{display:flex;flex-direction:column;gap:.6rem;margin:1rem 0}.charge-list label{display:flex;flex-direction:row;align-items:center;gap:.7rem;padding:.8rem;border:1px solid var(--line);border-radius:10px;background:#10262c;cursor:pointer}.charge-list input{width:20px;height:20px;accent-color:var(--accent)}.charge-list label>span:last-child{display:flex;flex-direction:column;gap:.3rem}.charge-list strong{font-size:.8rem;color:#eaffff}.charge-complete{border-color:var(--accent)!important;background:#154044!important}.charge-panel .section-heading>strong{font-size:.8rem;color:var(--accent)}
    .training{--accent:#60f3e9;--muted:#91abb1;--line:#28494f;font-family:"Manrope Variable",sans-serif;color:#eaffff}header,.session-header,.section-heading{display:flex;align-items:center;justify-content:space-between;gap:.8rem;margin-bottom:1rem}.kicker{color:var(--accent);font-size:.6rem;letter-spacing:.14em;font-weight:800;margin:0 0 .5rem}h2{font-size:1.45rem;letter-spacing:-.03em;margin:0}h3{margin:0 0 .6rem;font-size:1rem}h4{margin:.9rem 0 .6rem;font-size:.75rem}.date,small{font-size:.62rem;color:var(--muted);line-height:1.6}.tabs{display:flex;flex-wrap:wrap;gap:.35rem;margin-bottom:1rem}.tabs button{font-size:.66rem}.tabs .active,.exercise-list .selected{background:linear-gradient(100deg,#15575c,#153139);border-color:var(--accent);color:white;box-shadow:0 0 16px #36d9ce15}.panel{border:1px solid var(--line);border-radius:12px;background:linear-gradient(140deg,#16343a66,transparent),#0c1b20;padding:.9rem;min-width:0}.intro{display:flex;flex-direction:column;gap:.8rem;padding:1.5rem}.intro p{font-size:.8rem;color:#a9c7ca;max-width:60ch}.training-grid{display:grid;grid-template-columns:minmax(145px,.8fr) minmax(200px,1.25fr) minmax(150px,.8fr);gap:.65rem;align-items:start}.right{display:flex;flex-direction:column;gap:.65rem}.session-actions{display:flex;flex-wrap:wrap;gap:.3rem}.session-actions button{font-size:.62rem}.progress{height:6px;border-radius:8px;background:#20383e;overflow:hidden;margin:.9rem 0 .4rem}.progress span{display:block;height:100%;background:var(--accent)}.warmups{padding-bottom:.8rem;border-bottom:1px solid var(--line)}.warmups label{flex-direction:row;align-items:flex-start;gap:.3rem;font-size:.6rem;margin:.35rem 0}.warmups input{width:auto;accent-color:var(--accent)}.exercise-list{display:flex;flex-direction:column;gap:.45rem;margin-top:.8rem}.exercise-list button{display:flex;gap:.45rem;padding:.55rem;text-align:left}.exercise-list button>span:last-child{display:flex;flex-direction:column;gap:.2rem}.exercise-list strong{font-size:.66rem;font-weight:700}.number{display:grid;place-items:center;border-radius:50%;width:22px;height:22px;background:#1d4147;color:var(--accent);flex:none;font-size:.7rem}.current h3{font-size:1.25rem;line-height:1.3;letter-spacing:-.025em;margin:.6rem 0}.tags{display:flex;gap:.35rem;flex-wrap:wrap}.tags span{font-size:.6rem;padding:.3rem .5rem;background:#17373e;border-radius:5px;color:#b9dfdf}.context{display:flex;gap:.8rem;margin:.9rem 0;padding:.7rem;border:1px solid var(--line);border-radius:8px}.context div{display:flex;flex-direction:column;gap:.2rem;flex:1}.context strong{font-size:.85rem}.previous{border-left:2px solid var(--accent);padding-left:.6rem;margin-bottom:.9rem}.previous p,.analysis p{font-size:.67rem;line-height:1.7;color:#b7d5d8;margin:.35rem 0}.sets{display:flex;flex-direction:column;gap:.35rem}.set{display:flex;align-items:center;gap:.45rem;padding:.55rem;background:#10292f;border:1px solid var(--line);border-radius:6px}.set>span{color:var(--accent);font-size:.65rem}.set strong{font-size:.72rem}.set small{margin-left:auto;font-size:.55rem}.set button{border:0;background:none;padding:0}.set-form{margin-top:1rem;display:flex;flex-direction:column;gap:.7rem}.inputs{display:flex;gap:.5rem}.inputs label{flex:1;min-width:0}.stepper{display:flex;gap:.1rem}.stepper input{width:100%;min-width:0;text-align:center;padding:.4rem .1rem}.stepper button{padding:.4rem}.checkbox{flex-direction:row;align-items:center;font-size:.58rem}.checkbox input{width:auto;accent-color:var(--accent)}.exercise-nav{display:flex;justify-content:space-between;gap:.4rem;margin-top:.8rem}.exercise-nav button{font-size:.6rem}.rest{text-align:center}.rest .kicker{text-align:left}.rest-dial{margin:.7rem auto;max-width:165px;aspect-ratio:1;border-radius:50%;padding:8px;background:conic-gradient(var(--accent) var(--progress),#203b41 0)}.rest-dial>div{display:flex;flex-direction:column;justify-content:center;align-items:center;height:100%;border-radius:50%;background:#0c1b20;gap:.4rem}.rest time{font-size:2rem;font-weight:800;font-variant-numeric:tabular-nums}.rest-actions{display:flex;justify-content:center;gap:.3rem;margin-bottom:.6rem}.rest-actions button{font-size:.6rem}.metrics{display:flex;flex-wrap:wrap;gap:.7rem;margin:.8rem 0}.metrics>div{display:flex;flex-direction:column;gap:.3rem;flex:1;min-width:70px}.metrics strong{color:var(--accent);font-size:1.1rem;font-variant-numeric:tabular-nums}.metrics small{font-size:.56rem}.analysis label{margin:.7rem 0}.analysis textarea{min-height:60px}.analysis button{font-size:.6rem}.library-row{display:flex;align-items:center;justify-content:space-between;gap:.7rem;border-bottom:1px solid var(--line);padding:.75rem 0}.library-row>div{display:flex;flex-direction:column;gap:.3rem}.library-row strong{font-size:.8rem}.library-row button{flex:none}.exercise-form,.plan-editor{display:flex;flex-direction:column;gap:.7rem;margin-top:1rem}.plan-item{padding:.7rem;border:1px solid var(--line);border-radius:8px}.plan-item strong{font-size:.75rem}.plan-fields{display:flex;gap:.35rem;margin-top:.5rem;align-items:end}.plan-fields label{flex:1;min-width:0}.form-row{display:flex;gap:.5rem}.form-row>*{flex:1;min-width:0}.muscle-row{display:flex;align-items:center;gap:.7rem;font-size:.7rem;margin:.75rem 0}.muscle-row>span{width:85px}.muscle-row>div{flex:1;height:8px;background:#20383e;border-radius:5px;overflow:hidden}.muscle-row i{display:block;height:100%;background:var(--accent);border-radius:5px}.muscle-row b{width:25px;text-align:right}.error{padding:.7rem;background:#351c21;border:1px solid #805157;border-radius:7px;font-size:.75rem;color:#ffd0d0}.empty{font-size:.72rem;color:var(--muted);line-height:1.6}label{display:flex;flex-direction:column;gap:.3rem;color:var(--muted);font-size:.65rem}button,input,select,textarea{border:1px solid var(--line);border-radius:7px;background:#10262c;color:#eaffff;font:600 .7rem "Manrope Variable",sans-serif;padding:.5rem;min-width:0;box-sizing:border-box}input,textarea,select{width:100%;color-scheme:dark}button{cursor:pointer}button:hover:not(:disabled){border-color:var(--accent)}button:disabled{opacity:.4;cursor:default}.primary{background:linear-gradient(#21cbc4,#0a939a);border-color:var(--accent);color:#041e23;min-height:39px;font-weight:800}button:focus-visible,input:focus-visible,select:focus-visible,textarea:focus-visible{outline:2px solid var(--accent);outline-offset:2px}@media(max-width:1150px){.training-grid{grid-template-columns:minmax(150px,.8fr) minmax(210px,1.2fr)}.right{grid-column:1/-1;display:grid;grid-template-columns:repeat(3,minmax(0,1fr))}.session-header{align-items:flex-start;flex-direction:column}}@media(max-width:720px){.training-grid{grid-template-columns:1fr}.right{grid-template-columns:1fr}.exercise-list{display:grid;grid-template-columns:repeat(2,minmax(0,1fr))}.date{display:none}.plan-fields{flex-wrap:wrap}.plan-fields label{min-width:65px}.form-row{flex-wrap:wrap}}
    .input-group{flex:1;min-width:0}.input-group>label{margin-bottom:.3rem}
    .session-context{display:flex;align-items:center;gap:.65rem;flex-wrap:wrap;margin-bottom:.8rem;padding:.7rem;border:1px solid var(--line);border-radius:8px;background:#10262c}.session-context>span{font-size:.62rem;color:#aecbd0}.session-context label{margin-left:auto;font-size:.6rem}.session-context button{font-size:.6rem}
    .charge-panel{max-width:none;width:100%;box-sizing:border-box;margin:0 0 1rem}.charge-list{display:grid;grid-template-columns:repeat(auto-fit,minmax(min(100%,320px),1fr));align-items:stretch}.charge-list label{margin:0}
</style>

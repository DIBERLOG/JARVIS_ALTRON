import { invoke } from '@tauri-apps/api/core'

export const agentScenarios = [
    {id:'document_mail', title:'Документ → письмо → проверка', example:'Подготовь деловое письмо по требованиям из документа. Отметь, что нужно уточнить.', mail:true, steps:['Аналитик документа','Почтовый помощник','Проверяющий письмо']},
    {id:'reply_mail', title:'Письмо → ответ → проверка', example:'Подготовь вежливый ответ на письмо из контекста. Не пропускай вопросы отправителя.', mail:true, steps:['Аналитик вопросов','Автор ответа','Проверяющий полноту ответа']},
    {id:'meeting_plan', title:'Встреча → протокол и план', example:'Подготовь краткий протокол встречи и план действий. Неизвестные сроки и ответственных отметь для уточнения.', mail:false, steps:['Аналитик встречи','Планировщик поручений','Проверяющий сроки и исполнителей']},
    {id:'supplier_compare', title:'Сравнение поставщиков', example:'Сравни предложения из документов по цене, НДС, доставке, оплате и срокам. Покажи различия и недостающие данные.', mail:false, steps:['Аналитик предложений','Составитель сравнения','Проверяющий цены и условия']},
    {id:'weekly_report', title:'Недельный отчёт и письмо', example:'Составь недельный отчёт по заметкам и выполненным задачам, затем короткое письмо руководителю.', mail:true, steps:['Аналитик результатов недели','Автор отчёта и письма','Проверяющий факты отчёта']},
    {id:'document_check', title:'Проверка документа перед отправкой', example:'Проверь документ на неясности, противоречия в датах и суммах, персональные и конфиденциальные данные.', mail:false, steps:['Редактор ясности','Проверяющий даты и суммы','Проверяющий приватность']},
] as const

export type ChainReply = {content:string; subject:string|null; body:string|null; review:string|null; report:string|null; provider:string; model:string}
export type ChainStep = {title:string; status:'waiting'|'running'|'done'|'error'|'stopped'; result?:ChainReply; error?:string}
export type ChainInput = {scenario:string; task:string; context:string}

export async function runAgentChain(input:ChainInput, stopped:()=>boolean, progress:(index:number, step:ChainStep)=>void):Promise<ChainReply|null> {
    const scenario = agentScenarios.find(item=>item.id===input.scenario)
    if (!scenario || !input.task.trim() || !input.context.trim()) throw Error('Выберите сценарий и добавьте поручение с исходными данными.')
    const previous:string[]=[]
    let final:ChainReply|null=null
    for (let index=0;index<scenario.steps.length;index++) {
        if (stopped()) return null
        const title=scenario.steps[index]
        progress(index,{title,status:'running'})
        try {
            const result=await invoke<ChainReply>('agent_chain_step',{...input,step:index,previous:[...previous]})
            if (stopped()) { progress(index,{title,status:'stopped'}); return null }
            if (!result.content?.trim() || Array.from(result.content).length>8000) throw Error('Саб-агент вернул некорректный результат.')
            if (scenario.mail && index>0 && (!result.subject?.trim() || !result.body?.trim())) throw Error('Саб-агент не вернул корректное письмо.')
            if (scenario.mail && index===2 && !result.review?.trim()) throw Error('Проверяющий не вернул замечания.')
            if (scenario.id==='weekly_report' && index>0 && !result.report?.trim()) throw Error('Не получен недельный отчёт.')
            progress(index,{title,status:'done',result})
            previous.push(result.content)
            final=result
        } catch(error) {
            if (stopped()) { progress(index,{title,status:'stopped'}); return null }
            progress(index,{title,status:'error',error:String(error)})
            throw error
        }
    }
    return final
}

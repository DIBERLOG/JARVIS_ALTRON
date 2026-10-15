<script lang="ts">
    import { dayKey } from '@/lib/center'
    export let value=''
    export let daily=false
    $: datePart=value.split('T')[0]||''
    $: timePart=value.split('T')[1]?.slice(0,5)||'09:00'
    $: past=value && new Date(value).getTime()<=Date.now()
    function setParts(date:string,time:string){value=`${date}T${time}`}
    function quick(days:number,hours?:number){const d=new Date();if(hours)d.setHours(d.getHours()+hours);else{d.setDate(d.getDate()+days);d.setHours(9,0,0,0)}value=`${dayKey(d)}T${String(d.getHours()).padStart(2,'0')}:${String(d.getMinutes()).padStart(2,'0')}`}
</script>
<div class="schedule">
    <div class="fields"><label>{daily?"НАЧИНАЯ С":"ДАТА"}<input aria-label="Дата напоминания" type="date" value={datePart} on:input={e=>setParts(e.currentTarget.value,timePart)} required/></label><label>ВРЕМЯ<input aria-label="Время напоминания" type="time" value={timePart} on:input={e=>setParts(datePart,e.currentTarget.value)} required/></label></div>
    <div class="quick" role="group" aria-label="Быстрый выбор даты"><button type="button" on:click={()=>quick(0)}>Сегодня</button><button type="button" on:click={()=>quick(1)}>Завтра</button><button type="button" on:click={()=>quick(2)}>Послезавтра</button><button type="button" on:click={()=>quick(0,1)}>Через час</button></div>
    <p class:past={past&&!daily}>{daily?'Каждый день в выбранное местное время. Прошедшие дни не воспроизводятся.':past?'Прошлая дата — сохранится как запись без звукового уведомления.':'В местном времени компьютера. Можно выбрать любую дату, в том числе прошлую.'}</p>
</div>
<style>
    .schedule{padding:14px;border:1px solid #31545a;border-radius:12px;background:#091b20}.fields{display:grid;grid-template-columns:minmax(0,1.6fr) minmax(100px,1fr);gap:12px}label{display:grid;gap:7px;font-size:10px;letter-spacing:.12em;color:#9ec6c8;font-weight:800}input{min-width:0;width:100%;box-sizing:border-box;padding:11px 12px;border:1px solid #3d6268;border-radius:9px;background:#10272d;color:#e5ffff;color-scheme:dark;font:600 14px 'Manrope Variable',sans-serif}.quick{display:flex;flex-wrap:wrap;gap:7px;margin-top:12px}button{padding:7px 11px;border:1px solid #31575b;border-radius:20px;background:#133137;color:#ade4df;font:600 11px 'Manrope Variable',sans-serif;cursor:pointer}button:hover{border-color:#60f3e9;background:#1b454a}p{margin:10px 0 0;font-size:11px;line-height:1.6;color:#85a9af}p.past{color:#dfc784}input:focus-visible,button:focus-visible{outline:2px solid #60f3e9;outline-offset:2px}
</style>

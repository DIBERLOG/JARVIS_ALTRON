<script lang="ts">
    import {onMount} from 'svelte'
    import {invoke} from '@tauri-apps/api/core'
    import QRCode from 'qrcode'
    let ips:string[]=[],ip='',port=47832,error='',busy=false,qr=''
    let status:any={running:false,pending:[],devices:[],conflicts:[],allowVault:false}
    async function refresh(){try{status=await invoke('lan_sync_status');qr=status.qr?await QRCode.toDataURL(status.qr,{width:280,margin:2,errorCorrectionLevel:'M'}):''}catch(e){error=String(e)}}
    async function action(command:string,args:any={}){busy=true;error='';try{await invoke(command,args);await refresh()}catch(e){error=String(e)}finally{busy=false}}
    async function vaultChange(event:Event){const input=event.target as HTMLInputElement,enabled=input.checked;if(enabled&&!confirm('Разрешить обмен зашифрованным сейфом с подтверждёнными устройствами? Сначала сделайте резервную копию. Мастер-пароль по сети не передаётся.')){input.checked=false;await refresh();return}await action('lan_sync_vault_enabled',{enabled});input.checked=status.allowVault}
    onMount(()=>{invoke<string[]>('lan_sync_addresses').then(v=>{ips=v;ip=v[0]||''}).catch(e=>error=String(e));refresh();const timer=setInterval(refresh,2500);return()=>clearInterval(timer)})
</script>
<section aria-label="Синхронизация с телефоном" class="sync">
    <p class="eyebrow">JARVIS / LOCAL LINK · EXPERIMENTAL</p>
    <h2>Компьютер и телефон — рядом.</h2>
    <p>Обмен по локальной Wi-Fi сети, без облака. Подключите Android по QR-коду и сравните код на обоих устройствах. Ключи API, настройки моделей и таймеры не передаются.</p>
    {#if error}<p role="alert" class="error">{error}</p>{/if}
    {#if !status.running}
        <label>Локальный IPv4 компьютера<select bind:value={ip}>{#each ips as address}<option value={address}>{address}</option>{/each}</select></label>
        <label>Порт<input type="number" min="1024" max="65535" bind:value={port}></label>
        <button class="primary" disabled={busy||!ip} on:click={()=>action('lan_sync_start',{ip,port})}>Включить обмен по Wi-Fi</button>
        {#if !ips.length}<p>Локальная сеть не найдена. Подключите компьютер и телефон к одному роутеру, затем откройте раздел снова.</p>{/if}
    {:else}
        <p class="connected">Обмен включён: {status.address}</p>
        {#if qr}<img src={qr} width="280" height="280" alt="QR-код сопряжения телефона"><p>Код действует 2 минуты. Не публикуйте его и не отправляйте посторонним.</p>{:else}<p>QR-код истёк. Уже подключённые устройства продолжают работать.</p>{/if}
        <button disabled={busy} on:click={()=>action('lan_sync_qr')}>Новый QR-код</button>
        <button disabled={busy} on:click={()=>action('lan_sync_stop')}>Остановить обмен</button>
    {/if}
    {#each status.pending as device}<article><h3>Подтвердить: {device.name}</h3><p>Код на телефоне должен совпадать: <strong>{device.code}</strong></p><button disabled={busy} on:click={()=>action('lan_sync_approve',{id:device.id})}>Код совпадает — разрешить</button></article>{/each}
    <label class="check"><input type="checkbox" checked={status.allowVault} on:change={vaultChange}>Разрешить синхронизацию зашифрованного сейфа</label>
    <p>Разблокировка сейфа выполняется на устройстве. При конфликте обе зашифрованные версии сохраняются; объединение выполняется на телефоне после ввода мастер-пароля.</p>
    <h3>Подключённые устройства</h3>
    {#if !status.devices.length}<p>Устройств пока нет.</p>{/if}
    {#each status.devices as device}<article><strong>{device.name}</strong><p>Последний обмен: {device.lastSync?new Date(device.lastSync).toLocaleString():'ещё не выполнялся'}</p><button disabled={busy} on:click={()=>{if(confirm('Отключить устройство? Это не удалит уже сохранённую на нём копию данных.'))action('lan_sync_revoke',{id:device.id})}}>Отключить устройство</button></article>{/each}
    <h3>Конфликты данных</h3>
    <p>Обе версии сохраняются до выбора. Восстановление с телефона может изменить запись или удалить её, если она удалена на телефоне.</p>
    {#if !status.conflicts.length}<p>Конфликтов нет.</p>{/if}
    {#each status.conflicts as conflict}<article><strong>{conflict.collection}: {conflict.title}</strong><details><summary>Сравнить обе версии</summary><p>Компьютер</p><pre>{conflict.local===null?'Запись удалена':JSON.stringify(conflict.local,null,2)}</pre><p>Телефон</p><pre>{conflict.phone===null?'Запись удалена':JSON.stringify(conflict.phone,null,2)}</pre></details><button disabled={busy} on:click={()=>action('lan_sync_resolve',{id:conflict.id,usePhone:false})}>Оставить версию компьютера</button><button disabled={busy} on:click={()=>{if(confirm('Применить версию телефона? Резервная копия текущих данных будет сохранена.'))action('lan_sync_resolve',{id:conflict.id,usePhone:true})}}>Применить версию телефона</button></article>{/each}
    <p class="footnote">Компьютерный JARVIS должен оставаться запущенным. Если соединение не устанавливается, разрешите приложению входящие подключения только в частной сети. Резервные копии находятся в папке данных приложения / lan-sync / backups. Автозапуск сервера после перезапуска не включён.</p>
</section>
<style>
 .sync{font-family:'Manrope Variable',sans-serif;color:#eaf7f8;padding:24px;border:1px solid #29464d;border-radius:16px;background:linear-gradient(135deg,#12383c,#0b1b20)}.eyebrow{color:#5eeade;font-size:11px;font-weight:800;letter-spacing:.15em}h2{font-size:25px;letter-spacing:-.04em;margin:8px 0 18px}h3{font-size:16px;margin-top:24px}p{font-size:13px;line-height:1.7;color:#a4c1c7}label{display:flex;flex-direction:column;gap:8px;margin:16px 0;font-size:13px}.check{flex-direction:row;align-items:center}input,select,button{font:inherit;border:1px solid #36565c;border-radius:9px;background:#0b2228;color:#eaf7f8;padding:12px}button{cursor:pointer;margin:8px 8px 8px 0;font-size:12px}button:disabled{opacity:.45;cursor:default}.primary{background:#37d2c9;color:#05272d;font-weight:800}input:focus-visible,select:focus-visible,button:focus-visible{outline:2px solid #5eeade;outline-offset:3px}img{display:block;border-radius:14px;max-width:100%;height:auto;margin-top:16px}article{padding:16px;border:1px solid #36565c;border-radius:12px;margin-top:14px}.error{color:#ffb5be;white-space:pre-wrap}.connected{color:#77f2da}.footnote{font-size:11px}
</style>

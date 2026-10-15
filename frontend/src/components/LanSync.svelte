<script lang="ts">
    import {onMount} from 'svelte'
    import {invoke} from '@tauri-apps/api/core'
    import QRCode from 'qrcode'
    import {save,open} from '@tauri-apps/plugin-dialog'
    import {writeTextFile,readTextFile} from '@tauri-apps/plugin-fs'
    let ips:string[]=[],ip='',port=47832,error='',busy=false,qr='',password='',peerQr='',pairCode='',message='',shareChats=false
    let status:any={running:false,pending:[],devices:[],conflicts:[],allowVault:false,prepared:false,v2Conflicts:[],client:false}
    let pairingVisible=true,initialized=false,addressesLoading=true,qrSource='',refreshTask:Promise<void>|null=null
    function refresh():Promise<void>{if(refreshTask)return refreshTask;refreshTask=(async()=>{try{const next:any=await invoke('lan_sync_status');const source=next.qr||'';if(source!==qrSource){qr=source?await QRCode.toDataURL(source,{width:280,margin:2,errorCorrectionLevel:'M'}):'';qrSource=source}status=next;shareChats=!!status.shareChats;if(!initialized){pairingVisible=!status.devices.length;initialized=true}if(status.pending.length)pairingVisible=true}catch(e){error=String(e)}})().finally(()=>refreshTask=null);return refreshTask}
    async function copyPairing(){error='';try{await navigator.clipboard.writeText(status.qr);message='Строка сопряжения скопирована. Вставьте её в JARVIS на телефоне или втором ПК.'}catch(e){error='Не удалось скопировать автоматически. Выделите и скопируйте строку в поле ниже.'}}
    async function action(command:string,args:any={}){busy=true;error='';try{await refresh();await invoke(command,args);await refresh();if(command==='lan_sync_approve'){pairingVisible=false;message='Разрешение выдано. Нажмите «Подтвердить сопряжение» на телефоне.'}else if(command==='lan_sync_qr'||command==='lan_sync_start')pairingVisible=true}catch(e){error=String(e)}finally{busy=false}}
    async function vaultChange(event:Event){const input=event.target as HTMLInputElement,enabled=input.checked;if(enabled&&!confirm('Разрешить обмен зашифрованным сейфом с подтверждёнными устройствами? Сначала сделайте резервную копию. Мастер-пароль по сети не передаётся.')){input.checked=false;await refresh();return}await action('lan_sync_vault_enabled',{enabled});input.checked=status.allowVault}
    function parentIds(conflict:any):string[]{return conflict.versions.map((v:any)=>v.op_id)}
    async function prepare(){busy=true;error='';try{message='Копия восстановления: '+await invoke('sync_v2_prepare',{password});password='';await refresh()}catch(e){error=String(e)}finally{busy=false}}
    async function pairBegin(){busy=true;error='';try{const result:any=await invoke('sync_v2_pair_begin',{qr:peerQr,name:'Второй компьютер'});pairCode=result.code;await refresh()}catch(e){error=String(e)}finally{busy=false}}
    async function pairFinish(){busy=true;error='';try{const result:any=await invoke('sync_v2_pair_finish',{password});if(result.pending)message='Подтверждение ещё ожидается на хабе';else{message='Подключено. Копия восстановления: '+result.recovery;password='';peerQr='';pairCode=''}await refresh()}catch(e){error=String(e)}finally{busy=false}}
    async function recoveryExport(){busy=true;error='';try{const path=await save({defaultPath:'JARVIS-Sync-v2-recovery.json',filters:[{name:'Защищённая копия',extensions:['json']}]});if(path){const data:string=await invoke('sync_v2_export',{password});await writeTextFile(path,data);message='Копия сохранена: '+path;password=''}}catch(e){error=String(e)}finally{busy=false}}
    async function recoveryRestore(){busy=true;error='';try{const path=await open({multiple:false,filters:[{name:'Защищённая копия',extensions:['json']}]});if(typeof path==='string'){const backup=await readTextFile(path);await invoke('sync_v2_restore',{backup,password});password='';message='Восстановлено в новом профиле. Перезапустите LAB.';await refresh()}}catch(e){error=String(e)}finally{busy=false}}
    onMount(()=>{invoke<string[]>('lan_sync_addresses').then(v=>{ips=v;ip=v[0]||''}).catch(e=>error=String(e)).finally(()=>addressesLoading=false);refresh();const timer=setInterval(()=>{if(!busy)refresh()},2500);return()=>clearInterval(timer)})
</script>
<section aria-label="Синхронизация с телефоном" class="sync">
    <p class="eyebrow">JARVIS / УСТРОЙСТВА</p>
    <h2>Синхронизация устройств</h2>
    <p>Ваши записи на компьютере и телефоне. Обмен работает в одной домашней сети.</p>
    <details class="guide">
        <summary>Как подключить устройство</summary>
        <ol>
            <li>Подключите телефон и компьютер к одной Wi-Fi сети.</li>
            <li>Включите обмен и отсканируйте QR-код с телефона.</li>
            <li>Сравните код. Разрешите подключение на ПК, затем нажмите «Подтвердить сопряжение» на телефоне.</li>
            <li>На телефоне нажмите «Синхронизировать сейчас» и выберите направление.</li>
        </ol>
        <p>Для обмена оставьте JARVIS на компьютере открытым. Чаты и сейф включаются отдельно.</p>
    </details>
    {#if error}<p role="alert" class="error">{error}</p>{/if}
    {#if busy}<p role="status">Выполняем действие…</p>{/if}
    {#if message}<p role="status">{message}</p>{/if}
    <label>Пароль копии восстановления (от 6 символов)<input type="password" bind:value={password} autocomplete="new-password"></label>
    {#if !status.prepared}
        <p>Для начала сохраните резервную копию и настройте подключение.</p>
        <button disabled={busy||password.length<6} on:click={prepare}>Настроить синхронизацию и сохранить копию</button>
        <label>Строка QR другого компьютера<input bind:value={peerQr} autocomplete="off"></label>
        <button disabled={busy||!peerQr} on:click={pairBegin}>Запросить сопряжение второго ПК</button>
        {#if pairCode}<p>Сравните код на хабе: <strong>{pairCode}</strong></p><button disabled={busy||password.length<6} on:click={pairFinish}>Подтверждено на хабе — завершить</button>{/if}
        <button disabled={busy||!password} on:click={recoveryRestore}>Восстановить копию в пустом профиле</button>
    {:else}
        <p>Сейф защищён отдельным мастер-паролем. Пароль резервной копии нужен для восстановления данных.</p>
        <button disabled={busy||password.length<6} on:click={recoveryExport}>Сохранить новую копию восстановления</button>
        {#if status.client}<label class="check"><input type="checkbox" checked={status.auto} on:change={e=>action('sync_v2_auto',{enabled:e.currentTarget.checked})}>Автообмен с хабом каждые 30 секунд</label>{#if status.background?.error}<p role="alert">{status.background.error}</p>{/if}<button class="primary" disabled={busy} on:click={()=>action('sync_v2_run')}>Синхронизировать с хабом</button><button disabled={busy} on:click={()=>action('sync_v2_disconnect')}>Отключить хаб, сохранить данные</button>{/if}
        <label class="check"><input type="checkbox" bind:checked={shareChats} on:change={()=>action('sync_v2_sharing',{chats:shareChats,vault:status.allowVault})}>Передавать историю чатов</label>
    {/if}
    {#if status.prepared && status.restored && !status.client}
        <p>Восстановленный профиль можно использовать как новый хаб или повторно подключить к прежнему.</p>
        <label>QR прежнего хаба<input bind:value={peerQr} autocomplete="off"></label><button disabled={busy||!peerQr} on:click={pairBegin}>Запросить новое сопряжение</button>
        {#if pairCode}<p>Код: <strong>{pairCode}</strong></p><button disabled={busy||password.length<6} on:click={pairFinish}>Завершить сопряжение и сохранить копию</button>{/if}
    {/if}
    {#if status.prepared && !status.client}
    {#if !status.running}
        <label>Домашняя сеть<select bind:value={ip}>{#each ips as address}<option value={address}>{address}</option>{/each}</select></label>
        <label>Порт<input type="number" min="1024" max="65535" bind:value={port}></label>
        <button class="primary" disabled={busy||!ip} on:click={()=>action('lan_sync_start',{ip,port})}>Включить обмен по Wi-Fi</button>
        {#if addressesLoading}<p role="status">Ищем домашнюю сеть…</p>{:else if !ips.length}<p>Домашняя сеть не найдена. Подключите компьютер к Wi-Fi или роутеру и откройте раздел снова.</p>{/if}
    {:else}
        <p class="connected">Обмен включён: {status.address}</p>
        {#if pairingVisible}
        {#if qr}<img src={qr} width="280" height="280" alt="QR-код сопряжения телефона"><label>Строка сопряжения для телефона или второго ПК<textarea readonly value={status.qr} rows="4" spellcheck="false"></textarea></label><button disabled={busy} on:click={copyPairing}>Скопировать строку сопряжения</button><p>Код действует 2 минуты. Не публикуйте его и не отправляйте посторонним.</p>{:else}<p>Срок QR-кода истёк. Создайте новый для подключения устройства.</p>{/if}
        {/if}
        <button disabled={busy} on:click={()=>action('lan_sync_qr')}>{pairingVisible?'Новый QR-код':'Подключить ещё устройство'}</button>
        <button disabled={busy} on:click={()=>action('lan_sync_stop')}>Остановить обмен</button>
    {/if}
    {/if}
    {#each status.pending as device}<article><h3>Подтвердить: {device.name}</h3><p>Код на телефоне должен совпадать: <strong>{device.code}</strong></p><button disabled={busy} on:click={()=>action('lan_sync_approve',{id:device.id})}>Код совпадает — разрешить</button></article>{/each}
    <label class="check"><input type="checkbox" checked={status.allowVault} on:change={vaultChange}>Разрешить синхронизацию зашифрованного сейфа</label>
    <p>Разблокировка сейфа выполняется на устройстве. При конфликте обе зашифрованные версии сохраняются; объединение выполняется на телефоне после ввода мастер-пароля.</p>
    <h3>Подключённые устройства</h3>
    {#if !status.devices.length}<p>Устройств пока нет.</p>{/if}
    {#each status.devices as device}<article><strong>{device.name}</strong><p>Последний обмен: {device.lastSync?new Date(device.lastSync).toLocaleString():'ещё не выполнялся'}</p><button disabled={busy} on:click={()=>{if(confirm('Отключить устройство? Это не удалит уже сохранённую на нём копию данных.'))action('lan_sync_revoke',{id:device.id})}}>Отключить устройство</button></article>{/each}
    <h3>Конфликты данных</h3>
    <p>Все версии сохранены. При конфликте удаления запись остаётся удалённой до вашего выбора.</p>
    {#if !status.v2Conflicts?.length}<p>Конфликтов нет.</p>{/if}
    {#each status.v2Conflicts||[] as conflict}<article><strong>{conflict.collection}: {conflict.entity}</strong>
        {#each conflict.versions as version}<details><summary>{version.deleted?'Удаление':'Версия '+version.op_id.slice(0,8)}</summary><pre>{JSON.stringify(version.value,null,2)}</pre></details><button disabled={busy} on:click={()=>action('sync_v2_resolve',{collection:conflict.collection,entity:conflict.entity,parents:parentIds(conflict),value:version.deleted?null:version.value})}>Выбрать эту версию</button>{/each}
    </article>{/each}
    <p class="footnote">Для синхронизации оставьте JARVIS открытым на обоих устройствах. После перезапуска ПК включите обмен снова.</p>
</section>
<style>
 .guide{border:1px solid #36565c;border-radius:12px;padding:16px;margin:20px 0;background:#0b2228}.guide summary{cursor:pointer;font-weight:700}.guide ol{padding-left:22px;font-size:13px;line-height:1.7;color:#d0e4e8}.guide li{margin:10px 0}
 .sync{font-family:'Manrope Variable',sans-serif;color:#eaf7f8;padding:24px;border:1px solid #29464d;border-radius:16px;background:linear-gradient(135deg,#12383c,#0b1b20)}.eyebrow{color:#5eeade;font-size:11px;font-weight:800;letter-spacing:.15em}h2{font-size:25px;letter-spacing:-.04em;margin:8px 0 18px}h3{font-size:16px;margin-top:24px}p{font-size:13px;line-height:1.7;color:#a4c1c7}label{display:flex;flex-direction:column;gap:8px;margin:16px 0;font-size:13px}.check{flex-direction:row;align-items:center}input,textarea,select,button{font:inherit;border:1px solid #36565c;border-radius:9px;background:#0b2228;color:#eaf7f8;padding:12px}button{cursor:pointer;margin:8px 8px 8px 0;font-size:12px}button:disabled{opacity:.45;cursor:default}.primary{background:#37d2c9;color:#05272d;font-weight:800}input:focus-visible,select:focus-visible,button:focus-visible{outline:2px solid #5eeade;outline-offset:3px}img{display:block;border-radius:14px;max-width:100%;height:auto;margin-top:16px}article{padding:16px;border:1px solid #36565c;border-radius:12px;margin-top:14px}.error{color:#ffb5be;white-space:pre-wrap}.connected{color:#77f2da}.footnote{font-size:11px}
</style>

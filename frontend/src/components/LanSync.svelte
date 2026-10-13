<script lang="ts">
    import {onMount} from 'svelte'
    import {invoke} from '@tauri-apps/api/core'
    import QRCode from 'qrcode'
    import {save,open} from '@tauri-apps/plugin-dialog'
    import {writeTextFile,readTextFile} from '@tauri-apps/plugin-fs'
    let ips:string[]=[],ip='',port=47832,error='',busy=false,qr='',password='',peerQr='',pairCode='',message='',shareChats=false
    let status:any={running:false,pending:[],devices:[],conflicts:[],allowVault:false,prepared:false,v2Conflicts:[],client:false}
    async function refresh(){try{status=await invoke('lan_sync_status');shareChats=!!status.shareChats;qr=status.qr?await QRCode.toDataURL(status.qr,{width:280,margin:2,errorCorrectionLevel:'M'}):''}catch(e){error=String(e)}}
    async function copyPairing(){error='';try{await navigator.clipboard.writeText(status.qr);message='Строка сопряжения скопирована. Вставьте её в JARVIS на телефоне или втором ПК.'}catch(e){error='Не удалось скопировать автоматически. Выделите и скопируйте строку в поле ниже.'}}
    async function action(command:string,args:any={}){busy=true;error='';try{await invoke(command,args);await refresh()}catch(e){error=String(e)}finally{busy=false}}
    async function vaultChange(event:Event){const input=event.target as HTMLInputElement,enabled=input.checked;if(enabled&&!confirm('Разрешить обмен зашифрованным сейфом с подтверждёнными устройствами? Сначала сделайте резервную копию. Мастер-пароль по сети не передаётся.')){input.checked=false;await refresh();return}await action('lan_sync_vault_enabled',{enabled});input.checked=status.allowVault}
    function parentIds(conflict:any):string[]{return conflict.versions.map((v:any)=>v.op_id)}
    async function prepare(){busy=true;error='';try{message='Копия восстановления: '+await invoke('sync_v2_prepare',{password});password='';await refresh()}catch(e){error=String(e)}finally{busy=false}}
    async function pairBegin(){busy=true;error='';try{const result:any=await invoke('sync_v2_pair_begin',{qr:peerQr,name:'Второй компьютер'});pairCode=result.code;await refresh()}catch(e){error=String(e)}finally{busy=false}}
    async function pairFinish(){busy=true;error='';try{const result:any=await invoke('sync_v2_pair_finish',{password});if(result.pending)message='Подтверждение ещё ожидается на хабе';else{message='Подключено. Копия восстановления: '+result.recovery;password='';peerQr='';pairCode=''}await refresh()}catch(e){error=String(e)}finally{busy=false}}
    async function recoveryExport(){busy=true;error='';try{const path=await save({defaultPath:'JARVIS-Sync-v2-recovery.json',filters:[{name:'Защищённая копия',extensions:['json']}]});if(path){const data:string=await invoke('sync_v2_export',{password});await writeTextFile(path,data);message='Копия сохранена: '+path;password=''}}catch(e){error=String(e)}finally{busy=false}}
    async function recoveryRestore(){busy=true;error='';try{const path=await open({multiple:false,filters:[{name:'Защищённая копия',extensions:['json']}]});if(typeof path==='string'){const backup=await readTextFile(path);await invoke('sync_v2_restore',{backup,password});password='';message='Восстановлено в новом профиле. Перезапустите LAB.';await refresh()}}catch(e){error=String(e)}finally{busy=false}}
    onMount(()=>{invoke<string[]>('lan_sync_addresses').then(v=>{ips=v;ip=v[0]||''}).catch(e=>error=String(e));refresh();const timer=setInterval(refresh,2500);return()=>clearInterval(timer)})
</script>
<section aria-label="Синхронизация с телефоном" class="sync">
    <p class="eyebrow">JARVIS / LOCAL LINK · EXPERIMENTAL</p>
    <h2>ПК ↔ ПК ↔ Android · Sync v2 LAB</h2>
    <p>Обмен по локальной Wi-Fi сети, без облака. Подключите Android по QR-коду и сравните код на обоих устройствах. Ключи API, настройки моделей и таймеры не передаются.</p>
    <details class="guide" open>
        <summary>Как подключить телефон или второй компьютер</summary>
        <ol>
            <li>Подключите устройства к одной Wi-Fi сети и откройте JARVIS на обоих устройствах.</li>
            <li>На главном ПК задайте пароль копии восстановления (от 6 символов), нажмите «Создать хаб и копию восстановления» и сохраните копию в надёжном месте.</li>
            <li>Выберите локальный IPv4 этого ПК и нажмите «Включить обмен по Wi-Fi». Появятся QR-код и строка сопряжения.</li>
            <li><strong>Через QR:</strong> на Android откройте «Разделы → Синхронизация → Сканировать QR-код компьютера» и наведите камеру на экран ПК.</li>
            <li><strong>Без камеры:</strong> нажмите «Скопировать строку сопряжения», перенесите её на телефон и вставьте в поле «Вставить строку сопряжения вместо камеры». Затем нажмите «Подключиться по вставленному коду». На втором ПК вставьте её в поле «Строка QR другого компьютера» и запросите сопряжение.</li>
            <li>Сравните код на обоих экранах. На главном ПК нажмите «Код совпадает — разрешить», затем завершите подключение на другом устройстве. На втором ПК также задайте пароль своей копии восстановления.</li>
            <li>Нажмите «Синхронизировать» на подключённом устройстве. Для автоматического обмена включите соответствующий переключатель. Главный ПК должен оставаться запущенным.</li>
        </ol>
        <p>Строка сопряжения содержит адрес ПК и данные проверки защищённого соединения. Одного адреса или обычной ссылки недостаточно. Код действует 2 минуты: если он истёк, нажмите «Новый QR-код». Передавайте строку только своему устройству.</p>
        <p>Если подключения нет, проверьте общую Wi-Fi сеть и разрешите JARVIS входящие подключения в частной сети Windows. Изоляция устройств в гостевой сети может мешать обмену. Чаты и зашифрованный сейф включаются отдельно; API-ключи и мастер-пароль не передаются.</p>
    </details>
    {#if error}<p role="alert" class="error">{error}</p>{/if}
    {#if message}<p role="status">{message}</p>{/if}
    <label>Пароль копии восстановления (от 6 символов)<input type="password" bind:value={password} autocomplete="new-password"></label>
    {#if !status.prepared}
        <p>Этот LAB использует отдельный тестовый профиль. Создайте хаб или подключите этот ПК к хабу. Перед включением v2 будет создана и проверена зашифрованная копия.</p>
        <button disabled={busy||password.length<6} on:click={prepare}>Создать хаб и копию восстановления</button>
        <label>Строка QR другого компьютера<input bind:value={peerQr} autocomplete="off"></label>
        <button disabled={busy||!peerQr} on:click={pairBegin}>Запросить сопряжение второго ПК</button>
        {#if pairCode}<p>Сравните код на хабе: <strong>{pairCode}</strong></p><button disabled={busy||password.length<6} on:click={pairFinish}>Подтверждено на хабе — завершить</button>{/if}
        <button disabled={busy||!password} on:click={recoveryRestore}>Восстановить копию в пустом профиле</button>
    {:else}
        <p>Обычные данные разблокируются средствами ОС. Сейф сохраняет отдельный мастер-пароль. Копия восстановления защищена введённым здесь паролем.</p>
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
        <label>Локальный IPv4 компьютера<select bind:value={ip}>{#each ips as address}<option value={address}>{address}</option>{/each}</select></label>
        <label>Порт<input type="number" min="1024" max="65535" bind:value={port}></label>
        <button class="primary" disabled={busy||!ip} on:click={()=>action('lan_sync_start',{ip,port})}>Включить обмен по Wi-Fi</button>
        {#if !ips.length}<p>Локальная сеть не найдена. Подключите компьютер и телефон к одному роутеру, затем откройте раздел снова.</p>{/if}
    {:else}
        <p class="connected">Обмен включён: {status.address}</p>
        {#if qr}<img src={qr} width="280" height="280" alt="QR-код сопряжения телефона"><label>Строка сопряжения для телефона или второго ПК<textarea readonly value={status.qr} rows="4" spellcheck="false"></textarea></label><button disabled={busy} on:click={copyPairing}>Скопировать строку сопряжения</button><p>Код действует 2 минуты. Не публикуйте его и не отправляйте посторонним.</p>{:else}<p>QR-код истёк. Уже подключённые устройства продолжают работать.</p>{/if}
        <button disabled={busy} on:click={()=>action('lan_sync_qr')}>Новый QR-код</button>
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
    <p class="footnote">Компьютерный JARVIS должен оставаться запущенным. Если соединение не устанавливается, разрешите приложению входящие подключения только в частной сети. Резервные копии находятся в папке данных приложения / lan-sync / backups. Автозапуск сервера после перезапуска не включён.</p>
</section>
<style>
 .guide{border:1px solid #36565c;border-radius:12px;padding:16px;margin:20px 0;background:#0b2228}.guide summary{cursor:pointer;font-weight:700}.guide ol{padding-left:22px;font-size:13px;line-height:1.7;color:#d0e4e8}.guide li{margin:10px 0}
 .sync{font-family:'Manrope Variable',sans-serif;color:#eaf7f8;padding:24px;border:1px solid #29464d;border-radius:16px;background:linear-gradient(135deg,#12383c,#0b1b20)}.eyebrow{color:#5eeade;font-size:11px;font-weight:800;letter-spacing:.15em}h2{font-size:25px;letter-spacing:-.04em;margin:8px 0 18px}h3{font-size:16px;margin-top:24px}p{font-size:13px;line-height:1.7;color:#a4c1c7}label{display:flex;flex-direction:column;gap:8px;margin:16px 0;font-size:13px}.check{flex-direction:row;align-items:center}input,textarea,select,button{font:inherit;border:1px solid #36565c;border-radius:9px;background:#0b2228;color:#eaf7f8;padding:12px}button{cursor:pointer;margin:8px 8px 8px 0;font-size:12px}button:disabled{opacity:.45;cursor:default}.primary{background:#37d2c9;color:#05272d;font-weight:800}input:focus-visible,select:focus-visible,button:focus-visible{outline:2px solid #5eeade;outline-offset:3px}img{display:block;border-radius:14px;max-width:100%;height:auto;margin-top:16px}article{padding:16px;border:1px solid #36565c;border-radius:12px;margin-top:14px}.error{color:#ffb5be;white-space:pre-wrap}.connected{color:#77f2da}.footnote{font-size:11px}
</style>

<script lang="ts">
    import { onMount } from 'svelte'
    import { goto } from '@roxi/routify'
    onMount(() => { $goto('/settings') })
</script>

<main class="sync-page">
    <h1>Синхронизация устройств</h1>
    <p>Соедините компьютеры и Android в одной Wi-Fi сети. Здесь доступны сопряжение, обмен данными, разрешение конфликтов и восстановление доступа.</p>
    <p>Синхронизация перенесена в «Настройки → Синхронизация устройств».</p>
</main>

<style>
    .sync-page { max-width: 1000px; margin: 0 auto; padding: 24px; }
    h1 { font-size: 28px; margin-bottom: 12px; }
    p { line-height: 1.6; opacity: .8; margin-bottom: 24px; }
</style>

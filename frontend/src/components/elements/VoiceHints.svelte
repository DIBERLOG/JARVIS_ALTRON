<script lang="ts">
    import { isJarvisRunning, ipcConnected, sendTextCommand } from "@/stores"

    // Showcase phrases. Any wording works: the voice core picks the closest request.
    const hints = [
        { say: "Что у меня сегодня?", note: "погода, дела и новости" },
        { say: "Вечером нужен зонт?", note: "почасовой прогноз" },
        { say: "Что нового в технологиях за сегодня?", note: "новости с источниками" },
        { say: "Запомни идею: добавить управление музыкой", note: "заметка с датой" },
        { say: "Через сорок минут напомни выключить духовку", note: "точное напоминание" },
        { say: "Добавь молоко и хлеб в покупки", note: "список покупок" },
        { say: "Найди документ с курсовой, который я вчера редактировал", note: "поиск файлов" },
        { say: "Переведи это", note: "выделенный текст" },
        { say: "Включи режим работы", note: "VS Code и музыка" },
        { say: "Открой вход в GitHub", note: "через менеджер паролей" },
    ]

    $: ready = $isJarvisRunning && $ipcConnected
    let sent = -1

    function run(index: number) {
        if (!ready) return
        if (sendTextCommand(hints[index].say)) {
            sent = index
            setTimeout(() => { if (sent === index) sent = -1 }, 1600)
        }
    }
</script>

<section class="hints" aria-label="Примеры команд">
    <p class="hints-title">Попробуйте сказать <span>— или нажмите, чтобы выполнить</span></p>
    <div class="hints-list">
        {#each hints as hint, index}
            <button type="button" class="hint" class:sent={sent === index} style="--i:{index}" disabled={!ready}
                title={ready ? "Выполнить команду" : "Голосовое ядро не запущено"} on:click={() => run(index)}>
                <span class="quote">«{hint.say}»</span>
                <small>{sent === index ? "отправлено" : hint.note}</small>
            </button>
        {/each}
    </div>
</section>

<style lang="scss">
    .hints { width: min(100%, 900px); margin: 0 auto; padding: .4rem 1.5rem 1rem; }

    .hints-title {
        margin: 0 0 .7rem;
        color: #8fb3b8;
        font-size: .68rem;
        font-weight: 700;
        letter-spacing: .14em;
        text-transform: uppercase;

        span { color: #5f8489; font-weight: 500; letter-spacing: .02em; text-transform: none; }
    }

    .hints-list { display: flex; flex-wrap: wrap; gap: .5rem; }

    .hint {
        display: flex;
        flex-direction: column;
        align-items: flex-start;
        gap: .12rem;
        max-width: 100%;
        padding: .5rem .8rem;
        border: 1px solid #1c3439;
        border-radius: 12px;
        background: rgba(12, 26, 30, .6);
        color: #dff6f6;
        font: inherit;
        text-align: left;
        cursor: pointer;
        transition: border-color .25s, background .25s, transform .3s cubic-bezier(.22, 1, .36, 1);
        animation: hint-in .6s cubic-bezier(.22, 1, .36, 1) both;
        animation-delay: calc(var(--i) * 45ms + 300ms);

        .quote { font-size: .78rem; }
        small { color: #6f9499; font-size: .64rem; transition: color .25s; }

        &:hover:not(:disabled) { border-color: #2c6a6f; background: rgba(20, 48, 54, .7); transform: translateY(-2px); }
        &:active:not(:disabled) { transform: scale(.98); }
        &:disabled { cursor: default; opacity: .55; }
        &.sent { border-color: #2adbb8; small { color: #2adbb8; } }
    }

    @keyframes hint-in {
        from { opacity: 0; transform: translateY(10px); }
        to { opacity: 1; transform: none; }
    }

    @media (prefers-reduced-motion: reduce) {
        .hint { animation: none; }
        .hint:hover:not(:disabled) { transform: none; }
    }
</style>

# ### APP INFO
app-name = JARVIS
app-description = Голосовий асистент

# ### TRAY MENU
tray-restart = Перезапустити
tray-settings = Налаштування
tray-exit = Вихід
tray-tooltip = JARVIS - Голосовий асистент
tray-language = Мова
tray-voice = Голос
tray-wake-word = Рушій детекції
tray-noise-suppression = Шумозаглушення
tray-vad = Детекцiя голосу (VAD)
tray-gain-normalizer = Нормалізація гучності

# ### HEADER
header-commands = КОМАНДИ
header-settings = НАЛАШТУВАННЯ

# ### SEARCH
search-placeholder = Введіть команду вручну або скажіть «Джарвіс» ...

# ### MAIN PAGE
assistant-not-running = АСИСТЕНТ НЕ ЗАПУЩЕНО
assistant-offline-hint = Налаштувати його можна не запускаючи.
btn-start = ЗАПУСТИТИ
btn-starting = ЗАПУСК...

# ### STATUS
status-disconnected = Відключено
status-standby = Очікування
status-listening = Слухаю...
status-processing = Обробка...

# ### STATS
stats-microphone = МІКРОФОН
stats-neural-networks = НЕЙРОМЕРЕЖІ
stats-resources = РЕСУРСИ
stats-system-default = Системний
stats-not-selected = Не вибрано
stats-loading = Завантаження...

# ### FOOTER
footer-author = Автор проєкту
footer-telegram = Наш телеграм канал
footer-github = Github репозиторій проєкту
footer-support = Підтримати проєкт на

# ### SETTINGS
settings-title = Налаштування
settings-general = Основні
settings-devices = Пристрої
settings-neural-networks = Нейромережі
settings-audio = Аудіо
settings-recognition = Розпізнавання
settings-about = Про програму
settings-language = Мова
settings-microphone = Мікрофон
settings-microphone-desc = Його буде слухати асистент.
settings-mic-default = За замовчуванням (Система)
settings-voice = Голос асистента
settings-voice-desc =
    Не всі команди працюють з усіма звуковими пакетами.
    Натисніть, щоб прослухати як звучить голос.
settings-wake-word-engine = Рушій активації
settings-wake-word-desc = Виберіть нейромережу для розпізнавання активаційної фрази.
settings-stt-engine = Розпізнавання мовлення
settings-intent-engine = Визначення наміру
settings-intent-engine-desc = Виберіть нейромережу для розпізнавання команд.
settings-noise-suppression = Шумозаглушення
settings-noise-suppression-desc = Зменшує фоновий шум. Може негативно впливати на розпізнавання.
settings-vad = Визначення голосу (VAD)
settings-vad-desc = Пропускає тишу, економить ресурси CPU.
settings-gain-normalizer = Нормалізація гучності
settings-gain-normalizer-desc = Автоматично регулює рівень гучності.
settings-api-keys = API Ключі
settings-save = Зберегти
settings-cancel = Скасувати
settings-back = Назад
settings-enabled = Увімкнено
settings-disabled = Вимкнено

# settings - beta notice
settings-beta-title = БЕТА версія!
settings-beta-desc = Частина функцій може працювати некоректно.
settings-beta-feedback = Повідомляйте про всі знайдені баги в
settings-beta-bot = наш телеграм бот
settings-open-logs = Відкрити папку з логами

# settings - picovoice
settings-attention = Увага!
settings-picovoice-warning = Ця нейромережа працює не у всіх!
settings-picovoice-waiting = Ми чекаємо офіційного патча від розробників.
settings-picovoice-key-desc = Введіть сюди свій ключ Picovoice. Він видається безкоштовно при реєстрації в
settings-picovoice-key = Ключ Picovoice

# settings - vosk
settings-auto-detect = Авто-визначення
settings-vosk-model = Модель розпізнавання мовлення (Vosk)
settings-vosk-model-desc =
    Виберіть модель Vosk для розпізнавання мовлення.
    Ви можете завантажити моделі тут: https://alphacephei.com/vosk/models
settings-models-not-found = Моделі не знайдено
settings-models-hint = Помістіть моделі Vosk в папку resources/vosk

# settings - openai
settings-openai-key = Ключ OpenAI
settings-openai-not-supported = Наразі ChatGPT не підтримується. Він буде доданий у наступних оновленнях.

# ### COMMANDS PAGE
commands-title = Команди
commands-search = Пошук команд...
commands-count = { $count } команд
commands-wip-title = [404] Цей розділ ще в розробці!
commands-wip-desc = Тут буде список команд + повноцінний редактор команд.
commands-wip-follow = Слідкуйте за оновленнями в
commands-wip-channel = нашому телеграм каналі

# ### ERRORS
error-generic = Сталася помилка
error-connection = Помилка підключення
error-not-found = Не знайдено

# ### NOTIFICATIONS
notification-saved = Налаштування збережено!
notification-error = Помилка
notification-assistant-started = Асистент запущено
notification-assistant-stopped = Асистент зупинено

# SLOTS EXTRACTION
settings-slot-engine = Витяг параметрів
settings-slot-engine-desc = Витягує параметри з голосових команд (напр. назва міста, число).
settings-gliner-model = Модель GLiNER ONNX
settings-gliner-model-desc = 
    Оберіть варіант моделі.
    Квантизовані моделі (int8, uint8) швидші, але менш точні.
settings-gliner-models-hint = Моделі GLiNER не знайдено.

# ETC
search-error-not-running = Асистент не запущено
search-error-failed = Не вдалося виконати команду
settings-no-voices = Голоси не знайдено
# ### NOTES
header-notes = НОТАТКИ
notes-title = Нотатки
notes-new = Створити
notes-search = Пошук нотаток
notes-loading = Завантаження...
notes-empty = Нотаток поки немає
notes-locked-title = Нотатки заблоковано
notes-locked-desc = Нотатки зберігаються на цьому комп'ютері зашифрованими. Розблокуйте сховище, щоб прочитати їх.
notes-uninitialized-title = Створіть майстер-пароль
notes-uninitialized-desc = Нотатки шифруються випадковим ключем, який відкриває лише цей пароль. Відновити пароль неможливо.
notes-key-missing-title = Ключ сховища відсутній
notes-key-missing-desc = Зашифрована база на місці, але файл ключа втрачено. Імпортуйте переносну резервну копію, щоб повернути нотатки.
notes-password = Майстер-пароль
notes-password-confirm = Повторіть пароль
notes-password-hint = Щонайменше 8 символів. Збережіть його в надійному місці: без нього нотатки не розшифрувати.
notes-password-short = Пароль занадто короткий
notes-password-mismatch = Паролі не збігаються
notes-create = Створити
notes-unlock = Розблокувати
notes-unlock-dpapi = Розблокувати через Windows
notes-import-file = Імпорт із файлу
notes-import-required = Потрібна переносна копія
notes-import-required-desc = Локальний файл ключа не знайдено — вставте конверт резервної копії нижче.
notes-envelope-placeholder = Вставте конверт переносної резервної копії
notes-storage-dir = Сховище
notes-has-data-hint = У цій теці вже є нотатки.
notes-export = Зберегти копію ключа
notes-export-hint = Збережіть переносну копію ключа та не втрачайте майстер-пароль.
notes-export-needs-password = Введіть майстер-пароль для нової копії
notes-lock = Заблокувати
notes-saved = Збережено
notes-saving = Збереження...
notes-dirty = Не збережено
notes-save-error = Помилка збереження
notes-error = Помилка нотаток
notes-save-now = Зберегти
notes-untitled = Без назви
notes-no-text = Немає тексту
notes-body-placeholder = Текст нотатки
notes-revision = Ревізія
notes-updated = Змінено
notes-back = Назад
notes-folder = Тека
notes-none = Немає
notes-new-folder = Нова тека
notes-folder-name = Назва теки
notes-rename = Перейменувати
notes-delete-folder = Видалити теку
notes-all-folders = Усі теки
notes-tags = Теги
notes-tags-placeholder = Теги через кому
notes-trash = До кошика
notes-restore = Відновити
notes-delete-forever = Видалити назавжди
notes-delete-confirm = Підтвердити видалення
notes-pin = Закріпити
notes-pinned = Закріплено
notes-unpin = Відкріпити
notes-sort-updated-desc = Спершу змінені
notes-sort-updated-asc = Спершу давні
notes-sort-created-desc = Спершу нові
notes-sort-created-asc = Спершу старі
notes-sort-title-asc = За назвою
notes-filter-active = Нотатки
notes-filter-trashed = Кошик
notes-filter-all = Усі
notes-time-now = щойно
notes-time-minute = хв тому
notes-time-hour = год тому
notes-time-day = дн тому
notes-time-date = давно
notes-conflicts = Невирішені конфлікти
notes-conflict-current = Поточна версія
notes-conflict-incoming = Вхідна версія
notes-conflict-unreadable = Цю версію неможливо прочитати
notes-conflict-keep-current = Залишити поточну
notes-conflict-accept-incoming = Прийняти вхідну
notes-conflict-keep-both = Зберегти обидві
notes-unreadable = нотаток не вдалося розшифрувати
# ### VAULT
header-vault = ПАРОЛІ
vault-title = Паролі
vault-new = Новий запис
vault-loading = Завантаження...
vault-empty = Записів поки немає
vault-untitled = Без назви
vault-none = Немає
vault-back = Назад
vault-error = Помилка сховища паролів
vault-lock = Заблокувати
vault-unlock = Розблокувати
vault-unlock-dpapi = Розблокувати через Windows
vault-locked-title = Сховище паролів заблоковано
vault-locked-desc = Паролі зберігаються на цьому комп'ютері зашифрованими. Той самий майстер-пароль захищає й нотатки.
vault-created-title = Створіть майстер-пароль
vault-created-desc = Сховище паролів використовує майстер-пароль зашифрованого сховища. Відновити його неможливо.
vault-key-missing-title = Ключ сховища відсутній
vault-key-missing-desc = Зашифровані бази на місці, але файл ключа втрачено. Імпортуйте переносну резервну копію.
vault-master-password = Майстер-пароль
vault-password-confirm = Повторіть пароль
vault-password-hint = Щонайменше 8 символів. Збережіть його в надійному місці: без нього нічого не розшифрувати.
vault-password-short = Пароль занадто короткий
vault-password-mismatch = Паролі не збігаються
vault-create = Створити
vault-import-required = Потрібна переносна копія
vault-import-required-desc = Локальний файл ключа не знайдено — вставте конверт резервної копії нижче.
vault-envelope-placeholder = Вставте конверт переносної резервної копії
vault-import-file = Імпорт із файлу
vault-storage-dir = Сховище
vault-shared-storage-hint = Нотатки й паролі використовують один майстер-ключ, але різні похідні ключі та різні бази.
vault-experimental-title = Експериментальна функція
vault-experimental-warning = Сховище не проходило аудит. Злам неможливо виключити, а забутий майстер-пароль не відновити.
vault-search = Пошук записів
vault-filter-active = Записи
vault-filter-trashed = Кошик
vault-filter-all = Усі
vault-filter-favorites = Обране
vault-sort-name-asc = За назвою
vault-sort-updated-desc = Спершу змінені
vault-sort-created-desc = Спершу нові
vault-name-placeholder = Назва
vault-username-placeholder = Логін
vault-password-placeholder = Пароль
vault-urls = Посилання
vault-urls-placeholder = По одному посиланню в рядку
vault-tags = Теги
vault-tags-placeholder = Теги через кому
vault-notes = Нотатки
vault-notes-placeholder = Особисті нотатки
vault-notes-hidden = Покажіть запис, щоб прочитати та змінити нотатки
vault-secret-hidden = Приховано. Натисніть кнопку з оком, щоб показати.
vault-password-length = Довжина
vault-reveal = Показати
vault-hide = Приховати
vault-copy-username = Копіювати логін
vault-copy-password = Копіювати пароль
vault-favorite = В обране
vault-favorited = В обраному
vault-unfavorite = Прибрати з обраного
vault-trash = До кошика
vault-restore = Відновити
vault-delete-forever = Видалити назавжди
vault-delete-confirm = Підтвердити видалення
vault-save-now = Зберегти
vault-security = Безпека
vault-idle-timeout = Блокувати після простою
vault-idle-1 = 1 хвилина
vault-idle-5 = 5 хвилин
vault-idle-15 = 15 хвилин
vault-idle-30 = 30 хвилин
vault-idle-never = Ніколи
vault-idle-automatic = Сховище блокується саме й очищає все розшифроване.
vault-idle-disabled = Автоматичне блокування вимкнено.
vault-clipboard-timeout = Очищати буфер через
vault-clipboard-15 = 15 секунд
vault-clipboard-30 = 30 секунд
vault-clipboard-45 = 45 секунд
vault-clipboard-60 = 60 секунд
vault-clipboard-armed = Буфер буде очищено через
vault-clipboard-clear = Очистити зараз
vault-clipboard-idle = Буфер порожній
vault-change-password = Зміна майстер-пароля
vault-current-password = Поточний майстер-пароль
vault-new-password = Новий майстер-пароль
vault-change-submit = Змінити
vault-change-done = Майстер-пароль змінено. Дані не перешифровувалися.
vault-change-done-dpapi = Майстер-пароль змінено, копію ключа для Windows оновлено.
vault-change-needs-current = Введіть поточний майстер-пароль
vault-change-same-password = Новий пароль збігається з поточним
vault-export-backup = Переносна резервна копія
vault-backup-password = Пароль для копії
vault-export = Зберегти копію
vault-export-done = Копію резервної копії збережено
vault-export-needs-password = Введіть пароль для резервної копії
vault-no-rotation = Зміна пароля перевертає обгортку майстер-ключа. Повна ротація самого ключа ще не реалізована.
vault-generator = Генератор
vault-generator-length = Довжина
vault-generator-similar = Без схожих
vault-generator-each = По одному з кожної
vault-generator-categories = Категорії
vault-generator-entropy = Ентропія
vault-generator-generate = Згенерувати
vault-generator-copy = Згенерувати й скопіювати
vault-generator-not-saved = Ще не збережено
vault-generator-needs-category = Виберіть хоча б одну категорію символів
vault-generator-length-invalid = Довжина має бути від 8 до 128
vault-generator-too-short = Занадто коротко для символу з кожної категорії
vault-generator-invalid = Налаштування генератора непридатні
vault-conflicts = Невирішені конфлікти
vault-conflict-current = Поточна версія
vault-conflict-incoming = Вхідна версія
vault-conflict-unreadable = Цю версію неможливо прочитати
vault-conflict-keep-current = Залишити поточну
vault-conflict-accept-incoming = Прийняти вхідну
vault-conflict-keep-both = Зберегти обидві
vault-unreadable = записів не вдалося розшифрувати

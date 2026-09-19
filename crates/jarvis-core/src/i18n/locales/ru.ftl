# APP INFO
app-name = JARVIS
app-description = Голосовой ассистент

# TRAY MENU
tray-restart = Перезапустить
tray-settings = Настройки
tray-exit = Выход
tray-tooltip = JARVIS - Голосовой ассистент
tray-language = Язык
tray-voice = Голос
tray-wake-word = Движок wake-word
tray-noise-suppression = Шумоподавление
tray-vad = Детекция голоса (VAD)
tray-gain-normalizer = Нормализация громкости

# HEADER
header-commands = КОМАНДЫ
header-settings = НАСТРОЙКИ

# SEARCH
search-placeholder = Введите команду вручную или произнесите «Джарвис» ...

# MAIN PAGE
assistant-not-running = АССИСТЕНТ НЕ ЗАПУЩЕН
assistant-offline-hint = Настроить его можно не запуская.
btn-start = ЗАПУСТИТЬ
btn-starting = ЗАПУСК...

# STATUS
status-disconnected = Отключен
status-standby = Ожидание
status-listening = Слушаю...
status-processing = Обработка...

# STATS
stats-microphone = МИКРОФОН
stats-neural-networks = НЕЙРОСЕТИ
stats-resources = РЕСУРСЫ
stats-system-default = Системный
stats-not-selected = Не выбран
stats-loading = Загрузка...

# FOOTER
footer-author = Автор проекта
footer-telegram = Наш телеграм канал
footer-github = Github репозиторий проекта
footer-support = Поддержать проект на

# SETTINGS
settings-title = Настройки
settings-general = Основные
settings-devices = Устройства
settings-neural-networks = Нейросети
settings-audio = Аудио
settings-recognition = Распознавание
settings-about = О программе
settings-language = Язык
settings-microphone = Микрофон
settings-microphone-desc = Его будет слушать ассистент.
settings-mic-default = По умолчанию (Система)
settings-voice = Голос ассистента
settings-voice-desc =
    Не все команды работают со всеми звуковыми пакетами.
    Кликните, чтобы прослушать как звучит голос.
settings-wake-word-engine = Движок активации
settings-wake-word-desc = Выберите нейросеть для распознавания активационной фразы.
settings-stt-engine = Распознавание речи
settings-intent-engine = Определение намерения
settings-intent-engine-desc = Выберите нейросеть для распознавания команд.
settings-noise-suppression = Шумоподавление
settings-noise-suppression-desc = Уменьшает фоновый шум. Может негативно влиять на распознавание.
settings-vad = Определение голоса (VAD)
settings-vad-desc = Пропускает тишину, экономит ресурсы CPU.
settings-gain-normalizer = Нормализация громкости
settings-gain-normalizer-desc = Автоматически регулирует уровень громкости.
settings-api-keys = API Ключи
settings-save = Сохранить
settings-cancel = Отмена
settings-back = Назад
settings-enabled = Включено
settings-disabled = Отключено

# settings - beta notice
settings-beta-title = БЕТА версия!
settings-beta-desc = Часть функций может работать некорректно.
settings-beta-feedback = Сообщайте обо всех найденных багах в
settings-beta-bot = наш телеграм бот
settings-open-logs = Открыть папку с логами

# settings - picovoice
settings-attention = Внимание!
settings-picovoice-warning = Эта нейросеть работает не у всех!
settings-picovoice-waiting = Мы ждем официального патча от разработчиков.
settings-picovoice-key-desc = Введите сюда свой ключ Picovoice. Он выдается бесплатно при регистрации в
settings-picovoice-key = Ключ Picovoice

# settings - vosk
settings-auto-detect = Авто-определение
settings-vosk-model = Модель распознавания речи (Vosk)
settings-vosk-model-desc =
    Выберите модель Vosk для распознавания речи.
    Вы можете скачать модели здесь: https://alphacephei.com/vosk/models
settings-models-not-found = Модели не найдены
settings-models-hint = Поместите модели Vosk в папку resources/vosk

# settings - openai
settings-openai-key = Ключ OpenAI
settings-openai-not-supported = В данный момент ChatGPT не поддерживается. Он будет добавлен в ближайших обновлениях.

# COMMANDS PAGE
commands-title = Команды
commands-search = Поиск команд...
commands-count = { $count } команд
commands-wip-title = [404] Этот раздел еще находится в разработке!
commands-wip-desc = Тут будет список команд + полноценный редактор команд.
commands-wip-follow = Следите за обновлениями в
commands-wip-channel = нашем телеграм канале

# ERRORS
error-generic = Произошла ошибка
error-connection = Ошибка подключения
error-not-found = Не найдено

# NOTIFICATIONS
notification-saved = Настройки сохранены!
notification-error = Ошибка
notification-assistant-started = Ассистент запущен
notification-assistant-stopped = Ассистент остановлен

# SLOTS EXTRACTION
settings-slot-engine = Извлечение параметров
settings-slot-engine-desc = Извлекает параметры из голосовых команд (напр. название города, число).
settings-gliner-model = Модель GLiNER ONNX
settings-gliner-model-desc =
    Выберите вариант модели.
    Квантизированные модели (int8, uint8) быстрее, но менее точны.
settings-gliner-models-hint = Модели GLiNER не найдены.

# ETC
search-error-not-running = Ассистент не запущен
search-error-failed = Не удалось выполнить команду
settings-no-voices = Голоса не найдены
# ### NOTES
header-notes = ЗАМЕТКИ
notes-title = Заметки
notes-new = Создать
notes-search = Поиск заметок
notes-loading = Загрузка...
notes-empty = Заметок пока нет
notes-locked-title = Заметки заблокированы
notes-locked-desc = Заметки хранятся на этом компьютере в зашифрованном виде. Разблокируйте хранилище, чтобы прочитать их.
notes-uninitialized-title = Создайте мастер-пароль
notes-uninitialized-desc = Заметки шифруются случайным ключом, который открывает только этот пароль. Восстановить пароль невозможно.
notes-key-missing-title = Ключ хранилища отсутствует
notes-key-missing-desc = Зашифрованная база на месте, но файл ключа потерян. Импортируйте переносимую резервную копию, чтобы вернуть заметки.
notes-password = Мастер-пароль
notes-password-confirm = Повторите пароль
notes-password-hint = Не менее 8 символов. Сохраните его в надёжном месте: без него заметки не расшифровать.
notes-password-short = Пароль слишком короткий
notes-password-mismatch = Пароли не совпадают
notes-create = Создать
notes-unlock = Разблокировать
notes-unlock-dpapi = Разблокировать через Windows
notes-import-file = Импорт из файла
notes-import-required = Нужна переносимая копия
notes-import-required-desc = Локальный файл ключа не найден — вставьте конверт резервной копии ниже.
notes-envelope-placeholder = Вставьте конверт переносимой резервной копии
notes-storage-dir = Хранилище
notes-has-data-hint = В этой папке уже есть заметки.
notes-export = Сохранить копию ключа
notes-export-hint = Сохраните переносимую копию ключа и не теряйте мастер-пароль.
notes-export-needs-password = Введите мастер-пароль для новой копии
notes-lock = Заблокировать
notes-saved = Сохранено
notes-saving = Сохранение...
notes-dirty = Не сохранено
notes-save-error = Ошибка сохранения
notes-error = Ошибка заметок
notes-save-now = Сохранить
notes-untitled = Без названия
notes-no-text = Нет текста
notes-body-placeholder = Текст заметки
notes-revision = Ревизия
notes-updated = Изменено
notes-back = Назад
notes-folder = Папка
notes-none = Нет
notes-new-folder = Новая папка
notes-folder-name = Название папки
notes-rename = Переименовать
notes-delete-folder = Удалить папку
notes-all-folders = Все папки
notes-tags = Теги
notes-tags-placeholder = Теги через запятую
notes-trash = В корзину
notes-restore = Восстановить
notes-delete-forever = Удалить навсегда
notes-delete-confirm = Подтвердить удаление
notes-pin = Закрепить
notes-pinned = Закреплено
notes-unpin = Открепить
notes-sort-updated-desc = Сначала изменённые
notes-sort-updated-asc = Сначала давние
notes-sort-created-desc = Сначала новые
notes-sort-created-asc = Сначала старые
notes-sort-title-asc = По названию
notes-filter-active = Заметки
notes-filter-trashed = Корзина
notes-filter-all = Все
notes-time-now = только что
notes-time-minute = мин назад
notes-time-hour = ч назад
notes-time-day = дн назад
notes-time-date = давно
notes-conflicts = Неразрешённые конфликты
notes-conflict-current = Текущая версия
notes-conflict-incoming = Входящая версия
notes-conflict-unreadable = Эту версию невозможно прочитать
notes-conflict-keep-current = Оставить текущую
notes-conflict-accept-incoming = Принять входящую
notes-conflict-keep-both = Сохранить обе
notes-unreadable = заметок не удалось расшифровать
# ### VAULT
header-vault = ПАРОЛИ
vault-title = Пароли
vault-new = Новая запись
vault-loading = Загрузка...
vault-empty = Записей пока нет
vault-untitled = Без названия
vault-none = Нет
vault-back = Назад
vault-error = Ошибка хранилища паролей
vault-lock = Заблокировать
vault-unlock = Разблокировать
vault-unlock-dpapi = Разблокировать через Windows
vault-locked-title = Хранилище паролей заблокировано
vault-locked-desc = Пароли хранятся на этом компьютере в зашифрованном виде. Тот же мастер-пароль защищает и заметки.
vault-created-title = Создайте мастер-пароль
vault-created-desc = Хранилище паролей использует мастер-пароль шифрованного хранилища. Восстановить его невозможно.
vault-key-missing-title = Ключ хранилища отсутствует
vault-key-missing-desc = Зашифрованные базы на месте, но файл ключа потерян. Импортируйте переносимую резервную копию.
vault-master-password = Мастер-пароль
vault-password-confirm = Повторите пароль
vault-password-hint = Не менее 8 символов. Сохраните его в надёжном месте: без него ничего не расшифровать.
vault-password-short = Пароль слишком короткий
vault-password-mismatch = Пароли не совпадают
vault-create = Создать
vault-import-required = Нужна переносимая копия
vault-import-required-desc = Локальный файл ключа не найден — вставьте конверт резервной копии ниже.
vault-envelope-placeholder = Вставьте конверт переносимой резервной копии
vault-import-file = Импорт из файла
vault-storage-dir = Хранилище
vault-shared-storage-hint = Заметки и пароли используют один мастер-ключ, но разные производные ключи и разные базы.
vault-experimental-title = Экспериментальная функция
vault-experimental-warning = Хранилище не проходило аудит. Взлом невозможно исключить, а забытый мастер-пароль не восстановить.
vault-search = Поиск записей
vault-filter-active = Записи
vault-filter-trashed = Корзина
vault-filter-all = Все
vault-filter-favorites = Избранное
vault-sort-name-asc = По названию
vault-sort-updated-desc = Сначала изменённые
vault-sort-created-desc = Сначала новые
vault-name-placeholder = Название
vault-username-placeholder = Логин
vault-password-placeholder = Пароль
vault-urls = Ссылки
vault-urls-placeholder = По одной ссылке в строке
vault-tags = Теги
vault-tags-placeholder = Теги через запятую
vault-notes = Заметки
vault-notes-placeholder = Личные заметки
vault-notes-hidden = Покажите запись, чтобы прочитать и изменить заметки
vault-secret-hidden = Скрыто. Нажмите кнопку с глазом, чтобы показать.
vault-password-length = Длина
vault-reveal = Показать
vault-hide = Скрыть
vault-copy-username = Копировать логин
vault-copy-password = Копировать пароль
vault-favorite = В избранное
vault-favorited = В избранном
vault-unfavorite = Убрать из избранного
vault-trash = В корзину
vault-restore = Восстановить
vault-delete-forever = Удалить навсегда
vault-delete-confirm = Подтвердить удаление
vault-save-now = Сохранить
vault-security = Безопасность
vault-idle-timeout = Блокировать после простоя
vault-idle-1 = 1 минута
vault-idle-5 = 5 минут
vault-idle-15 = 15 минут
vault-idle-30 = 30 минут
vault-idle-never = Никогда
vault-idle-automatic = Хранилище блокируется само и очищает всё расшифрованное.
vault-idle-disabled = Автоматическая блокировка выключена.
vault-clipboard-timeout = Очищать буфер через
vault-clipboard-15 = 15 секунд
vault-clipboard-30 = 30 секунд
vault-clipboard-45 = 45 секунд
vault-clipboard-60 = 60 секунд
vault-clipboard-armed = Буфер будет очищен через
vault-clipboard-clear = Очистить сейчас
vault-clipboard-idle = Буфер пуст
vault-change-password = Смена мастер-пароля
vault-current-password = Текущий мастер-пароль
vault-new-password = Новый мастер-пароль
vault-change-submit = Сменить
vault-change-done = Мастер-пароль изменён. Данные не перешифровывались.
vault-change-done-dpapi = Мастер-пароль изменён, копия ключа для Windows обновлена.
vault-change-needs-current = Введите текущий мастер-пароль
vault-change-same-password = Новый пароль совпадает с текущим
vault-export-backup = Переносимая резервная копия
vault-backup-password = Пароль для копии
vault-export = Сохранить копию
vault-export-done = Копия резервной копии сохранена
vault-export-needs-password = Введите пароль для резервной копии
vault-no-rotation = Смена пароля переворачивает обёртку мастер-ключа. Полная ротация самого ключа пока не реализована.
vault-generator = Генератор
vault-generator-length = Длина
vault-generator-similar = Без похожих
vault-generator-each = По одному из каждой
vault-generator-categories = Категории
vault-generator-entropy = Энтропия
vault-generator-generate = Сгенерировать
vault-generator-copy = Сгенерировать и скопировать
vault-generator-not-saved = Ещё не сохранён
vault-generator-needs-category = Выберите хотя бы одну категорию символов
vault-generator-length-invalid = Длина должна быть от 8 до 128
vault-generator-too-short = Слишком коротко для символа из каждой категории
vault-generator-invalid = Настройки генератора непригодны
vault-conflicts = Неразрешённые конфликты
vault-conflict-current = Текущая версия
vault-conflict-incoming = Входящая версия
vault-conflict-unreadable = Эту версию невозможно прочитать
vault-conflict-keep-current = Оставить текущую
vault-conflict-accept-incoming = Принять входящую
vault-conflict-keep-both = Сохранить обе
vault-unreadable = записей не удалось расшифровать

# ### LOCAL AI (настройки модели)
ai-settings-title = Локальный ИИ
ai-settings-desc = Локальная модель работает на этом компьютере. Укажите исполняемый файл llama-server и файл модели GGUF; ничего не скачивается автоматически.
ai-settings-server-path = Исполняемый файл llama-server
ai-settings-model-path = Файл модели GGUF
ai-settings-browse = Обзор
ai-settings-host = Интерфейс
ai-settings-host-desc = Только loopback. Другие устройства в сети не могут подключиться к серверу модели.
ai-settings-port = Порт
ai-settings-context = Размер контекста (токены)
ai-settings-context-desc = Больший контекст требует больше памяти.
ai-settings-threads = Потоки CPU
ai-settings-threads-desc = 0 — пусть решает llama.cpp.
ai-settings-gpu-layers = Слои на GPU
ai-settings-gpu-layers-desc = 0 — только CPU. Память GPU не оценивается.
ai-settings-timeout = Тайм-аут запуска (секунды)
ai-settings-temperature = Температура
ai-settings-top-p = Top-p
ai-settings-max-tokens = Максимум токенов ответа
ai-settings-loopback-note = Сервер модели слушает только loopback, и приложение останавливает только тот процесс, который запустило само.
ai-settings-report-title = Перед запуском
ai-settings-report-ok = Конфигурацию можно использовать.
ai-settings-report-warning = Можно использовать, но обратите внимание на предупреждения.
ai-settings-report-blocked = Запуск запрещён, пока эти проблемы не устранены.
ai-settings-architecture = Заявленная архитектура
ai-settings-quantisation = Заявленное квантование
ai-settings-declared-note = Эти значения прочитаны из заголовка файла. JARVIS не проверяет, действительно ли модель соответствует заявленному.
ai-settings-memory = Оценка потребности
ai-settings-memory-available = Доступная память
ai-settings-saved = Настройки ИИ сохранены.
ai-settings-save = Сохранить настройки ИИ
ai-settings-export = Экспорт
ai-settings-import = Импорт
ai-settings-field-server-path = llama-server
ai-settings-field-model-path = Файл модели
ai-settings-field-host = Интерфейс
ai-settings-field-port = Порт
ai-settings-field-context = Размер контекста
ai-settings-field-threads = Потоки CPU
ai-settings-field-gpu-layers = Слои на GPU
ai-settings-field-timeout = Тайм-аут запуска
ai-settings-field-temperature = Температура
ai-settings-field-top-p = Top-p
ai-settings-field-max-tokens = Максимум токенов
ai-settings-field-allow-lan = Сетевой доступ
ai-settings-field-schema = Версия настроек
ai-settings-field-generic = Настройки
ai-issue-server-missing = укажите исполняемый файл llama-server
ai-issue-model-missing = укажите файл модели GGUF
ai-issue-model-extension = имя файла не заканчивается на .gguf
ai-issue-allow-lan = доступ за пределы loopback в этой версии недоступен
ai-issue-host-loopback = принимается только loopback-адрес: 127.0.0.1, ::1 или localhost
ai-issue-port = выберите порт от 1024 до 65535
ai-issue-context = контекст должен быть от 512 до 262144 токенов
ai-issue-threads = слишком большое число потоков
ai-issue-gpu-layers = слишком большое число слоёв GPU
ai-issue-timeout = тайм-аут должен быть от 5 до 900 секунд
ai-issue-temperature = температура должна быть от 0 до 2
ai-issue-top-p = top-p должен быть от 0.05 до 1
ai-issue-max-tokens = максимум должен быть от 1 до 32768 токенов

# ### LOCAL AI (чат)
ai-chat-title = Локальный ИИ
ai-chat-local-note = Работает только на этом компьютере. Диалог не сохраняется: закрытие окна завершает его.
ai-chat-profile = Профиль
ai-chat-profile-desc = Профиль меняет тон и глубину ответов, но не права.
ai-chat-profile-jarvis = JARVIS
ai-chat-profile-altron = ALTRON
ai-chat-thinking = Вывод рассуждений
ai-chat-thinking-desc = Зависит от шаблона чата модели и сборки сервера.
ai-chat-thinking-auto = Автоматически
ai-chat-thinking-enabled = Включено
ai-chat-thinking-disabled = Отключено
ai-chat-thinking-unavailable = Этот сервер не сообщает о переключателе рассуждений, поэтому настройка не применяется.
ai-chat-state-stopped = остановлен
ai-chat-state-starting = запускается
ai-chat-state-ready = готов
ai-chat-state-generating = генерирует
ai-chat-state-stopping = останавливается
ai-chat-state-failed = ошибка
ai-chat-start = Запустить модель
ai-chat-stop-server = Остановить модель
ai-chat-restart = Перезапустить
ai-chat-clear = Очистить
ai-chat-not-running = Сервер модели не запущен.
ai-chat-needs-config = Сначала укажите в настройках исполняемый файл llama-server и модель GGUF.
ai-chat-uptime = работает
ai-chat-cap-streaming = поток
ai-chat-cap-thinking = рассуждения
ai-chat-cap-reasoning = поле рассуждений
ai-chat-out-of-memory = Сервер сообщил о нехватке памяти. Уменьшите контекст или число слоёв GPU.
ai-chat-stderr = Сообщения сервера
ai-chat-reasoning = Рассуждения
ai-chat-generating = Генерация…
ai-chat-cancelled = Генерация отменена.
ai-chat-no-answer = Модель не вернула текст.
ai-chat-input = Напишите сообщение. Enter — отправить, Shift+Enter — новая строка.
ai-chat-send = Отправить
ai-chat-stop = Остановить
ai-chat-elapsed = Время
ai-chat-tokens = Токены

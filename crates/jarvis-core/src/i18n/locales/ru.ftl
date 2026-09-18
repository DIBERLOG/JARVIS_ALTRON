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

# ### APP INFO
app-name = JARVIS
app-description = Voice Assistant

# ### TRAY MENU
tray-restart = Restart
tray-settings = Settings
tray-exit = Exit
tray-tooltip = JARVIS - Voice Assistant
tray-language = Language
tray-voice = Voice
tray-wake-word = Wake Word Engine
tray-noise-suppression = Noise Suppression
tray-vad = Voice Activity Detection
tray-gain-normalizer = Gain Normalizer

# ### HEADER
header-commands = COMMANDS
header-settings = SETTINGS

# ### SEARCH
search-placeholder = Enter a command manually or say «Jarvis» ...

# ### MAIN PAGE
assistant-not-running = ASSISTANT NOT RUNNING
assistant-offline-hint = You can configure it without starting.
btn-start = START
btn-starting = STARTING...

# ### STATUS
status-disconnected = Disconnected
status-standby = Standby
status-listening = Listening...
status-processing = Processing...

# ### STATS
stats-microphone = MICROPHONE
stats-neural-networks = NEURAL NETWORKS
stats-resources = RESOURCES
stats-system-default = System Default
stats-not-selected = Not selected
stats-loading = Loading...

# ### FOOTER
footer-author = Project author
footer-telegram = Our Telegram channel
footer-github = Github repository
footer-support = Support the project on

# ### SETTINGS
settings-title = Settings
settings-general = General
settings-devices = Devices
settings-neural-networks = Neural Networks
settings-audio = Audio
settings-recognition = Recognition
settings-about = About
settings-language = Language
settings-microphone = Microphone
settings-microphone-desc = The assistant will listen to this microphone.
settings-mic-default = Default (System)
settings-voice = Assistant voice
settings-voice-desc =
    Not all commands work with all sound packs.
    Click to listen the preview of sound.
settings-wake-word-engine = Wake word engine
settings-wake-word-desc = Choose the engine for wake word recognition.
settings-stt-engine = Speech recognition
settings-intent-engine = Intent recognition
settings-intent-engine-desc = Select neural network for command recognition.
settings-noise-suppression = Noise suppression
settings-noise-suppression-desc = Reduces background noise. May negatively affect recognition.
settings-vad = Voice detection (VAD)
settings-vad-desc = Skips silence, saves CPU resources.
settings-gain-normalizer = Gain normalizer
settings-gain-normalizer-desc = Automatically adjusts volume level.
settings-api-keys = API Keys
settings-save = Save
settings-cancel = Cancel
settings-back = Back
settings-enabled = Enabled
settings-disabled = Disabled

# settings - beta notice
settings-beta-title = BETA version!
settings-beta-desc = Some features may not work correctly.
settings-beta-feedback = Report all bugs to
settings-beta-bot = our Telegram bot
settings-open-logs = Open logs folder

# settings - picovoice
settings-attention = Attention!
settings-picovoice-warning = This neural network doesn't work for everyone!
settings-picovoice-waiting = We are waiting for an official patch from the developers.
settings-picovoice-key-desc = Enter your Picovoice key here. It is issued for free upon registration at
settings-picovoice-key = Picovoice Key

# settings - vosk
settings-auto-detect = Auto-detect
settings-vosk-model = Speech recognition model (Vosk)
settings-vosk-model-desc =
    Select Vosk model for speech recognition.
    You can download models here: https://alphacephei.com/vosk/models
settings-models-not-found = Models not found
settings-models-hint = Place Vosk models in resources/vosk folder

# settings - openai
settings-openai-key = OpenAI Key
settings-openai-not-supported = ChatGPT is not currently supported. It will be added in future updates.

# ### COMMANDS PAGE
commands-title = Commands
commands-search = Search commands...
commands-count = { $count } commands
commands-wip-title = [404] This section is under development!
commands-wip-desc = Here will be a list of commands + full-featured command editor.
commands-wip-follow = Follow updates in
commands-wip-channel = our Telegram channel

# ### ERRORS
error-generic = An error occurred
error-connection = Connection error
error-not-found = Not found

# ### NOTIFICATIONS
notification-saved = Settings saved!
notification-error = Error
notification-assistant-started = Assistant started
notification-assistant-stopped = Assistant stopped

# SLOTS EXTRACTION
settings-slot-engine = Slot extraction
settings-slot-engine-desc = Extract parameters from voice commands (e.g. city name, number).
settings-gliner-model = GLiNER ONNX model
settings-gliner-model-desc =
    Select model variant.
    Smaller quantized models (int8, uint8) are faster but less accurate.
settings-gliner-models-hint = No GLiNER models found.

# ETC
search-error-not-running = Assistant is not running
search-error-failed = Failed to execute command
settings-no-voices = No voices found
# ### NOTES
header-notes = NOTES
notes-title = Notes
notes-new = New
notes-search = Search notes
notes-loading = Loading...
notes-empty = No notes yet
notes-locked-title = Notes are locked
notes-locked-desc = Your notes are stored encrypted on this computer. Unlock the storage to read them.
notes-uninitialized-title = Create a master password
notes-uninitialized-desc = Notes are encrypted with a random key that only this password unlocks. The password cannot be recovered.
notes-key-missing-title = Storage key is missing
notes-key-missing-desc = The encrypted database is here, but its key file is gone. Import a portable backup to recover the notes.
notes-password = Master password
notes-password-confirm = Repeat the password
notes-password-hint = At least 8 characters. Keep it somewhere safe: without it the notes cannot be decrypted.
notes-password-short = The password is too short
notes-password-mismatch = The passwords do not match
notes-create = Create
notes-unlock = Unlock
notes-unlock-dpapi = Unlock with Windows
notes-import-file = Import backup file
notes-import-required = Portable backup required
notes-import-required-desc = No local key file was found, so paste the portable backup envelope below.
notes-envelope-placeholder = Paste the portable backup envelope here
notes-storage-dir = Storage
notes-has-data-hint = Existing notes were found in this folder.
notes-export = Save a backup copy
notes-export-hint = Save a portable backup copy and keep the master password safe.
notes-export-needs-password = Enter the master password for the new backup
notes-lock = Lock
notes-saved = Saved
notes-saving = Saving...
notes-dirty = Unsaved
notes-save-error = Not saved
notes-error = Notes error
notes-save-now = Save now
notes-untitled = Untitled
notes-no-text = No text
notes-body-placeholder = Write your note
notes-revision = Revision
notes-updated = Changed
notes-back = Back
notes-folder = Folder
notes-none = None
notes-new-folder = New folder
notes-folder-name = Folder name
notes-rename = Rename
notes-delete-folder = Delete folder
notes-all-folders = All folders
notes-tags = Tags
notes-tags-placeholder = Add tags, separated by commas
notes-trash = Trash
notes-restore = Restore
notes-delete-forever = Delete forever
notes-delete-confirm = Confirm deletion
notes-pin = Pin
notes-pinned = Pinned
notes-unpin = Unpin
notes-sort-updated-desc = Recently changed
notes-sort-updated-asc = Oldest change first
notes-sort-created-desc = Newest first
notes-sort-created-asc = Oldest first
notes-sort-title-asc = By title
notes-filter-active = Notes
notes-filter-trashed = Trash
notes-filter-all = All
notes-time-now = just now
notes-time-minute = min ago
notes-time-hour = h ago
notes-time-day = d ago
notes-time-date = long ago
notes-conflicts = Unresolved conflicts
notes-conflict-current = Current version
notes-conflict-incoming = Incoming version
notes-conflict-unreadable = This version cannot be read
notes-conflict-keep-current = Keep current
notes-conflict-accept-incoming = Accept incoming
notes-conflict-keep-both = Keep both
notes-unreadable = notes could not be decrypted

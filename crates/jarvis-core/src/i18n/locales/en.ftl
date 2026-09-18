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
# ### VAULT
header-vault = VAULT
vault-title = Passwords
vault-new = New item
vault-loading = Loading...
vault-empty = No entries yet
vault-untitled = Untitled
vault-none = None
vault-back = Back
vault-error = Vault error
vault-lock = Lock
vault-unlock = Unlock
vault-unlock-dpapi = Unlock with Windows
vault-locked-title = Vault is locked
vault-locked-desc = Passwords are stored encrypted on this computer. The same master password also protects your notes.
vault-created-title = Create a master password
vault-created-desc = The vault uses the encrypted storage master password. There is no recovery if it is lost.
vault-key-missing-title = Storage key is missing
vault-key-missing-desc = The encrypted databases are here, but the key file is gone. Import a portable backup to recover them.
vault-master-password = Master password
vault-password-confirm = Repeat the password
vault-password-hint = At least 8 characters. Keep it somewhere safe: without it nothing can be decrypted.
vault-password-short = The password is too short
vault-password-mismatch = The passwords do not match
vault-create = Create
vault-import-required = Portable backup required
vault-import-required-desc = No local key file was found, so paste the portable backup envelope below.
vault-envelope-placeholder = Paste the portable backup envelope here
vault-import-file = Import backup file
vault-storage-dir = Storage
vault-shared-storage-hint = Notes and passwords share one master key, each with its own derived key and database.
vault-experimental-title = Experimental feature
vault-experimental-warning = This vault has not been audited. It cannot be made unbreakable, and a forgotten master password cannot be recovered.
vault-search = Search entries
vault-filter-active = Entries
vault-filter-trashed = Trash
vault-filter-all = All
vault-filter-favorites = Favorites
vault-sort-name-asc = By name
vault-sort-updated-desc = Recently changed
vault-sort-created-desc = Newest first
vault-name-placeholder = Name
vault-username-placeholder = Username
vault-password-placeholder = Password
vault-urls = URLs
vault-urls-placeholder = One URL per line
vault-tags = Tags
vault-tags-placeholder = Tags, separated by commas
vault-notes = Notes
vault-notes-placeholder = Private notes
vault-notes-hidden = Reveal the entry to read and edit its notes
vault-secret-hidden = Hidden. Use the eye button to reveal it.
vault-password-length = Length
vault-reveal = Show
vault-hide = Hide
vault-copy-username = Copy username
vault-copy-password = Copy password
vault-favorite = Favorite
vault-favorited = Favorite
vault-unfavorite = Remove from favorites
vault-trash = Trash
vault-restore = Restore
vault-delete-forever = Delete forever
vault-delete-confirm = Confirm deletion
vault-save-now = Save now
vault-security = Security
vault-idle-timeout = Lock after inactivity
vault-idle-1 = 1 minute
vault-idle-5 = 5 minutes
vault-idle-15 = 15 minutes
vault-idle-30 = 30 minutes
vault-idle-never = Never
vault-idle-automatic = The vault locks itself and clears everything it decrypted.
vault-idle-disabled = Automatic locking is off.
vault-clipboard-timeout = Clear clipboard after
vault-clipboard-15 = 15 seconds
vault-clipboard-30 = 30 seconds
vault-clipboard-45 = 45 seconds
vault-clipboard-60 = 60 seconds
vault-clipboard-armed = Clipboard will be cleared
vault-clipboard-clear = Clear now
vault-clipboard-idle = Clipboard is clear
vault-change-password = Change master password
vault-current-password = Current master password
vault-new-password = New master password
vault-change-submit = Change
vault-change-done = Master password changed. The stored data was not re-encrypted.
vault-change-done-dpapi = Master password changed and the Windows key copy was refreshed.
vault-change-needs-current = Enter the current master password
vault-change-same-password = The new password matches the current one
vault-export-backup = Portable backup
vault-backup-password = Backup password
vault-export = Save copy
vault-export-done = Backup copy saved
vault-export-needs-password = Enter a password for the backup copy
vault-no-rotation = Changing the password re-wraps the master key. Rotating the master key itself is not implemented yet.
vault-generator = Generator
vault-generator-length = Length
vault-generator-similar = Avoid similar
vault-generator-each = One of each
vault-generator-categories = Categories
vault-generator-entropy = Entropy
vault-generator-generate = Generate
vault-generator-copy = Generate and copy
vault-generator-not-saved = Not saved yet
vault-generator-needs-category = Select at least one character category
vault-generator-length-invalid = Length must be between 8 and 128
vault-generator-too-short = Too short for one character of every category
vault-generator-invalid = The generator settings are not usable
vault-conflicts = Unresolved conflicts
vault-conflict-current = Current version
vault-conflict-incoming = Incoming version
vault-conflict-unreadable = This version cannot be read
vault-conflict-keep-current = Keep current
vault-conflict-accept-incoming = Accept incoming
vault-conflict-keep-both = Keep both
vault-unreadable = entries could not be decrypted

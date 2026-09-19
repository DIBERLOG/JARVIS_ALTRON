# JARVIS Voice Assistant (this readme is outdated)

![We are NOT limited by the technology of our time!](poster.jpg)

`Jarvis` - is a voice assistant made as an experiment using neural networks for things like **STT/TTS/Wake Word/NLU** etc.

The main project challenges we try to achieve is:
 - 100% offline *(no cloud)*
 - Open source *(full transparency)*
 - No data collection *(we respect your privacy)*

Our backend stack is 🦀 **[Rust](https://www.rust-lang.org/)** with ❤️ **[Tauri](https://tauri.app/)**.<br>
For the frontend we use ⚡️ **[Vite](https://vitejs.dev/)** + 🛠️ **[Svelte](https://svelte.dev/)**.

*Other libraries, tools and packages can be found in source code.*

## Neural Networks

This are the neural networks we are currently using:

 - Speech-To-Text
	 - [Vosk Speech Recognition Toolkit](https://github.com/alphacep/vosk-api) via [Vosk-rs](https://github.com/Bear-03/vosk-rs)
 - Text-To-Speech
	 - [~~Silero TTS~~](https://github.com/snakers4/silero-models) *(currently not used)*
	 - [~~Coqui TTS~~](https://github.com/coqui-ai/TTS) *(currently not used)*
	 - [~~WinRT~~](https://github.com/ndarilek/tts-rs) *(currently not used)*
	 - [~gTTS~](https://github.com/nightlyistaken/tts_rust) *(currently not used)*
	 - [~~SAM~~](https://github.com/s-macke/SAM) *(currently not used)*
 - Wake Word
	 - [Rustpotter](https://github.com/GiviMAD/rustpotter) *(Partially implemented, still WIP)*
	 - [Picovoice Porcupine](https://github.com/Picovoice/porcupine) via [official SDK](https://github.com/Picovoice/porcupine#rust) *(requires API key)*
	 - [Vosk Speech Recognition Toolkit](https://github.com/alphacep/vosk-api) via [Vosk-rs](https://github.com/Bear-03/vosk-rs) *(very slow)*
	 - [~~Snowboy~~](https://github.com/Kitt-AI/snowboy) *(currently not used)*
 - NLU
	 - Nothing yet.
- Chat
	- [~~ChatGPT~~](https://chat.openai.com/) (coming soon)

## Supported Languages

Currently, only Russian language is supported.<br>
But soon, Ukranian and English will be added for the interface, wake-word detection and speech recognition.

## How to build?

Nothing special was used to build this project.<br>
You need only Rust and NodeJS installed on your system.<br>
Other than that, all you need is to install all the dependencies and then compile the code with `cargo tauri build` command.<br>
Or run dev with `cargo tauri dev`.

<br><br>
*Thought you might need some of the platform specific libraries for [PvRecorder](https://github.com/Picovoice/pvrecorder) and [Vosk](https://github.com/alphacep/vosk-api).*

## Author

Abraham Tugalov

## Python version?
Old version of Jarvis was built with Python.<br>
The last Python version commit can be found [here](https://github.com/Priler/jarvis/tree/943efbfbdb8aeb5889fa5e2dc7348ca4ea0b81df).

## License

[Attribution-NonCommercial-ShareAlike 4.0 International](https://creativecommons.org/licenses/by-nc-sa/4.0/)<br>
See LICENSE.txt file for more details.

**Known conflict:** the Rust workspace metadata in `Cargo.toml` declares
`GPL-3.0-only`, while this file and `LICENSE.txt` describe CC-BY-NC-SA-4.0. The
two are not interchangeable: GPL-3.0-only permits commercial use and
CC-BY-NC-SA-4.0 forbids it. Only the copyright holder can resolve this, and no
attempt to choose one has been made by this project.

Until it is resolved:

* a build produced from this repository is named **Personal Windows MVP build**;
* **a public release is blocked**;
* the state of every component's licence, and what may be done with the build
  today, is recorded in [`docs/LICENSING_STATUS.md`](docs/LICENSING_STATUS.md)
  and [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md).

## Documentation

| Document | What it covers |
|---|---|
| [`docs/WINDOWS_MVP.md`](docs/WINDOWS_MVP.md) | what is ready for cautious personal testing, and what is not implemented |
| [`docs/INSTALLATION_WINDOWS.md`](docs/INSTALLATION_WINDOWS.md) | building and running on Windows, and what an installer will have to contain |
| [`docs/WINDOWS_ACTIONS.md`](docs/WINDOWS_ACTIONS.md) | the safe Windows command surface and its risk table |
| [`docs/WHISPER.md`](docs/WHISPER.md) | local dictation |
| [`docs/BACKUP_RESTORE.md`](docs/BACKUP_RESTORE.md) | what exists today, and the container that does not |
| [`docs/RUNTIME_DIAGNOSTICS.md`](docs/RUNTIME_DIAGNOSTICS.md) | the redacted diagnostics report |
| [`docs/RELEASE_CHECKLIST.md`](docs/RELEASE_CHECKLIST.md) | the checks that have to pass before anything is shared |
| [`docs/SECURITY.md`](docs/SECURITY.md) | the security baseline |

Wake-on-LAN is not implemented, is not planned, and is excluded from the project;
a test fails if the words appear in the workspace's own sources.

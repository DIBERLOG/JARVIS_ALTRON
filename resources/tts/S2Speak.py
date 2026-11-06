"""Persistent localhost-only Fish S2 Pro adapter for Jarvis's READY/OK protocol."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

import requests

URL = "http://127.0.0.1:29871"
MODEL_ID = "jarvis-s2"

def release_model():
    """Free S2 VRAM before another local neural engine is loaded."""
    session = requests.Session()
    session.trust_env = False
    try:
        response = session.post(URL + "/v1/tasks/unload_models", json={"model_ids": [MODEL_ID]}, timeout=(2, 120))
        response.raise_for_status()
    except requests.ConnectionError:
        pass  # No resident S2 server to unload.

def paths():
    here = Path(__file__).resolve().parent
    for project in here.parents:
        root = project / "tools/voice_training/s2_local_test"
        if (root / "runtime/audiocpp_server.exe").is_file():
            model = root / "models/Fish-Audio-S2-Pro-GGUF/fish-audio-s2-pro-q4_k-preserved-v2.gguf"
            reference = here / "xtts-references/jarvis_ru_012.wav"
            if model.is_file() and reference.is_file():
                return root, model, reference
    raise FileNotFoundError("Local S2 runtime/model/reference is missing")

def ensure_server(session, root):
    def ready():
        try:
            response = session.get(URL + "/v1/models", timeout=2)
            response.raise_for_status()
            data = response.json()
            if MODEL_ID not in json.dumps(data):
                raise RuntimeError("S2 port is occupied by another server")
            return True
        except requests.ConnectionError:
            return False
    if ready():
        return
    # GUI and voice module can prewarm simultaneously; serialize server creation.
    import msvcrt
    with (root / "server-start.lock").open("a+b") as lock:
        if lock.tell() == 0:
            lock.write(b"0")
            lock.flush()
        deadline = time.monotonic() + 180
        while True:
            try:
                lock.seek(0)
                msvcrt.locking(lock.fileno(), msvcrt.LK_NBLCK, 1)
                break
            except OSError:
                if time.monotonic() > deadline:
                    raise TimeoutError("S2 server startup lock timed out")
                time.sleep(.2)
        try:
            if ready():
                return
            with (root / "resident-server.log").open("a", encoding="utf-8") as log:
                process = subprocess.Popen([str(root / "runtime/audiocpp_server.exe"), "--config",
                    str(root / "resident-server.json"), "--no-ui", "--log"],
                    cwd=root, stdin=subprocess.DEVNULL, stdout=log, stderr=log,
                    creationflags=subprocess.CREATE_NO_WINDOW)
            while not ready():
                if process.poll() is not None:
                    raise RuntimeError("S2 server exited; see resident-server.log")
                if time.monotonic() > deadline:
                    process.terminate()
                    raise TimeoutError("S2 server startup timed out")
                time.sleep(.3)
        finally:
            lock.seek(0)
            msvcrt.locking(lock.fileno(), msvcrt.LK_UNLCK, 1)

def split_text(text, limit=180):
    """Sentence-sized pieces of at most `limit` characters, short ones merged."""
    import re
    sentences = [part.strip() for part in re.split(r"(?<=[.!?…])\s+", text) if part.strip()]
    pieces = []
    for sentence in sentences:
        while len(sentence) > limit:
            cut = max(sentence.rfind(mark, 0, limit) for mark in (", ", "; ", " — ", ": "))
            cut = cut + 1 if cut > limit // 3 else sentence.rfind(" ", 0, limit)
            if cut <= 0:
                cut = limit
            pieces.append(sentence[:cut].strip())
            sentence = sentence[cut:].strip()
        if pieces and len(pieces[-1]) + len(sentence) < 90:
            pieces[-1] += " " + sentence
        elif sentence:
            pieces.append(sentence)
    return pieces or [text]

def main():
    for stream in (sys.stdin, sys.stdout):
        if hasattr(stream, "reconfigure"):
            stream.reconfigure(encoding="utf-8")
    parser = argparse.ArgumentParser()
    parser.add_argument("--text")
    parser.add_argument("--server", action="store_true")
    parser.add_argument("--checkpoint")  # Compatibility with the existing local-voice launcher.
    parser.add_argument("--config")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--no-play", action="store_true")
    args = parser.parse_args()
    root, model, reference = paths()
    session = requests.Session()
    session.trust_env = False  # Never route local voice/audio through a system proxy.
    ensure_server(session, root)
    cache = Path(os.environ["LOCALAPPDATA"]) / "JarvisVoiceStudio/speech-cache/s2-pro"
    cache.mkdir(parents=True, exist_ok=True)
    profile = f"s2-q4:{model}:{model.stat().st_mtime_ns}:{reference.stat().st_mtime_ns}:28:.7:.8:30"
    def synthesize(text):
        """Returns the cached WAV path for one piece of text."""
        import soundfile as sf
        import numpy as np
        started = time.perf_counter()
        key = hashlib.sha256((profile + text).encode("utf-8")).hexdigest()
        output = cache / (key + ".wav")
        hit = output.is_file()
        if not hit:
            response = session.post(URL + "/v1/audio/speech", json={"model": MODEL_ID,
                "input": text, "seed": 28, "temperature": .7, "top_p": .8, "top_k": 30,
                "options": {"max_new_tokens": 1024}}, timeout=(5, 120))
            response.raise_for_status()
            if not response.content.startswith(b"RIFF"):
                raise RuntimeError("S2 returned no WAV")
            temporary = output.with_suffix(".tmp.wav")
            temporary.write_bytes(response.content)
            audio, rate = sf.read(temporary, dtype="float32")
            if not audio.size or not np.isfinite(audio).all() or np.max(np.abs(audio)) < 1e-4:
                raise RuntimeError("S2 returned invalid/silent audio")
            temporary.replace(output)
        with (cache / "latency.jsonl").open("a", encoding="utf-8") as log:
            log.write(json.dumps({"time": time.time(), "characters": len(text), "cache_hit": hit,
                "synthesis_ms": round((time.perf_counter() - started) * 1000, 1)}) + "\n")
        return output

    def speak(text):
        if args.output:  # Single file export for tools; playback stays streamed below.
            output = synthesize(text)
            args.output.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(output, args.output)
            if args.no_play:
                return
        # Long replies are spoken sentence by sentence: the next piece is
        # synthesized while the current one plays, so speech starts in seconds.
        import queue
        import threading
        import soundfile as sf
        import numpy as np
        ready = queue.Queue()
        def produce():
            try:
                for piece in split_text(text):
                    ready.put(synthesize(piece))
                ready.put(None)
            except Exception as error:
                ready.put(error)
        threading.Thread(target=produce, daemon=True).start()
        device = None
        while True:
            item = ready.get()
            if item is None:
                return
            if isinstance(item, Exception):
                raise item
            if args.no_play:
                continue
            import sounddevice as sd
            from AudioOutput import select_output
            if device is None:
                device = select_output(sd)
            audio, rate = sf.read(item, dtype="float32")
            sd.play(np.pad(audio, (0, round(rate * .12))), rate, device=device)
            sd.wait()
    if args.server:
        print("READY", flush=True)
        for line in sys.stdin:
            try:
                text = json.loads(line)["text"].strip()
                if text:
                    speak(text)
                print("OK", flush=True)
            except Exception as error:
                print("ERROR " + str(error).replace("\n", " "), flush=True)
    elif args.text and args.text.strip():
        speak(args.text.strip())
    return 0

if __name__ == "__main__":
    raise SystemExit(main())

"""Stops the shared S2 server once no JARVIS process is left, even after a crash."""
import csv
import os
import signal
import subprocess
import sys
import time

SERVER_NAME = "audiocpp_server.exe"
JARVIS_PREFIXES = ("jarvis-gui", "jarvis-app")
POLL_SECONDS = 5
MISSES_BEFORE_STOP = 2  # Lets the window and the voice core restart without losing the model.
CREATE_NO_WINDOW = 0x08000000


def parse_processes(output):
    """Maps pid to lowercase image name from `tasklist /FO CSV /NH` output."""
    processes = {}
    for row in csv.reader(output.splitlines()):
        if len(row) >= 2 and row[1].isdigit():
            processes[int(row[1])] = row[0].lower()
    return processes


def snapshot():
    result = subprocess.run(["tasklist", "/FO", "CSV", "/NH"], capture_output=True, timeout=30,
        stdin=subprocess.DEVNULL, creationflags=CREATE_NO_WINDOW, check=True)
    return parse_processes(result.stdout.decode("utf-8", "replace"))


def jarvis_running(processes):
    return any(name.startswith(JARVIS_PREFIXES) for name in processes.values())


def main(server_pid):
    if not jarvis_running(snapshot()):
        return 0  # Started by a standalone voice tool: that server is not ours to stop.
    misses = 0
    while True:
        time.sleep(POLL_SECONDS)
        try:
            processes = snapshot()
        except (OSError, subprocess.SubprocessError):
            continue
        if processes.get(server_pid) != SERVER_NAME:
            return 0  # The server is already gone; never touch a reused pid.
        misses = 0 if jarvis_running(processes) else misses + 1
        if misses >= MISSES_BEFORE_STOP:
            os.kill(server_pid, signal.SIGTERM)  # TerminateProcess on Windows.
            return 0


if __name__ == "__main__":
    raise SystemExit(main(int(sys.argv[1])))

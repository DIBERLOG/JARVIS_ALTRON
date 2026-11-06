"""Select JARVIS output without changing Windows audio defaults."""
import json
import os
from pathlib import Path


def output_mode():
    # JARVIS passes the route of the current profile; the default is direct output.
    mode = os.environ.get('JARVIS_AUDIO_OUTPUT')
    if mode is not None:
        return 'voicemod' if mode == 'voicemod' else 'direct'
    try:
        settings = Path(os.environ['APPDATA']) / 'com.priler.jarvis' / 'app.db'
        return json.loads(settings.read_text(encoding='utf-8')).get('audio_output_mode', 'direct')
    except (OSError, KeyError, ValueError):
        return 'direct'


def is_virtual(name):
    return any(word in name.lower() for word in ('cable', 'voicemod', 'virtual'))


def select_output(sd):
    devices = sd.query_devices()
    outputs = [(index, device) for index, device in enumerate(devices) if device['max_output_channels'] > 0]
    if output_mode() == 'voicemod':
        cable = next((index for index, device in outputs if 'cable input' in device['name'].lower()), None)
        if cable is None:
            raise RuntimeError('Voicemod output unavailable: install VB-CABLE or choose direct output')
        return cable
    default = sd.default.device[1]
    physical = [(index, device) for index, device in outputs if not is_virtual(device['name'])]
    if any(index == default for index, _ in physical):
        return default
    # Prefer headphones over a monitor's HDMI speakers when Windows defaults to a cable.
    for word in ('jbl', 'headphone', 'наушник', 'speaker', 'динамик'):
        selected = next((index for index, device in physical if word in device['name'].lower()), None)
        if selected is not None:
            return selected
    if physical:
        return physical[0][0]
    raise RuntimeError('No physical audio output found; connect headphones or speakers')

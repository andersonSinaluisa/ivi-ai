"""Voz del sistema de Windows (System.Speech / SAPI 5), sin clonación y sin dependencias.

Se invoca ``powershell.exe`` (Windows PowerShell 5.1, que trae System.Speech). El texto viaja por
un archivo temporal y las rutas por variables de entorno: nada del texto se interpola en el
script, así que no hay inyección de comandos.
"""

from __future__ import annotations

import base64
import os
import subprocess
import sys
import tempfile
from pathlib import Path

from ..audio.signal import decode_wav
from .base import Speech, TtsError, VoiceRef

SCRIPT = r"""
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Speech
$s = New-Object System.Speech.Synthesis.SpeechSynthesizer
$v = $s.GetInstalledVoices() | Where-Object { $_.Enabled -and $_.VoiceInfo.Culture.Name -like ($env:JARVIS_TTS_CULTURE + '*') } | Select-Object -First 1
if ($v) { $s.SelectVoice($v.VoiceInfo.Name) } else { [Console]::Error.WriteLine('NO_LOCALE_VOICE') }
$s.Rate = [int]$env:JARVIS_TTS_RATE
$s.SetOutputToWaveFile($env:JARVIS_TTS_OUT)
$s.Speak([IO.File]::ReadAllText($env:JARVIS_TTS_TEXT, [Text.Encoding]::UTF8))
$s.Dispose()
"""

CREATE_NO_WINDOW = 0x08000000


def speed_to_rate(speed: float) -> int:
    """1.0 → 0; 2.0 → 10; 0.5 → -5 (escala de SAPI: -10..10)."""
    return max(-10, min(10, round((speed - 1.0) * 10)))


def encoded_script() -> str:
    return base64.b64encode(SCRIPT.encode("utf-16-le")).decode("ascii")


class SapiTts:
    name = "sapi"
    supports_cloning = False

    def __init__(self, culture: str = "es") -> None:
        self.culture = culture
        self.warning: str | None = None

    @property
    def ready(self) -> bool:
        return sys.platform == "win32"

    @property
    def detail(self) -> str:
        if not self.ready:
            return "solo disponible en Windows"
        return self.warning or "voz de Windows"

    def synthesize(self, text: str, voice: VoiceRef | None, speed: float) -> Speech:  # noqa: ARG002
        if not self.ready:
            raise TtsError("la voz de Windows solo existe en Windows")
        with tempfile.TemporaryDirectory(prefix="jarvis-tts-") as tmp:
            text_path = Path(tmp) / "text.txt"
            out_path = Path(tmp) / "out.wav"
            text_path.write_text(text, encoding="utf-8")
            env = {
                **os.environ,
                "JARVIS_TTS_TEXT": str(text_path),
                "JARVIS_TTS_OUT": str(out_path),
                "JARVIS_TTS_RATE": str(speed_to_rate(speed)),
                "JARVIS_TTS_CULTURE": self.culture,
            }
            proc = subprocess.run(  # noqa: S603
                ["powershell.exe", "-NoProfile", "-NonInteractive", "-EncodedCommand", encoded_script()],
                env=env,
                capture_output=True,
                timeout=60,
                creationflags=CREATE_NO_WINDOW,
                check=False,
            )
            stderr = proc.stderr.decode("utf-8", "ignore")
            if "NO_LOCALE_VOICE" in stderr:
                self.warning = (
                    f"no hay voz de Windows para '{self.culture}': se usa la predeterminada. "
                    "Instala el paquete de voz en español en Configuración de Windows > Hora e idioma > Voz."
                )
            if proc.returncode != 0 or not out_path.is_file():
                raise TtsError(f"System.Speech falló: {stderr.strip()[:300]}")
            samples, rate = decode_wav(out_path.read_bytes())
        return Speech(samples, rate, self.name)

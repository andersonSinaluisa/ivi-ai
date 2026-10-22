"""VibeVoice (Microsoft) como proveedor de voz OPCIONAL y EXPERIMENTAL.

Lo que se sabe de ``VibeVoice-Realtime-0.5B`` (ficha del modelo, octubre de 2026):

- Licencia MIT, pensado para **investigación**; Microsoft no recomienda usarlo en aplicaciones reales sin más pruebas.
- Inglés de forma oficial; español solo «experimental». Un único hablante; **sin clonación de voz**.
- Añade un aviso audible de que el audio es de IA y una marca de agua imperceptible.
- Requiere GPU para ir en tiempo real (probado por Microsoft en NVIDIA T4 y Mac M4 Pro).
- Se ejecuta con los scripts del repositorio (``pip install -e .[streamingtts]``); no hay una API de Python estable
  documentada.

Por eso el adaptador no importa el modelo: ejecuta un **comando externo** configurable que recibe el texto en un
archivo y deja un WAV en la ruta indicada. Contrato del comando (``vibevoice_command`` en la configuración, lista de
argumentos sin shell):

    {text_file}  archivo UTF-8 con la frase
    {out_wav}    ruta donde el comando debe escribir un WAV PCM
    {voice}      hablante (p. ej. ``Carter``)

ESTADO: adaptador probado con un comando de prueba; **VibeVoice real no se ejecutó** (requiere GPU y descargar el
modelo). Si falla, ``FallbackTts`` usa la voz de Windows.
"""

from __future__ import annotations

import importlib.util
import os
import shutil
import subprocess
import tempfile
from pathlib import Path

from ..audio.signal import decode_wav
from .base import Speech, TtsError, VoiceRef

PLACEHOLDERS = ("{text_file}", "{out_wav}", "{voice}")


def vibevoice_status(command: list[str]) -> dict:
    """Estado para Configuración → Modelos (sin cargar nada pesado)."""
    package = importlib.util.find_spec("vibevoice") is not None
    torch = importlib.util.find_spec("torch") is not None
    executable = bool(command) and (shutil.which(command[0]) is not None or Path(command[0]).is_file())
    if not command:
        detail = "no configurado (opcional). Requiere GPU; oficialmente solo inglés"
    elif not executable:
        detail = f"no se encuentra el ejecutable '{command[0]}'"
    else:
        detail = "configurado (experimental: español sin garantías, añade un aviso audible de IA)"
    return {
        "configured": bool(command) and executable,
        "packageInstalled": package,
        "torchInstalled": torch,
        "detail": detail,
        "license": "MIT (uso de investigación recomendado por Microsoft)",
    }


class VibeVoiceTts:
    name = "vibevoice"
    supports_cloning = False

    def __init__(self, command: list[str], voice: str = "Carter", timeout_s: float = 120.0) -> None:
        self.command = list(command)
        self.voice = voice
        self.timeout_s = timeout_s
        self._error: str | None = None
        if not self.command:
            self._error = "vibevoice_command no está configurado"
        elif "{out_wav}" not in self.command or "{text_file}" not in self.command:
            self._error = "vibevoice_command debe incluir {text_file} y {out_wav}"

    @property
    def ready(self) -> bool:
        return self._error is None

    @property
    def detail(self) -> str:
        return self._error or "comando externo configurado (experimental)"

    def synthesize(self, text: str, voice: VoiceRef | None, speed: float) -> Speech:  # noqa: ARG002
        if self._error:
            raise TtsError(self._error)
        with tempfile.TemporaryDirectory(prefix="jarvis-vibevoice-") as tmp:
            text_file = Path(tmp) / "frase.txt"
            out_wav = Path(tmp) / "frase.wav"
            text_file.write_text(text, encoding="utf-8")
            # Sin shell: el texto nunca se interpola en la línea de comandos.
            args = [
                a.replace("{text_file}", str(text_file)).replace("{out_wav}", str(out_wav)).replace("{voice}", self.voice)
                for a in self.command
            ]
            try:
                result = subprocess.run(  # noqa: S603
                    args,
                    capture_output=True,
                    timeout=self.timeout_s,
                    check=False,
                    env={**os.environ, "PYTHONIOENCODING": "utf-8"},
                )
            except (OSError, subprocess.TimeoutExpired) as exc:
                raise TtsError(f"VibeVoice no se pudo ejecutar: {exc}") from exc
            if result.returncode != 0 or not out_wav.is_file():
                err = (result.stderr or b"").decode("utf-8", "replace").strip()[-300:]
                raise TtsError(f"VibeVoice terminó con código {result.returncode}: {err}")
            samples, rate = decode_wav(out_wav.read_bytes())
            return Speech(samples=samples, sample_rate=rate, provider=self.name)

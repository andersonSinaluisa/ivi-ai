"""Transcripción en tu servidor (jarvis-stt detrás de ``<servidor>/stt``).

El micrófono, el VAD y la palabra de activación siguen en la PC: solo se envía el audio de cada frase (WAV PCM de
16 kHz) por HTTPS con la credencial del equipo. En la PC no hay modelo de transcripción.
"""

from __future__ import annotations

import threading
from collections.abc import Callable

import numpy as np

from ..audio.signal import encode_wav
from ..cloud import CloudCredential
from ..config.settings import SAMPLE_RATE
from ..server_http import ServerClient, ServerError, multipart
from .base import Transcript


class RemoteStt:
    name = "server"

    def __init__(self, credential: CloudCredential, timeout_s: float = 60.0, opener: Callable[..., object] | None = None) -> None:
        kwargs = {"opener": opener} if opener else {}
        self.http = ServerClient(credential, "/stt", timeout_s, **kwargs)
        self._server: dict | None = None
        self._error: str | None = None
        self._lock = threading.Lock()

    @property
    def ready(self) -> bool:
        return self._error is None and self._server is not None and bool(self._server.get("ready"))

    @property
    def detail(self) -> str:
        if self._error:
            return self._error
        if self._server is None:
            return "servidor sin comprobar" if self.http.credential.get() else "sin conexión con tu servidor"
        state = "listo" if self._server.get("ready") else "cargando el modelo"
        return f"servidor · modelo {self._server.get('model', '?')} {state}"

    def load(self) -> None:
        """Comprueba el servidor (arranque, cambio de credencial y «Probar» en Configuración)."""
        with self._lock:
            try:
                self._server = self.http.json("/health")
                self._error = None if self._server.get("ready") else (self._server.get("error") or None)
            except ServerError as exc:
                self._server = None
                self._error = str(exc)
                raise

    def transcribe(self, audio: np.ndarray, language: str) -> Transcript:
        body, ctype = multipart(
            {"language": language, "response_format": "json"},
            "file",
            "frase.wav",
            encode_wav(audio.astype(np.float32), SAMPLE_RATE),
            "audio/wav",
        )
        try:
            data = self.http.json("/v1/audio/transcriptions", body, ctype)
        except ServerError as exc:
            self._error = str(exc)
            raise
        self._error = None
        return Transcript(
            text=str(data.get("text", "")).strip(),
            language=str(data.get("language") or language),
            duration_ms=int(1000 * audio.size / SAMPLE_RATE),
        )

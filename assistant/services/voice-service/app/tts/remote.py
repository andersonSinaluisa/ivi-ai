"""Voz generada en tu servidor (jarvis-tts con Piper detrás de ``<servidor>/tts``)."""

from __future__ import annotations

import json
import threading
from collections.abc import Callable

from ..audio.signal import decode_wav
from ..cloud import CloudCredential
from ..server_http import ServerClient, ServerError
from .base import Speech, TtsError


class RemoteTts:
    name = "server"

    def __init__(self, credential: CloudCredential, timeout_s: float = 60.0, opener: Callable[..., object] | None = None) -> None:
        kwargs = {"opener": opener} if opener else {}
        self.http = ServerClient(credential, "/tts", timeout_s, **kwargs)
        self._server: dict | None = None
        self._error: str | None = None
        self._lock = threading.Lock()

    @property
    def ready(self) -> bool:
        return self._error is None and self.http.credential.get() is not None

    @property
    def detail(self) -> str:
        if self._error:
            return self._error
        if self._server is None:
            return "servidor sin comprobar" if self.http.credential.get() else "sin conexión con tu servidor"
        state = "lista" if self._server.get("ready") else "cargando la voz"
        return f"servidor · voz {self._server.get('voice', '?')} {state}"

    @property
    def cache_id(self) -> str:
        """Identifica la voz para la caché (otro servidor u otra voz = otra entrada)."""
        voice = (self._server or {}).get("voice", "")
        return f"server:{self.http.credential.server_url or ''}:{voice}"

    def load(self) -> None:
        with self._lock:
            try:
                self._server = self.http.json("/health")
                self._error = None if self._server.get("ready") else (self._server.get("error") or None)
            except ServerError as exc:
                self._server = None
                self._error = str(exc)
                raise

    def synthesize(self, text: str, speed: float) -> Speech:
        payload = json.dumps({"input": text, "response_format": "wav", "speed": speed}).encode("utf-8")
        try:
            body, _ = self.http.call("/v1/audio/speech", payload, "application/json")
            samples, rate = decode_wav(body)
        except ServerError as exc:
            self._error = str(exc)
            raise TtsError(str(exc)) from exc
        except Exception as exc:  # noqa: BLE001 - WAV dañado
            self._error = f"audio inválido del servidor: {exc}"
            raise TtsError(self._error) from exc
        self._error = None
        return Speech(samples=samples, sample_rate=rate, provider=self.name)

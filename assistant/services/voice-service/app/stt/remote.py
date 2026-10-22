"""Transcripción en un servidor (jarvis-stt en el VPS u otro compatible con ``/v1/audio/transcriptions``).

El micrófono, el VAD y la palabra de activación siguen en la PC: solo se envía el audio de cada frase (WAV PCM de
16 kHz) por HTTPS con un token. Si el servidor no responde y hay un modelo local copiado en ``models/whisper/``,
se usa ese (sin intentar descargas). Sin red ni modelo local, la frase falla con un error claro.
"""

from __future__ import annotations

import json
import logging
import secrets
import threading
import urllib.error
import urllib.request
from collections.abc import Callable
from pathlib import Path
from urllib.parse import urlparse

import numpy as np

from ..audio.signal import encode_wav
from ..config.paths import data_dir
from ..config.settings import SAMPLE_RATE
from .whisper import Transcript

log = logging.getLogger(__name__)

TOKEN_FILE = "stt-remote-token"


def token_path(base: Path | None = None) -> Path:
    return (base or data_dir()) / "secrets" / TOKEN_FILE


def read_token(base: Path | None = None) -> str | None:
    path = token_path(base)
    try:
        value = path.read_text(encoding="utf-8").strip()
    except OSError:
        return None
    return value or None


def write_token(token: str | None, base: Path | None = None) -> None:
    """Guarda (o borra con ``None``/vacío) el token. Nunca va a voice.json ni se devuelve por la API."""
    path = token_path(base)
    if not token:
        path.unlink(missing_ok=True)
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(token.strip(), encoding="utf-8")
    try:
        path.chmod(0o600)
    except OSError:  # pragma: no cover - Windows: la carpeta del perfil ya es privada
        pass


def validate_url(url: str) -> str:
    """HTTPS obligatorio salvo hacia esta misma máquina (pruebas o un servidor local)."""
    parsed = urlparse(url.strip())
    host = (parsed.hostname or "").lower()
    loopback = host in {"localhost", "127.0.0.1", "::1"}
    if parsed.scheme != "https" and not (parsed.scheme == "http" and loopback):
        raise ValueError("el servidor de transcripción debe usar HTTPS (HTTP solo hacia localhost)")
    if not host:
        raise ValueError("URL inválida")
    return url.strip().rstrip("/")


def _multipart(fields: dict[str, str], file_name: str, file_bytes: bytes) -> tuple[bytes, str]:
    boundary = "jarvis" + secrets.token_hex(12)
    out = bytearray()
    for key, value in fields.items():
        out += f'--{boundary}\r\nContent-Disposition: form-data; name="{key}"\r\n\r\n{value}\r\n'.encode()
    out += (
        f'--{boundary}\r\nContent-Disposition: form-data; name="file"; filename="{file_name}"\r\n'
        "Content-Type: audio/wav\r\n\r\n"
    ).encode()
    out += file_bytes + f"\r\n--{boundary}--\r\n".encode()
    return bytes(out), f"multipart/form-data; boundary={boundary}"


class RemoteStt:
    name = "remote"

    def __init__(
        self,
        url: str,
        token: str | None,
        model: str = "base",
        timeout_s: float = 60.0,
        fallback: object | None = None,
        opener: Callable[..., object] = urllib.request.urlopen,
    ) -> None:
        self.url = url.strip().rstrip("/")
        self.model = model
        self.timeout_s = timeout_s
        self._token = token
        self._fallback = fallback
        self._open = opener
        self._error: str | None = None
        self._server: dict | None = None
        self._lock = threading.Lock()
        if not self.url:
            self._error = "falta la URL del servidor de transcripción"
        else:
            try:
                validate_url(self.url)
            except ValueError as exc:
                self._error = str(exc)

    # ------------------------------------------------------------------ estado
    @property
    def ready(self) -> bool:
        return self._error is None and self._server is not None and bool(self._server.get("ready"))

    @property
    def detail(self) -> str:
        if self._error:
            extra = " (se usa el modelo local)" if self._fallback_ready() else ""
            return f"{self._error}{extra}"
        if self._server is None:
            return f"servidor {self.url} sin comprobar"
        state = "listo" if self._server.get("ready") else "cargando el modelo"
        return f"servidor {self.url} · modelo {self._server.get('model', '?')} {state}"

    def _fallback_ready(self) -> bool:
        return self._fallback is not None

    def _request(self, path: str, data: bytes | None = None, content_type: str | None = None) -> dict:
        req = urllib.request.Request(self.url + path, data=data, method="POST" if data is not None else "GET")
        if self._token:
            req.add_header("Authorization", f"Bearer {self._token}")
        if content_type:
            req.add_header("Content-Type", content_type)
        req.add_header("User-Agent", "jarvis-voice-service")
        try:
            with self._open(req, timeout=self.timeout_s) as resp:  # type: ignore[attr-defined]
                return json.loads(resp.read().decode("utf-8"))
        except urllib.error.HTTPError as exc:
            if exc.code == 401:
                raise ConnectionError("el servidor rechazó el token (401)") from exc
            if exc.code == 413:
                raise ConnectionError("la frase es demasiado larga para el servidor (413)") from exc
            body = exc.read().decode("utf-8", "replace")[:200]
            raise ConnectionError(f"el servidor respondió {exc.code}: {body}") from exc
        except (urllib.error.URLError, TimeoutError, OSError) as exc:
            reason = getattr(exc, "reason", exc)
            raise ConnectionError(f"no se pudo conectar con {self.url}: {reason}") from exc

    def load(self) -> None:
        """Comprueba el servidor (lo llama el arranque del servicio y Configuración → Probar)."""
        if self._error and not self.url:
            raise ConnectionError(self._error)
        with self._lock:
            try:
                self._server = self._request("/health")
                self._error = None if self._server.get("ready") else self._server.get("error") or None
            except ConnectionError as exc:
                self._server = None
                self._error = str(exc)
                raise

    # ------------------------------------------------------------------ transcripción
    def transcribe(self, audio: np.ndarray, language: str) -> Transcript:
        duration_ms = int(1000 * audio.size / SAMPLE_RATE)
        body, ctype = _multipart(
            {"language": language, "model": self.model, "response_format": "json"},
            "frase.wav",
            encode_wav(audio.astype(np.float32), SAMPLE_RATE),
        )
        try:
            data = self._request("/v1/audio/transcriptions", body, ctype)
        except ConnectionError as exc:
            self._error = str(exc)
            if self._fallback is not None:
                log.warning("transcripción remota fallida (%s); se usa el modelo local", exc)
                return self._fallback.transcribe(audio, language)  # type: ignore[attr-defined]
            raise
        self._error = None
        text = str(data.get("text", "")).strip()
        return Transcript(text=text, language=str(data.get("language") or language), duration_ms=duration_ms)

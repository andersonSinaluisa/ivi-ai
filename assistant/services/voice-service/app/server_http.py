"""Llamadas HTTP al servidor (``<servidor>/stt`` y ``<servidor>/tts``) con la credencial del equipo."""

from __future__ import annotations

import json
import secrets
import urllib.error
import urllib.request
from collections.abc import Callable

from .cloud import CloudCredential

Opener = Callable[..., object]


class ServerError(ConnectionError):
    """Fallo al hablar con el servidor. ``auth`` = la credencial caducó o fue revocada."""

    def __init__(self, message: str, *, auth: bool = False) -> None:
        super().__init__(message)
        self.auth = auth


def multipart(fields: dict[str, str], file_field: str, file_name: str, file_bytes: bytes, content_type: str) -> tuple[bytes, str]:
    boundary = "jarvis" + secrets.token_hex(12)
    out = bytearray()
    for key, value in fields.items():
        out += f'--{boundary}\r\nContent-Disposition: form-data; name="{key}"\r\n\r\n{value}\r\n'.encode()
    out += (
        f'--{boundary}\r\nContent-Disposition: form-data; name="{file_field}"; filename="{file_name}"\r\n'
        f"Content-Type: {content_type}\r\n\r\n"
    ).encode()
    out += file_bytes + f"\r\n--{boundary}--\r\n".encode()
    return bytes(out), f"multipart/form-data; boundary={boundary}"


class ServerClient:
    def __init__(self, credential: CloudCredential, prefix: str, timeout_s: float, opener: Opener = urllib.request.urlopen) -> None:
        self.credential = credential
        self.prefix = prefix
        self.timeout_s = timeout_s
        self._open = opener

    def base(self) -> str:
        value = self.credential.get()
        if value is None:
            raise ServerError("este equipo no está conectado a tu servidor (empareja la PC en Configuración → Infraestructura)", auth=True)
        return value.server_url + self.prefix

    def call(self, path: str, data: bytes | None = None, content_type: str | None = None) -> tuple[bytes, str]:
        """Devuelve (cuerpo, content-type). Lanza ``ServerError`` con un mensaje para el usuario."""
        value = self.credential.get()
        url = self.base() + path
        req = urllib.request.Request(url, data=data, method="POST" if data is not None else "GET")
        if value is not None:
            req.add_header("Authorization", f"Bearer {value.token}")
        if content_type:
            req.add_header("Content-Type", content_type)
        req.add_header("User-Agent", "jarvis-voice-service")
        try:
            with self._open(req, timeout=self.timeout_s) as resp:  # type: ignore[attr-defined]
                return resp.read(), resp.headers.get("Content-Type", "")
        except urllib.error.HTTPError as exc:
            if exc.code in (401, 403):
                raise ServerError("el servidor rechazó la credencial de este equipo (se renovará sola)", auth=True) from exc
            if exc.code in (502, 503, 504):
                raise ServerError(f"el servicio {self.prefix} no está disponible en el servidor ({exc.code})") from exc
            body = exc.read().decode("utf-8", "replace")[:200]
            raise ServerError(f"el servidor respondió {exc.code}: {body}") from exc
        except (urllib.error.URLError, TimeoutError, OSError) as exc:
            reason = getattr(exc, "reason", exc)
            raise ServerError(f"no se pudo conectar con el servidor: {reason}") from exc

    def json(self, path: str, data: bytes | None = None, content_type: str | None = None) -> dict:
        body, _ = self.call(path, data, content_type)
        try:
            return json.loads(body.decode("utf-8"))
        except ValueError as exc:
            raise ServerError("respuesta inválida del servidor") from exc

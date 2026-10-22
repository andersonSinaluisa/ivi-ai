"""Credencial con la que el servicio de voz habla con tu servidor (transcripción y voz).

La app de escritorio la entrega por ``PUT /cloud`` (URL del servidor emparejado + token de acceso del device, que
caduca en minutos) y la renueva antes de que caduque. Solo vive en memoria: nunca se escribe a disco.
"""

from __future__ import annotations

import threading
from dataclasses import dataclass
from urllib.parse import urlparse


def validate_url(url: str) -> str:
    """HTTPS obligatorio salvo hacia esta misma máquina (pruebas o un servidor local)."""
    parsed = urlparse(url.strip())
    host = (parsed.hostname or "").lower()
    if not host:
        raise ValueError("URL inválida")
    loopback = host in {"localhost", "127.0.0.1", "::1"}
    if parsed.scheme != "https" and not (parsed.scheme == "http" and loopback):
        raise ValueError("el servidor debe usar HTTPS (HTTP solo hacia localhost)")
    return url.strip().rstrip("/")


@dataclass(frozen=True)
class Credential:
    server_url: str
    token: str


class CloudCredential:
    def __init__(self) -> None:
        self._value: Credential | None = None
        self._lock = threading.Lock()

    def set(self, server_url: str, token: str) -> Credential:
        if not token.strip():
            raise ValueError("falta el token")
        value = Credential(validate_url(server_url), token.strip())
        with self._lock:
            self._value = value
        return value

    def clear(self) -> None:
        with self._lock:
            self._value = None

    def get(self) -> Credential | None:
        with self._lock:
            return self._value

    @property
    def server_url(self) -> str | None:
        value = self.get()
        return value.server_url if value else None

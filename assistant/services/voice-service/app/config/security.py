"""Token local compartido con la app de escritorio.

El servicio crea ``<data_dir>/voice-token`` la primera vez (32 bytes aleatorios) y lo reutiliza.
Ese archivo está en el perfil del usuario: otros usuarios de la PC no lo pueden leer.
"""

from __future__ import annotations

import os
import secrets
from pathlib import Path

TOKEN_FILE = "voice-token"


def ensure_token(base: Path) -> str:
    if env := os.environ.get("JARVIS_VOICE_TOKEN"):
        return env
    path = base / TOKEN_FILE
    if path.is_file():
        token = path.read_text(encoding="utf-8").strip()
        if len(token) >= 32:
            return token
    base.mkdir(parents=True, exist_ok=True)
    token = secrets.token_urlsafe(32)
    path.write_text(token, encoding="utf-8")
    try:
        os.chmod(path, 0o600)
    except OSError:
        pass
    return token


def token_matches(expected: str, given: str | None) -> bool:
    return bool(given) and secrets.compare_digest(expected.encode(), str(given).encode())

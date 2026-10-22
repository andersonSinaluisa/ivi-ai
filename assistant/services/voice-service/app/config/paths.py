"""Rutas compartidas con la app de escritorio (mismo directorio que usa Rust)."""

from __future__ import annotations

import os
import sys
from pathlib import Path


def data_dir() -> Path:
    """``%LOCALAPPDATA%\\Jarvis`` en Windows, ``~/.local/share/Jarvis`` en Linux."""
    if env := os.environ.get("JARVIS_DATA_DIR"):
        return Path(env)
    if sys.platform == "win32" and (local := os.environ.get("LOCALAPPDATA")):
        return Path(local) / "Jarvis"
    xdg = os.environ.get("XDG_DATA_HOME")
    return (Path(xdg) if xdg else Path.home() / ".local" / "share") / "Jarvis"


def service_root() -> Path:
    """Carpeta ``services/voice-service``."""
    return Path(__file__).resolve().parents[2]

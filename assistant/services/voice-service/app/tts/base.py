"""Interfaz TTSProvider, cadena de respaldo, caché y troceado en frases."""

from __future__ import annotations

import hashlib
import logging
import re
from dataclasses import dataclass
from pathlib import Path
from typing import Protocol

import numpy as np

from ..audio.signal import decode_wav, encode_wav

log = logging.getLogger(__name__)


class TtsError(RuntimeError):
    pass


@dataclass
class VoiceRef:
    """Voz de referencia activa (perfil clonado)."""

    id: str
    reference_wav: Path
    cache_dir: Path


@dataclass
class Speech:
    samples: np.ndarray  # float32 mono
    sample_rate: int
    provider: str


class TtsProvider(Protocol):
    name: str
    supports_cloning: bool

    @property
    def ready(self) -> bool: ...

    @property
    def detail(self) -> str: ...

    def synthesize(self, text: str, voice: VoiceRef | None, speed: float) -> Speech: ...


class FallbackTts:
    """Usa ``primary``; si falla (o no hay voz clonada activa), usa ``fallback``."""

    def __init__(self, primary: TtsProvider, fallback: TtsProvider | None) -> None:
        self.primary = primary
        self.fallback = fallback if fallback is not None and fallback is not primary else None
        self.name = primary.name
        self.supports_cloning = primary.supports_cloning
        self.last_error: str | None = None

    @property
    def ready(self) -> bool:
        return self.primary.ready or bool(self.fallback and self.fallback.ready)

    @property
    def detail(self) -> str:
        d = f"{self.primary.name}: {self.primary.detail}"
        if self.fallback:
            d += f" · respaldo {self.fallback.name}: {self.fallback.detail}"
        return d

    def synthesize(self, text: str, voice: VoiceRef | None, speed: float) -> Speech:
        try:
            if self.primary.supports_cloning and voice is None:
                raise TtsError("no hay una voz clonada activa")
            return self.primary.synthesize(text, voice, speed)
        except Exception as exc:  # noqa: BLE001
            self.last_error = str(exc)
            if not self.fallback:
                raise
            log.warning("TTS %s falló (%s); uso %s", self.primary.name, exc, self.fallback.name)
            return self.fallback.synthesize(text, None, speed)


class TtsCache:
    """Caché en disco de audios sintetizados, con límite de tamaño (se borran los más antiguos)."""

    def __init__(self, directory: Path, max_mb: int) -> None:
        self.dir = directory
        self.max_bytes = max_mb * 1024 * 1024
        self.hits = 0

    @staticmethod
    def key(provider: str, voice_id: str | None, speed: float, text: str) -> str:
        raw = f"{provider}|{voice_id or '-'}|{speed:.2f}|{text}".encode()
        return hashlib.sha256(raw).hexdigest()

    def get(self, key: str) -> Speech | None:
        path = self.dir / f"{key}.wav"
        if not path.is_file():
            return None
        try:
            samples, rate = decode_wav(path.read_bytes())
        except Exception:  # noqa: BLE001
            path.unlink(missing_ok=True)
            return None
        path.touch()
        self.hits += 1
        return Speech(samples, rate, "cache")

    def put(self, key: str, speech: Speech) -> None:
        if self.max_bytes <= 0:
            return
        self.dir.mkdir(parents=True, exist_ok=True)
        (self.dir / f"{key}.wav").write_bytes(encode_wav(speech.samples, speech.sample_rate))
        self._evict()

    def _evict(self) -> None:
        files = sorted(self.dir.glob("*.wav"), key=lambda p: p.stat().st_mtime)
        total = sum(p.stat().st_size for p in files)
        while files and total > self.max_bytes:
            oldest = files.pop(0)
            total -= oldest.stat().st_size
            oldest.unlink(missing_ok=True)


_SENTENCE = re.compile(r"(?<=[.!?¿¡…:;])\s+")


def split_sentences(text: str, max_chars: int = 220) -> list[str]:
    """Trocea en frases para empezar a hablar antes de sintetizar todo."""
    parts: list[str] = []
    for sentence in _SENTENCE.split(text.strip()):
        sentence = sentence.strip()
        while len(sentence) > max_chars:
            cut = sentence.rfind(" ", 0, max_chars)
            cut = cut if cut > 40 else max_chars
            parts.append(sentence[:cut].strip())
            sentence = sentence[cut:].strip()
        if sentence:
            parts.append(sentence)
    return parts


class SilentTts:
    """TTS de pruebas: un tono corto por cada carácter/50."""

    name = "silent"
    supports_cloning = False

    def __init__(self, fail: bool = False) -> None:
        self.fail = fail
        self.calls: list[str] = []

    @property
    def ready(self) -> bool:
        return True

    @property
    def detail(self) -> str:
        return "sin sonido"

    def synthesize(self, text: str, voice: VoiceRef | None, speed: float) -> Speech:  # noqa: ARG002
        self.calls.append(text)
        if self.fail:
            raise TtsError("fallo simulado")
        n = max(160, len(text) * 160)
        return Speech(np.zeros(n, dtype=np.float32), 16_000, self.name)

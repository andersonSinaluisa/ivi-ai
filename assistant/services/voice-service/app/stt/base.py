"""Contrato de transcripción (Speech-to-Text)."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Protocol

import numpy as np


@dataclass
class Transcript:
    text: str
    language: str
    duration_ms: int


class SttProvider(Protocol):
    name: str

    @property
    def ready(self) -> bool: ...

    @property
    def detail(self) -> str: ...

    def transcribe(self, audio: np.ndarray, language: str) -> Transcript:
        """``audio``: float32 mono a 16 kHz."""
        ...


class ScriptedStt:
    """STT de pruebas."""

    name = "scripted"

    def __init__(self, texts: list[str]) -> None:
        self._texts = list(texts)
        self.calls: list[np.ndarray] = []

    @property
    def ready(self) -> bool:
        return True

    @property
    def detail(self) -> str:
        return "scripted"

    def transcribe(self, audio: np.ndarray, language: str) -> Transcript:
        self.calls.append(audio)
        text = self._texts.pop(0) if self._texts else ""
        return Transcript(text=text, language=language, duration_ms=int(1000 * audio.size / 16_000))

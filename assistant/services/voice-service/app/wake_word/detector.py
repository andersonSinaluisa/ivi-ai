"""Detectores de palabra de activación.

Situación real (verificada en octubre de 2026):
- openWakeWord 0.6 trae un modelo preentrenado **"hey jarvis"** (no "Jarvis" a secas).
- Sus modelos preentrenados son solo para inglés y tienen licencia CC BY-NC-SA 4.0 (no comercial).
- En Windows solo funciona con onnxruntime.

Por eso el detector viene **desactivado** por defecto y Jarvis funciona con pulsar-para-hablar.
Para activarlo: ``wake_word_enabled=true`` y un modelo (``hey_jarvis`` o la ruta de un ``.onnx``
entrenado para "Jarvis" en español). Ver docs/voice-wake-word.md.
"""

from __future__ import annotations

import logging
from pathlib import Path
from typing import Protocol

import numpy as np

log = logging.getLogger(__name__)


class WakeWordDetector(Protocol):
    available: bool
    detail: str
    model_name: str | None

    def process(self, frame: np.ndarray) -> float:
        """Puntuación 0..1 para un bloque de 80 ms (int16, 16 kHz)."""
        ...

    def reset(self) -> None: ...


class DisabledDetector:
    def __init__(self, detail: str = "desactivada: usa pulsar para hablar") -> None:
        self.available = False
        self.detail = detail
        self.model_name = None

    def process(self, frame: np.ndarray) -> float:  # noqa: ARG002
        return 0.0

    def reset(self) -> None:
        pass


class OpenWakeWordDetector:
    def __init__(self, model: str) -> None:
        self.model_name = model
        self.available = False
        self.detail = ""
        self._model = None
        self._key: str | None = None
        try:
            from openwakeword.model import Model  # type: ignore[import-not-found]
        except Exception as exc:  # noqa: BLE001
            self.detail = f"openwakeword no instalado ({exc}). Instala el extra [wakeword]."
            return
        try:
            path = Path(model)
            models = [str(path)] if path.suffix in {".onnx", ".tflite"} else [model]
            self._model = Model(wakeword_models=models, inference_framework="onnx")
            self._key = next(iter(self._model.models.keys()))
            self.available = True
            self.detail = f"modelo '{self._key}' cargado (solo inglés si es preentrenado)"
        except Exception as exc:  # noqa: BLE001
            self.detail = (
                f"no se pudo cargar el modelo '{model}': {exc}. "
                "Descárgalo con scripts/install-wakeword.ps1 o indica la ruta de un .onnx."
            )
            log.warning(self.detail)

    def process(self, frame: np.ndarray) -> float:
        if not self._model or not self._key:
            return 0.0
        scores = self._model.predict(frame)
        return float(scores.get(self._key, 0.0))

    def reset(self) -> None:
        if self._model is not None:
            self._model.reset()


class ScriptedDetector:
    """Detector de pruebas: devuelve las puntuaciones dadas, en orden."""

    def __init__(self, scores: list[float]) -> None:
        self.available = True
        self.detail = "scripted"
        self.model_name = "scripted"
        self._scores = list(scores)
        self.calls = 0

    def process(self, frame: np.ndarray) -> float:  # noqa: ARG002
        self.calls += 1
        return self._scores.pop(0) if self._scores else 0.0

    def reset(self) -> None:
        pass


def build_detector(enabled: bool, model: str) -> WakeWordDetector:
    if not enabled:
        return DisabledDetector()
    return OpenWakeWordDetector(model)

"""Speech-to-Text con faster-whisper (CTranslate2), 100 % local."""

from __future__ import annotations

import logging
import threading
from collections.abc import Callable
from dataclasses import dataclass
from typing import Any, Protocol

import numpy as np

from ..config.paths import service_root
from ..config.settings import SAMPLE_RATE

log = logging.getLogger(__name__)


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


def _default_factory(model: str, device: str, compute_type: str) -> Any:  # pragma: no cover - modelo real
    from faster_whisper import WhisperModel  # type: ignore[import-not-found]

    if device == "auto":
        try:
            import ctranslate2  # type: ignore[import-not-found]

            device = "cuda" if ctranslate2.get_cuda_device_count() > 0 else "cpu"
        except Exception:  # noqa: BLE001
            device = "cpu"
    if compute_type == "auto":
        compute_type = "float16" if device == "cuda" else "int8"
    root = service_root() / "models" / "whisper"
    log.info("cargando faster-whisper %s en %s (%s)", model, device, compute_type)
    return WhisperModel(model, device=device, compute_type=compute_type, download_root=str(root))


class FasterWhisperStt:
    name = "faster-whisper"

    def __init__(
        self,
        model: str,
        device: str = "auto",
        compute_type: str = "auto",
        factory: Callable[[str, str, str], Any] = _default_factory,
    ) -> None:
        self.model_name = model
        self._args = (model, device, compute_type)
        self._factory = factory
        self._model: Any = None
        self._error: str | None = None
        self._lock = threading.Lock()

    @property
    def ready(self) -> bool:
        return self._model is not None

    @property
    def detail(self) -> str:
        if self._error:
            return self._error
        return f"modelo {self.model_name} listo" if self.ready else f"modelo {self.model_name} sin cargar"

    def load(self) -> None:
        """Carga el modelo (la primera vez lo descarga a services/voice-service/models/whisper)."""
        with self._lock:
            if self._model is not None:
                return
            try:
                self._model = self._factory(*self._args)
                self._error = None
            except Exception as exc:  # noqa: BLE001
                self._error = f"no se pudo cargar faster-whisper ({exc}). Instala el extra [stt]."
                log.error(self._error)
                raise

    def transcribe(self, audio: np.ndarray, language: str) -> Transcript:
        if self._model is None:
            self.load()
        segments, info = self._model.transcribe(
            audio.astype(np.float32),
            language=language,
            beam_size=1,
            vad_filter=True,
            condition_on_previous_text=False,
        )
        text = " ".join(s.text.strip() for s in segments).strip()
        return Transcript(
            text=text,
            language=getattr(info, "language", language) or language,
            duration_ms=int(1000 * audio.size / SAMPLE_RATE),
        )


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
        return Transcript(text=text, language=language, duration_ms=int(1000 * audio.size / SAMPLE_RATE))

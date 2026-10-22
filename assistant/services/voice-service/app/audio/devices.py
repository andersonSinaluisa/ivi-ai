"""Entrada y salida de audio. ``sounddevice`` (PortAudio) es opcional: sin él, el servicio arranca
igual y reporta que no hay audio, para que la UI lo muestre sin bloquear nada."""

from __future__ import annotations

import logging
import threading
from collections.abc import Callable
from typing import Protocol

import numpy as np

from ..config.settings import FRAME_SAMPLES, SAMPLE_RATE

log = logging.getLogger(__name__)

try:  # pragma: no cover - depende del sistema
    import sounddevice as sd  # type: ignore[import-not-found]
except Exception as exc:  # noqa: BLE001
    sd = None
    _IMPORT_ERROR = str(exc)
else:
    _IMPORT_ERROR = ""


def audio_available() -> tuple[bool, str]:
    if sd is None:
        return False, f"sounddevice no disponible ({_IMPORT_ERROR}). Instala el extra [audio]."
    return True, "PortAudio OK"


def list_devices() -> list[dict]:
    if sd is None:
        return []
    default_in, default_out = sd.default.device if sd.default.device else (None, None)
    out = []
    for i, d in enumerate(sd.query_devices()):
        out.append(
            {
                "index": i,
                "name": d["name"],
                "inputs": int(d["max_input_channels"]),
                "outputs": int(d["max_output_channels"]),
                "isDefaultInput": i == default_in,
                "isDefaultOutput": i == default_out,
            }
        )
    return out


class AudioInput(Protocol):
    def start(self, on_frame: Callable[[np.ndarray], None]) -> None: ...
    def stop(self) -> None: ...


class AudioOutput(Protocol):
    def play(self, samples: np.ndarray, sample_rate: int, volume: float, stop: threading.Event) -> bool:
        """Reproduce bloqueando hasta terminar. Devuelve ``False`` si se interrumpió."""
        ...


class SoundDeviceInput:
    """Micrófono a 16 kHz mono en bloques de 80 ms."""

    def __init__(self, device: int | None) -> None:
        self.device = device
        self._stream = None

    def start(self, on_frame: Callable[[np.ndarray], None]) -> None:
        if sd is None:
            raise RuntimeError(audio_available()[1])

        def callback(indata, _frames, _time, status):  # noqa: ANN001
            if status:
                log.debug("audio input status: %s", status)
            on_frame(indata[:, 0].copy())

        self._stream = sd.InputStream(
            samplerate=SAMPLE_RATE,
            channels=1,
            dtype="int16",
            blocksize=FRAME_SAMPLES,
            device=self.device,
            callback=callback,
        )
        self._stream.start()

    def stop(self) -> None:
        if self._stream is not None:
            self._stream.stop()
            self._stream.close()
            self._stream = None


class SoundDeviceOutput:
    def __init__(self, device: int | None) -> None:
        self.device = device

    def play(self, samples: np.ndarray, sample_rate: int, volume: float, stop: threading.Event) -> bool:
        if sd is None:
            raise RuntimeError(audio_available()[1])
        data = (samples.astype(np.float32) * float(volume)).reshape(-1, 1)
        block = max(256, sample_rate // 20)  # 50 ms: permite interrumpir rápido
        with sd.OutputStream(samplerate=sample_rate, channels=1, dtype="float32", device=self.device) as stream:
            for i in range(0, len(data), block):
                if stop.is_set():
                    return False
                stream.write(data[i : i + block])
        return True


class NullOutput:
    """Salida silenciosa: registra lo que se habría reproducido (pruebas o sin altavoz)."""

    def __init__(self) -> None:
        self.played: list[tuple[np.ndarray, int]] = []

    def play(self, samples: np.ndarray, sample_rate: int, volume: float, stop: threading.Event) -> bool:  # noqa: ARG002
        if stop.is_set():
            return False
        self.played.append((samples, sample_rate))
        return True


class ManualInput:
    """Entrada controlada por código (pruebas): ``push(frame)`` entrega un bloque."""

    def __init__(self) -> None:
        self._cb: Callable[[np.ndarray], None] | None = None

    def start(self, on_frame: Callable[[np.ndarray], None]) -> None:
        self._cb = on_frame

    def stop(self) -> None:
        self._cb = None

    def push(self, frame: np.ndarray) -> None:
        if self._cb:
            self._cb(frame)


def record_blocking(seconds: float, device: int | None) -> np.ndarray:
    """Graba una muestra (para perfiles de voz). Devuelve int16 a 16 kHz."""
    if sd is None:
        raise RuntimeError(audio_available()[1])
    data = sd.rec(int(seconds * SAMPLE_RATE), samplerate=SAMPLE_RATE, channels=1, dtype="int16", device=device)
    sd.wait()
    return data[:, 0].copy()

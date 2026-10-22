"""Detección de actividad de voz por energía con piso de ruido adaptativo.

Decide cuándo empieza y termina la instrucción en tiempo real. Es deliberadamente simple: no
depende de modelos y se adapta al ruido de la habitación. El servidor (faster-whisper) aplica además
su propio VAD (Silero) al transcribir, que limpia los silencios internos.
"""

from __future__ import annotations

from dataclasses import dataclass

import numpy as np

from .signal import rms_int16


@dataclass
class VadResult:
    is_speech: bool
    started: bool
    ended: bool


class EnergyVad:
    def __init__(
        self,
        frame_ms: int,
        min_rms: float = 300.0,
        noise_ratio: float = 2.5,
        end_silence_ms: int = 900,
        start_frames: int = 2,
    ) -> None:
        self.frame_ms = frame_ms
        self.min_rms = min_rms
        self.noise_ratio = noise_ratio
        self.end_silence_frames = max(1, end_silence_ms // frame_ms)
        self.start_frames = start_frames
        self.noise_floor = min_rms / noise_ratio
        self.reset()

    def reset(self) -> None:
        self.speech_started = False
        self._voiced_run = 0
        self._silence_run = 0

    @property
    def threshold(self) -> float:
        return max(self.min_rms, self.noise_floor * self.noise_ratio)

    def calibrate(self, frame: np.ndarray) -> None:
        """Ajusta el piso de ruido con audio sin voz (p. ej. mientras espera la palabra clave)."""
        self.noise_floor = 0.9 * self.noise_floor + 0.1 * rms_int16(frame)

    def update(self, frame: np.ndarray) -> VadResult:
        energy = rms_int16(frame)
        voiced = energy >= self.threshold
        started = ended = False
        if voiced:
            self._voiced_run += 1
            self._silence_run = 0
            if not self.speech_started and self._voiced_run >= self.start_frames:
                self.speech_started = True
                started = True
        else:
            self._voiced_run = 0
            self._silence_run += 1
            # Solo se aprende el ruido en silencio, para no "aprender" la propia voz.
            self.calibrate(frame)
            if self.speech_started and self._silence_run >= self.end_silence_frames:
                ended = True
        return VadResult(is_speech=voiced, started=started, ended=ended)

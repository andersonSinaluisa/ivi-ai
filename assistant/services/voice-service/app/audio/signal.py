"""Utilidades de señal: niveles, WAV y la señal de activación."""

from __future__ import annotations

import io
import wave

import numpy as np

from ..config.settings import SAMPLE_RATE


def rms_int16(frame: np.ndarray) -> float:
    if frame.size == 0:
        return 0.0
    f = frame.astype(np.float32)
    return float(np.sqrt(np.mean(f * f)))


def level(frame: np.ndarray) -> tuple[float, float]:
    """(rms, pico) normalizados a 0..1 para la visualización."""
    if frame.size == 0:
        return 0.0, 0.0
    rms = min(1.0, rms_int16(frame) / 8_000.0)
    peak = min(1.0, float(np.max(np.abs(frame.astype(np.int32)))) / 32_768.0)
    return round(rms, 3), round(peak, 3)


def int16_to_float(samples: np.ndarray) -> np.ndarray:
    return (samples.astype(np.float32) / 32_768.0).clip(-1.0, 1.0)


def float_to_int16(samples: np.ndarray) -> np.ndarray:
    return (np.clip(samples, -1.0, 1.0) * 32_767.0).astype(np.int16)


def encode_wav(samples: np.ndarray, sample_rate: int) -> bytes:
    """WAV PCM 16 bits mono."""
    pcm = samples if samples.dtype == np.int16 else float_to_int16(samples)
    buf = io.BytesIO()
    with wave.open(buf, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(sample_rate)
        w.writeframes(pcm.tobytes())
    return buf.getvalue()


def decode_wav(data: bytes) -> tuple[np.ndarray, int]:
    """Devuelve audio float32 mono y su frecuencia. Acepta PCM de 8, 16 o 32 bits."""
    with wave.open(io.BytesIO(data), "rb") as w:
        channels, width, rate, n = w.getnchannels(), w.getsampwidth(), w.getframerate(), w.getnframes()
        raw = w.readframes(n)
    if width == 2:
        x = np.frombuffer(raw, dtype=np.int16).astype(np.float32) / 32_768.0
    elif width == 4:
        x = np.frombuffer(raw, dtype=np.int32).astype(np.float32) / 2_147_483_648.0
    elif width == 1:
        x = (np.frombuffer(raw, dtype=np.uint8).astype(np.float32) - 128.0) / 128.0
    else:
        raise ValueError(f"WAV de {width * 8} bits no soportado")
    if channels > 1:
        x = x.reshape(-1, channels).mean(axis=1)
    return x.astype(np.float32), rate


def resample(x: np.ndarray, src: int, dst: int) -> np.ndarray:
    """Remuestreo lineal (suficiente para voz)."""
    if src == dst or x.size == 0:
        return x
    n = int(round(x.size * dst / src))
    return np.interp(np.linspace(0, x.size - 1, n), np.arange(x.size), x).astype(np.float32)


def chime(sample_rate: int = SAMPLE_RATE) -> np.ndarray:
    """Dos tonos cortos ascendentes (~180 ms) para indicar que Jarvis escucha."""
    def tone(freq: float, ms: int) -> np.ndarray:
        t = np.arange(int(sample_rate * ms / 1000)) / sample_rate
        env = np.minimum(1.0, np.minimum(t, t[::-1]) * 60)
        return (0.25 * np.sin(2 * np.pi * freq * t) * env).astype(np.float32)

    return np.concatenate([tone(660, 80), np.zeros(int(sample_rate * 0.02), np.float32), tone(990, 80)])

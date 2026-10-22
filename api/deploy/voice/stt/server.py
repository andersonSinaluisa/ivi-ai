"""jarvis-stt: transcripción con faster-whisper para el servicio de voz del escritorio.

No forma parte del Cloud Core (ADR-010: el Cloud Core no ejecuta modelos). Es un servicio aparte del owner, como
Ollama: escucha solo en 127.0.0.1 y nginx lo publica en /stt/, exigiendo la credencial de Jarvis (auth_request a
/api/v1/access/check).

API compatible con OpenAI (``POST /v1/audio/transcriptions``, multipart con ``file`` y ``language`` opcional) para
que el cliente también funcione contra otros servidores compatibles. Solo acepta WAV PCM (lo que envía el escritorio):
no hace falta ffmpeg y no se procesan formatos arbitrarios.
"""

from __future__ import annotations

import asyncio
import io
import logging
import os
import threading
import time
import wave
from collections.abc import AsyncIterator, Callable
from contextlib import asynccontextmanager
from typing import Any

import numpy as np
from fastapi import FastAPI, File, Form, HTTPException, UploadFile

log = logging.getLogger("jarvis-stt")
logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(name)s: %(message)s")

SAMPLE_RATE = 16_000
MAX_BYTES = int(os.environ.get("STT_MAX_BYTES", str(10 * 1024 * 1024)))  # ~5 min de audio a 16 kHz
MAX_SECONDS = int(os.environ.get("STT_MAX_SECONDS", "120"))


def decode_wav(data: bytes) -> np.ndarray:
    """WAV PCM (8/16/32 bits, mono o estéreo) → float32 mono a 16 kHz."""
    with wave.open(io.BytesIO(data), "rb") as w:
        channels, width, rate, n = w.getnchannels(), w.getsampwidth(), w.getframerate(), w.getnframes()
        if w.getcomptype() != "NONE":
            raise ValueError("solo WAV PCM sin comprimir")
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
    if rate != SAMPLE_RATE:
        # Remuestreo lineal: suficiente para voz (el escritorio ya envía 16 kHz).
        target = int(round(x.size * SAMPLE_RATE / rate))
        x = np.interp(np.linspace(0, x.size - 1, target), np.arange(x.size), x) if x.size else x
    return x.astype(np.float32)


def _load_model(name: str) -> Any:  # pragma: no cover - modelo real
    from faster_whisper import WhisperModel

    device = os.environ.get("WHISPER_DEVICE", "cpu")
    compute = os.environ.get("WHISPER_COMPUTE_TYPE", "int8")
    root = os.environ.get("WHISPER_CACHE", "/models")
    log.info("cargando faster-whisper %s en %s (%s)", name, device, compute)
    return WhisperModel(name, device=device, compute_type=compute, download_root=root,
                        cpu_threads=int(os.environ.get("WHISPER_THREADS", "0")))


class Transcriber:
    """Un modelo cargado una vez; las peticiones se atienden de una en una (CPU limitada)."""

    def __init__(self, model: str, loader: Callable[[str], Any] = _load_model) -> None:
        self.model_name = model
        self._loader = loader
        self._model: Any = None
        self._error: str | None = None
        self._load_lock = threading.Lock()
        self._run = asyncio.Semaphore(1)

    def load(self) -> None:
        with self._load_lock:
            if self._model is None:
                try:
                    self._model = self._loader(self.model_name)
                    self._error = None
                except Exception as exc:  # noqa: BLE001
                    self._error = str(exc)
                    raise

    @property
    def status(self) -> dict:
        return {"model": self.model_name, "ready": self._model is not None, "error": self._error}

    def _transcribe(self, audio: np.ndarray, language: str | None) -> tuple[str, str]:
        self.load()
        segments, info = self._model.transcribe(
            audio, language=language or None, beam_size=1, vad_filter=True, condition_on_previous_text=False
        )
        text = " ".join(s.text.strip() for s in segments).strip()
        return text, getattr(info, "language", language) or (language or "")

    async def transcribe(self, audio: np.ndarray, language: str | None) -> tuple[str, str]:
        async with self._run:
            return await asyncio.to_thread(self._transcribe, audio, language)


def create_app(transcriber: Transcriber | None = None) -> FastAPI:
    stt = transcriber or Transcriber(os.environ.get("WHISPER_MODEL", "base"))

    @asynccontextmanager
    async def lifespan(_: FastAPI) -> AsyncIterator[None]:
        # Carga el modelo al arrancar para que la primera frase no espere (y el healthcheck lo refleje).
        if os.environ.get("STT_PRELOAD", "1") == "1":
            try:
                await asyncio.to_thread(stt.load)
            except Exception:  # noqa: BLE001
                log.exception("no se pudo cargar el modelo")
        yield

    app = FastAPI(title="jarvis-stt", docs_url=None, redoc_url=None, openapi_url=None, lifespan=lifespan)

    @app.get("/health")
    def health() -> dict:
        return {"status": "ok", **stt.status}

    @app.post("/v1/audio/transcriptions")
    async def transcriptions(
        file: UploadFile = File(...),  # noqa: B008
        language: str | None = Form(None),  # noqa: B008
        model: str | None = Form(None),  # noqa: ARG001,B008 - compatibilidad OpenAI: se usa el modelo del servidor
        response_format: str | None = Form(None),  # noqa: ARG001,B008
    ) -> dict:
        data = await file.read(MAX_BYTES + 1)
        if len(data) > MAX_BYTES:
            raise HTTPException(status_code=413, detail="audio demasiado largo")
        if language is not None and (len(language) > 8 or not language.replace("-", "").isalpha()):
            raise HTTPException(status_code=422, detail="idioma inválido")
        try:
            audio = decode_wav(data)
        except Exception as exc:  # noqa: BLE001
            raise HTTPException(status_code=415, detail=f"se esperaba WAV PCM: {exc}") from exc
        seconds = audio.size / SAMPLE_RATE
        if seconds > MAX_SECONDS:
            raise HTTPException(status_code=413, detail=f"audio de {seconds:.0f} s (máximo {MAX_SECONDS} s)")
        started = time.perf_counter()
        try:
            text, lang = await stt.transcribe(audio, language)
        except Exception as exc:  # noqa: BLE001
            log.exception("transcripción fallida")
            raise HTTPException(status_code=503, detail=f"modelo no disponible: {exc}") from exc
        elapsed = time.perf_counter() - started
        log.info("transcrito %.1f s de audio en %.2f s", seconds, elapsed)
        return {"text": text, "language": lang, "duration": round(seconds, 2)}

    return app


app = create_app() if os.environ.get("JARVIS_STT_NO_APP") != "1" else None

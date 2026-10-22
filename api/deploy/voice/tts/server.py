"""jarvis-tts: síntesis de voz con Piper (CPU) para el servicio de voz del escritorio.

Servicio aparte del Cloud Core (ADR-010), como Ollama y jarvis-stt: escucha solo en 127.0.0.1 y nginx lo publica en
/tts/ exigiendo la credencial de Jarvis. API compatible con OpenAI (``POST /v1/audio/speech``) que devuelve WAV PCM.

La voz (``PIPER_VOICE``, por defecto ``es_MX-claude-high``) se descarga una vez en ``/models`` al arrancar.
Piper es GPL-3.0; las voces tienen su propia licencia (ver la ficha de cada voz en rhasspy/piper-voices).
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
from pathlib import Path
from typing import Any

from fastapi import FastAPI, HTTPException
from fastapi.responses import Response
from pydantic import BaseModel, Field

log = logging.getLogger("jarvis-tts")
logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(name)s: %(message)s")

MAX_CHARS = int(os.environ.get("TTS_MAX_CHARS", "4000"))


def _load_voice(name: str) -> Any:  # pragma: no cover - voz real
    from piper import PiperVoice
    from piper.download_voices import download_voice

    root = Path(os.environ.get("PIPER_DIR", "/models"))
    model = root / f"{name}.onnx"
    if not model.is_file():
        log.info("descargando la voz %s en %s", name, root)
        root.mkdir(parents=True, exist_ok=True)
        download_voice(name, root)
    log.info("cargando la voz %s", name)
    return PiperVoice.load(model)


def _synthesize_wav(voice: Any, text: str, speed: float) -> bytes:  # pragma: no cover - voz real
    from piper import SynthesisConfig

    buf = io.BytesIO()
    with wave.open(buf, "wb") as w:
        voice.synthesize_wav(text, w, syn_config=SynthesisConfig(length_scale=1.0 / speed))
    return buf.getvalue()


class Speaker:
    """Una voz cargada una vez; las peticiones se atienden de una en una (CPU limitada)."""

    def __init__(
        self,
        voice: str,
        loader: Callable[[str], Any] = _load_voice,
        synth: Callable[[Any, str, float], bytes] = _synthesize_wav,
    ) -> None:
        self.voice_name = voice
        self._loader = loader
        self._synth = synth
        self._voice: Any = None
        self._error: str | None = None
        self._load_lock = threading.Lock()
        self._run = asyncio.Semaphore(1)

    def load(self) -> None:
        with self._load_lock:
            if self._voice is None:
                try:
                    self._voice = self._loader(self.voice_name)
                    self._error = None
                except Exception as exc:  # noqa: BLE001
                    self._error = str(exc)
                    raise

    @property
    def status(self) -> dict:
        return {"voice": self.voice_name, "ready": self._voice is not None, "error": self._error}

    def _speak(self, text: str, speed: float) -> bytes:
        self.load()
        return self._synth(self._voice, text, speed)

    async def speak(self, text: str, speed: float) -> bytes:
        async with self._run:
            return await asyncio.to_thread(self._speak, text, speed)


class SpeechRequest(BaseModel):
    input: str = Field(min_length=1)
    model: str | None = None  # compatibilidad OpenAI: se usa la voz del servidor
    voice: str | None = None
    response_format: str = "wav"
    speed: float = Field(1.0, ge=0.5, le=2.0)


def create_app(speaker: Speaker | None = None) -> FastAPI:
    tts = speaker or Speaker(os.environ.get("PIPER_VOICE", "es_MX-claude-high"))

    @asynccontextmanager
    async def lifespan(_: FastAPI) -> AsyncIterator[None]:
        if os.environ.get("TTS_PRELOAD", "1") == "1":
            try:
                await asyncio.to_thread(tts.load)
            except Exception:  # noqa: BLE001
                log.exception("no se pudo cargar la voz")
        yield

    app = FastAPI(title="jarvis-tts", docs_url=None, redoc_url=None, openapi_url=None, lifespan=lifespan)

    @app.get("/health")
    def health() -> dict:
        return {"status": "ok", **tts.status}

    @app.post("/v1/audio/speech")
    async def speech(body: SpeechRequest) -> Response:
        if body.response_format != "wav":
            raise HTTPException(status_code=422, detail="solo response_format=wav")
        text = body.input.strip()
        if not text:
            raise HTTPException(status_code=422, detail="texto vacío")
        if len(text) > MAX_CHARS:
            raise HTTPException(status_code=413, detail=f"texto de {len(text)} caracteres (máximo {MAX_CHARS})")
        started = time.perf_counter()
        try:
            audio = await tts.speak(text, body.speed)
        except Exception as exc:  # noqa: BLE001
            log.exception("síntesis fallida")
            raise HTTPException(status_code=503, detail=f"voz no disponible: {exc}") from exc
        log.info("sintetizados %d caracteres en %.2f s", len(text), time.perf_counter() - started)
        return Response(content=audio, media_type="audio/wav")

    return app


app = create_app() if os.environ.get("JARVIS_TTS_NO_APP") != "1" else None

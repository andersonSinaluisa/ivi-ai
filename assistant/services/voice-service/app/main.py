"""Construcción del servicio: componentes, API y ciclo de vida.

El servicio de voz se queda con lo que exige la PC (micrófono, detección de frases, palabra de activación y
altavoz). La transcripción y la voz las hace tu servidor (``<servidor>/stt`` y ``<servidor>/tts``) con la credencial
que entrega la app de escritorio por ``PUT /cloud``.
"""

from __future__ import annotations

import asyncio
import contextlib
import logging
from collections.abc import AsyncIterator
from dataclasses import dataclass, field
from pathlib import Path

from fastapi import FastAPI
from fastapi.middleware.cors import CORSMiddleware

from .api.routes import router
from .api.ws import ws_router
from .audio.devices import AudioInput, AudioOutput, NullOutput, SoundDeviceInput, SoundDeviceOutput, audio_available
from .cloud import CloudCredential
from .config.paths import data_dir
from .config.security import ensure_token
from .config.settings import VoiceSettings, load_settings
from .engine.events import EventBus
from .engine.voice_engine import VoiceEngine
from .stt.base import SttProvider
from .stt.remote import RemoteStt
from .tts.base import FallbackTts, SilentTts, TtsCache, TtsProvider
from .tts.remote import RemoteTts
from .tts.sapi import SapiTts
from .wake_word.detector import WakeWordDetector, build_detector

log = logging.getLogger("jarvis.voice")


def build_tts(settings: VoiceSettings, cloud: CloudCredential) -> FallbackTts:
    if settings.tts_provider == "sapi":
        primary: TtsProvider = SapiTts(culture=settings.language)
    elif settings.tts_provider == "silent":
        primary = SilentTts()
    else:
        primary = RemoteTts(cloud, settings.server_timeout_s)
    return FallbackTts(primary, None)


def build_stt(settings: VoiceSettings, cloud: CloudCredential) -> SttProvider:
    return RemoteStt(cloud, settings.server_timeout_s)


@dataclass
class Components:
    settings: VoiceSettings
    base_dir: Path
    token: str
    bus: EventBus
    engine: VoiceEngine
    cloud: CloudCredential
    check_on_credential: bool = True
    extra: dict = field(default_factory=dict)


def build_components(
    settings: VoiceSettings | None = None,
    base_dir: Path | None = None,
    *,
    stt: SttProvider | None = None,
    tts: TtsProvider | None = None,
    detector: WakeWordDetector | None = None,
    audio_in: AudioInput | None = None,
    audio_out: AudioOutput | None = None,
    cloud: CloudCredential | None = None,
    check_on_credential: bool = True,
) -> Components:
    base = base_dir or data_dir()
    settings = settings or load_settings(base)
    settings.check_binding()
    token = ensure_token(base)
    bus = EventBus()
    cloud = cloud or CloudCredential()
    ok, _ = audio_available()
    engine = VoiceEngine(
        settings=settings,
        stt=stt or build_stt(settings, cloud),
        tts=tts or build_tts(settings, cloud),
        detector=detector or build_detector(settings.wake_word_enabled, settings.wake_word_model),
        audio_in=audio_in or SoundDeviceInput(settings.input_device),
        audio_out=audio_out or (SoundDeviceOutput(settings.output_device) if ok else NullOutput()),
        bus=bus,
        cache=TtsCache(base / "tts-cache", settings.tts_cache_max_mb),
    )
    return Components(settings, base, token, bus, engine, cloud, check_on_credential)


async def check_server(c: Components) -> dict:
    """Comprueba transcripción y voz en el servidor; publica un error si algo falla."""
    result = {}
    for kind, provider in (("stt", c.engine.stt), ("tts", c.engine.tts)):
        try:
            if hasattr(provider, "load"):
                await asyncio.to_thread(provider.load)  # type: ignore[attr-defined]
            result[kind] = {"ok": bool(provider.ready), "detail": provider.detail}
        except Exception as exc:  # noqa: BLE001
            result[kind] = {"ok": False, "detail": str(exc)}
            c.bus.publish({"type": "error", "code": f"{kind}_unavailable", "message": str(exc)})
    return result


def create_app(components: Components | None = None) -> FastAPI:
    c = components or build_components()

    @contextlib.asynccontextmanager
    async def lifespan(app: FastAPI) -> AsyncIterator[None]:  # noqa: ARG001
        await c.engine.start()
        log.info("servicio de voz en http://%s:%s (esperando la credencial del servidor)", c.settings.host, c.settings.port)
        yield
        await c.engine.stop()

    app = FastAPI(title="Jarvis Voice Service", version="0.2.0", lifespan=lifespan, docs_url=None, redoc_url=None)
    app.state.c = c
    app.add_middleware(
        CORSMiddleware,
        allow_origins=c.settings.allowed_origins,
        allow_methods=["GET", "POST", "PUT", "DELETE"],
        allow_headers=["X-Jarvis-Token", "Content-Type"],
    )
    app.include_router(router)
    app.include_router(ws_router)
    return app

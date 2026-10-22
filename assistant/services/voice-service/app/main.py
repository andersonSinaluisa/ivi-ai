"""Construcción del servicio: componentes, API y ciclo de vida."""

from __future__ import annotations

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
from .config.paths import data_dir, service_root
from .config.security import ensure_token
from .config.settings import VoiceSettings, load_settings
from .engine.events import EventBus
from .engine.voice_engine import VoiceEngine
from .speakers.profiles import SpeakerProfileStore
from .stt.remote import RemoteStt, read_token
from .stt.whisper import FasterWhisperStt, SttProvider, local_model_dir
from .tts.base import FallbackTts, SilentTts, TtsCache, TtsProvider
from .tts.openvoice import OpenVoiceTts
from .tts.sapi import SapiTts
from .tts.vibevoice import VibeVoiceTts
from .wake_word.detector import WakeWordDetector, build_detector

log = logging.getLogger("jarvis.voice")


def build_tts(name: str, settings: VoiceSettings) -> TtsProvider:
    if name == "openvoice":
        ckpt = Path(settings.openvoice_checkpoints)
        if not ckpt.is_absolute():
            ckpt = service_root() / ckpt
        return OpenVoiceTts(ckpt)
    if name == "vibevoice":
        return VibeVoiceTts(settings.vibevoice_command, settings.vibevoice_voice)
    if name == "silent":
        return SilentTts()
    return SapiTts(culture=settings.language)


def build_tts_chain(settings: VoiceSettings) -> FallbackTts:
    primary = build_tts(settings.tts_provider, settings)
    fallback = build_tts(settings.tts_fallback, settings) if settings.tts_fallback != settings.tts_provider else None
    return FallbackTts(primary, fallback)


def build_stt(settings: VoiceSettings, base_dir: Path | None = None) -> SttProvider:
    local = FasterWhisperStt(settings.whisper_model, settings.whisper_device, settings.whisper_compute_type)
    if settings.stt_provider != "remote":
        return local
    # Respaldo solo si el modelo ya está copiado en models/whisper/<modelo>/: nunca descarga en silencio.
    has_local = local_model_dir(settings.whisper_model, service_root() / "models" / "whisper") is not None
    return RemoteStt(
        settings.stt_remote_url,
        read_token(base_dir),
        settings.whisper_model,
        settings.stt_remote_timeout_s,
        fallback=local if has_local else None,
    )


@dataclass
class Components:
    settings: VoiceSettings
    base_dir: Path
    token: str
    bus: EventBus
    engine: VoiceEngine
    profiles: SpeakerProfileStore
    preload_stt: bool = True
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
    preload_stt: bool = True,
) -> Components:
    base = base_dir or data_dir()
    settings = settings or load_settings(base)
    settings.check_binding()
    token = ensure_token(base)
    bus = EventBus()
    profiles = SpeakerProfileStore(base / "voices")
    ok, _ = audio_available()
    engine = VoiceEngine(
        settings=settings,
        stt=stt or build_stt(settings, base),
        tts=tts or build_tts_chain(settings),
        detector=detector or build_detector(settings.wake_word_enabled, settings.wake_word_model),
        audio_in=audio_in or SoundDeviceInput(settings.input_device),
        audio_out=audio_out or (SoundDeviceOutput(settings.output_device) if ok else NullOutput()),
        bus=bus,
        profiles=profiles,
        cache=TtsCache(base / "tts-cache", settings.tts_cache_max_mb),
    )
    return Components(settings, base, token, bus, engine, profiles, preload_stt)


def create_app(components: Components | None = None) -> FastAPI:
    c = components or build_components()

    @contextlib.asynccontextmanager
    async def lifespan(app: FastAPI) -> AsyncIterator[None]:
        await c.engine.start()
        if c.preload_stt and hasattr(c.engine.stt, "load"):
            # El modelo se carga en segundo plano: el servicio responde mientras tanto.
            import asyncio

            async def preload() -> None:
                try:
                    await asyncio.to_thread(c.engine.stt.load)  # type: ignore[attr-defined]
                except Exception as exc:  # noqa: BLE001
                    c.bus.publish({"type": "error", "code": "stt_unavailable", "message": str(exc)})

            app.state.preload = asyncio.create_task(preload())
        log.info("servicio de voz en http://%s:%s", c.settings.host, c.settings.port)
        yield
        await c.engine.stop()

    app = FastAPI(title="Jarvis Voice Service", version="0.1.0", lifespan=lifespan, docs_url=None, redoc_url=None)
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

"""API REST. Todo exige el token local (cabecera ``X-Jarvis-Token``) y un origen permitido."""

from __future__ import annotations

import asyncio
from typing import Annotated, Any

from fastapi import APIRouter, Depends, File, Header, HTTPException, Request, UploadFile
from pydantic import BaseModel, Field

from ..audio.devices import SoundDeviceInput, SoundDeviceOutput, audio_available, list_devices
from ..audio.signal import decode_wav
from ..config.security import token_matches
from ..config.settings import SAMPLE_RATE, apply_patch, save_settings
from ..wake_word.detector import build_detector


async def check_server(c: Any) -> dict:
    from ..main import check_server as run

    return await run(c)


def components(request: Request) -> Any:
    return request.app.state.c


def require_auth(
    request: Request,
    x_jarvis_token: Annotated[str | None, Header()] = None,
) -> Any:
    c = components(request)
    origin = request.headers.get("origin")
    if origin and origin not in c.settings.allowed_origins:
        raise HTTPException(status_code=403, detail="origen no permitido")
    if not token_matches(c.token, x_jarvis_token):
        raise HTTPException(status_code=401, detail="token inválido o ausente")
    return c



Auth = Annotated[Any, Depends(require_auth)]
router = APIRouter()


@router.get("/health")
def health(c: Auth) -> dict:
    e = c.engine
    audio_ok, audio_detail = audio_available()
    return {
        "status": "ok",
        "state": e.state.value,
        "stt": {"ready": e.stt.ready, "provider": e.stt.name, "detail": e.stt.detail},
        "tts": {"ready": e.tts.ready, "provider": e.tts.name, "detail": e.tts.detail},
        "audio": {
            "ready": audio_ok and e.audio_ok,
            "provider": "sounddevice",
            "detail": e.audio_error or audio_detail,
        },
        "wakeWord": e.wake_word_status(),
        "server": {"connected": c.cloud.get() is not None, "url": c.cloud.server_url},
    }


@router.get("/devices")
def devices(c: Auth) -> list[dict]:  # noqa: ARG001
    return list_devices()


@router.get("/config")
def get_config(c: Auth) -> dict:
    return c.settings.model_dump(exclude={"allowed_origins", "host", "port", "allow_remote"})


class CloudBody(BaseModel):
    server_url: str = Field(min_length=8, max_length=512)
    token: str = Field(min_length=1, max_length=8_192)


@router.put("/cloud")
async def put_cloud(c: Auth, body: CloudBody) -> dict:
    """La app entrega la URL del servidor y el token del device (se renueva cada minuto). Solo en memoria."""
    previous = c.cloud.get()
    try:
        current = c.cloud.set(body.server_url, body.token)
    except ValueError as exc:
        raise HTTPException(status_code=422, detail=str(exc)) from exc
    changed_server = previous is None or previous.server_url != current.server_url
    result = await check_server(c) if (changed_server and c.check_on_credential) else None
    return {"connected": True, "url": current.server_url, "check": result}


@router.delete("/cloud", status_code=204)
def delete_cloud(c: Auth) -> None:
    c.cloud.clear()


@router.post("/check")
async def check(c: Auth) -> dict:
    """Comprueba transcripción y voz en el servidor (para «Probar» en Configuración)."""
    result = await check_server(c)
    return {"ok": all(r["ok"] for r in result.values()), **result}


@router.put("/config")
def put_config(c: Auth, patch: dict[str, Any]) -> dict:
    old = c.settings
    try:
        new = apply_patch(old, patch)
    except ValueError as exc:
        raise HTTPException(status_code=422, detail=str(exc)) from exc
    kwargs: dict[str, Any] = {}
    if new.input_device != old.input_device:
        kwargs["audio_in"] = SoundDeviceInput(new.input_device)
    if new.output_device != old.output_device and audio_available()[0]:
        kwargs["audio_out"] = SoundDeviceOutput(new.output_device)
    if new.tts_provider != old.tts_provider:
        from ..main import build_tts

        kwargs["tts"] = build_tts(new, c.cloud)
    if new.wake_word_enabled != old.wake_word_enabled:
        kwargs["detector"] = build_detector(new.wake_word_enabled, new.wake_word_model)
    c.settings = new
    c.engine.reconfigure(new, **kwargs)
    save_settings(new, c.base_dir)
    return get_config(c)


class SpeakBody(BaseModel):
    text: str = Field(min_length=1, max_length=4_000)


@router.post("/tts/speak", status_code=202)
async def speak(c: Auth, body: SpeakBody) -> dict:
    await c.engine.speak(body.text)
    return {"state": c.engine.state.value}


@router.post("/stt/transcribe")
async def transcribe(c: Auth, file: Annotated[UploadFile, File()]) -> dict:
    data = await file.read()
    if len(data) > 25 * 1024 * 1024:
        raise HTTPException(status_code=413, detail="archivo demasiado grande")
    try:
        samples, rate = decode_wav(data)
    except Exception as exc:  # noqa: BLE001
        raise HTTPException(status_code=422, detail=f"WAV inválido: {exc}") from exc
    if rate != SAMPLE_RATE:
        from ..audio.signal import resample

        samples = resample(samples, rate, SAMPLE_RATE)
    t = await asyncio.to_thread(c.engine.stt.transcribe, samples, c.settings.language)
    return {"text": t.text, "language": t.language, "durationMs": t.duration_ms}

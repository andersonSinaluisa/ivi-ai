"""API REST. Todo exige el token local (cabecera ``X-Jarvis-Token``) y un origen permitido."""

from __future__ import annotations

import asyncio
from typing import Annotated, Any

from fastapi import APIRouter, Depends, File, Form, Header, HTTPException, Request, UploadFile
from pydantic import BaseModel, Field

from ..audio.devices import audio_available, list_devices, record_blocking
from ..audio.devices import SoundDeviceInput, SoundDeviceOutput
from ..audio.signal import decode_wav, int16_to_float
from ..config.security import token_matches
from ..config.settings import SAMPLE_RATE, apply_patch, save_settings
from ..speakers.profiles import ConsentError
from ..wake_word.detector import build_detector


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


from ..tts.vibevoice import vibevoice_status

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
        "engines": {"vibevoice": vibevoice_status(c.settings.vibevoice_command)},
    }


@router.get("/devices")
def devices(c: Auth) -> list[dict]:  # noqa: ARG001
    return list_devices()


@router.get("/config")
def get_config(c: Auth) -> dict:
    return c.settings.model_dump(exclude={"allowed_origins", "host", "port", "allow_remote"})


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
    if new.whisper_model != old.whisper_model:
        from ..main import build_stt

        kwargs["stt"] = build_stt(new)
    if new.tts_provider != old.tts_provider or new.vibevoice_command != old.vibevoice_command or new.vibevoice_voice != old.vibevoice_voice:
        from ..main import build_tts_chain

        kwargs["tts"] = build_tts_chain(new)
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


# ---------------------------------------------------------------- voces


@router.get("/voices")
def voices(c: Auth) -> list[dict]:
    return c.profiles.list()


class RecordBody(BaseModel):
    seconds: float = Field(10, ge=3, le=30)


@router.post("/voices/record")
async def record(c: Auth, body: RecordBody) -> dict:
    if c.engine.state.value not in ("IDLE", "LISTENING_WAKE_WORD"):
        raise HTTPException(status_code=409, detail="Jarvis está ocupado; inténtalo en un momento")
    try:
        samples = await asyncio.to_thread(record_blocking, body.seconds, c.settings.input_device)
    except Exception as exc:  # noqa: BLE001
        raise HTTPException(status_code=503, detail=f"no se pudo grabar: {exc}") from exc
    if float(abs(int16_to_float(samples)).max(initial=0.0)) < 0.01:
        raise HTTPException(status_code=422, detail="la grabación está en silencio; revisa el micrófono")
    return {"recordingId": c.profiles.save_recording(samples), "seconds": body.seconds}


@router.post("/voices", status_code=201)
async def create_voice(
    c: Auth,
    name: Annotated[str, Form()],
    is_self: Annotated[bool, Form()],
    consent_confirmed: Annotated[bool, Form()],
    consent_by: Annotated[str, Form()] = "",
    file: Annotated[UploadFile | None, File()] = None,
    recording_id: Annotated[str | None, Form()] = None,
) -> dict:
    try:
        if file is not None:
            wav = await file.read()
        elif recording_id:
            wav = c.profiles.take_recording(recording_id)
        else:
            raise ValueError("adjunta un WAV o graba una muestra")
        profile = c.profiles.create(name, is_self, consent_confirmed, consent_by or None, wav)
    except ConsentError as exc:
        raise HTTPException(status_code=403, detail=str(exc)) from exc
    except ValueError as exc:
        raise HTTPException(status_code=422, detail=str(exc)) from exc
    return {**profile.__dict__, "active": False}


class ActiveBody(BaseModel):
    id: str | None


@router.put("/voices/active")
def set_active(c: Auth, body: ActiveBody) -> list[dict]:
    try:
        c.profiles.set_active(body.id)
    except (KeyError, ValueError) as exc:
        raise HTTPException(status_code=404, detail="perfil no encontrado") from exc
    return c.profiles.list()


@router.delete("/voices/{pid}", status_code=204)
def delete_voice(c: Auth, pid: str) -> None:
    try:
        c.profiles.delete(pid)
    except (KeyError, ValueError) as exc:
        raise HTTPException(status_code=404, detail="perfil no encontrado") from exc

"""Configuración del servicio de voz.

Se guarda en ``<data_dir>/voice.json``. El host y el puerto no se pueden cambiar desde la API, y el
servicio se niega a escuchar fuera de loopback salvo que ``allow_remote`` esté en el archivo.
"""

from __future__ import annotations

import ipaddress
import json
from pathlib import Path

from pydantic import BaseModel, Field, field_validator

from .paths import data_dir

SAMPLE_RATE = 16_000
#: 80 ms a 16 kHz: el tamaño de bloque que espera openWakeWord.
FRAME_SAMPLES = 1_280


class VoiceSettings(BaseModel):
    host: str = "127.0.0.1"
    port: int = 8765
    allow_remote: bool = False
    language: str = "es"

    input_device: int | None = None
    output_device: int | None = None
    volume: float = Field(0.9, ge=0.0, le=1.0)
    listening_enabled: bool = True

    #: "faster-whisper" (en esta PC) o "remote" (servidor jarvis-stt; el token va aparte, en secrets/).
    stt_provider: str = "faster-whisper"
    stt_remote_url: str = ""
    stt_remote_timeout_s: float = Field(60.0, ge=5.0, le=300.0)
    whisper_model: str = "base"
    whisper_device: str = "auto"
    whisper_compute_type: str = "auto"

    tts_provider: str = "sapi"
    tts_fallback: str = "sapi"
    tts_speed: float = Field(1.0, ge=0.5, le=2.0)
    tts_cache_max_mb: int = Field(200, ge=0, le=5_000)

    wake_word_enabled: bool = False
    wake_word_model: str = "hey_jarvis"
    wake_word_threshold: float = Field(0.5, ge=0.05, le=0.99)

    vad_min_rms: float = Field(300.0, ge=10.0)
    vad_noise_ratio: float = Field(2.5, ge=1.1)
    vad_end_silence_ms: int = Field(900, ge=200, le=5_000)
    vad_no_speech_timeout_ms: int = Field(5_000, ge=1_000, le=30_000)
    max_command_seconds: int = Field(20, ge=2, le=120)
    post_speech_cooldown_ms: int = Field(700, ge=0, le=5_000)
    thinking_timeout_s: int = Field(180, ge=10, le=1_800)

    openvoice_checkpoints: str = "models/openvoice/checkpoints_v2"
    # VibeVoice (opcional, experimental): comando externo con {text_file}, {out_wav} y {voice}. Ver tts/vibevoice.py.
    vibevoice_command: list[str] = []
    vibevoice_voice: str = "Carter"
    allowed_origins: list[str] = [
        "http://localhost:1420",
        "http://127.0.0.1:1420",
        "http://tauri.localhost",
        "https://tauri.localhost",
        "tauri://localhost",
    ]

    @field_validator("tts_provider", "tts_fallback")
    @classmethod
    def _known_tts(cls, v: str) -> str:
        if v not in {"sapi", "openvoice", "vibevoice", "silent"}:
            raise ValueError("motor de voz desconocido")
        return v

    @field_validator("stt_provider")
    @classmethod
    def _known_stt(cls, v: str) -> str:
        if v not in {"faster-whisper", "remote"}:
            raise ValueError("motor de transcripción desconocido (faster-whisper o remote)")
        return v

    @field_validator("stt_remote_url")
    @classmethod
    def _remote_url(cls, v: str) -> str:
        if not v.strip():
            return ""
        from ..stt.remote import validate_url

        return validate_url(v)

    @field_validator("whisper_model")
    @classmethod
    def _known_whisper(cls, v: str) -> str:
        allowed = {"tiny", "base", "small", "medium", "large-v3", "turbo"}
        if v not in allowed:
            raise ValueError(f"modelo de whisper no soportado; usa uno de {sorted(allowed)}")
        return v

    def check_binding(self) -> None:
        """Rechaza escuchar en la red sin permiso explícito."""
        try:
            loopback = ipaddress.ip_address(self.host).is_loopback
        except ValueError:
            loopback = self.host == "localhost"
        if not loopback and not self.allow_remote:
            raise ValueError(
                f"host={self.host} expone el control de voz a la red. "
                "Usa 127.0.0.1 o pon allow_remote=true en voice.json si de verdad lo necesitas."
            )


#: Campos que la UI puede cambiar por la API.
MUTABLE_FIELDS = {
    "input_device",
    "output_device",
    "volume",
    "listening_enabled",
    "tts_provider",
    "tts_speed",
    "whisper_model",
    "stt_provider",
    "stt_remote_url",
    "wake_word_enabled",
    "wake_word_threshold",
    "vad_min_rms",
    "vad_noise_ratio",
    "vad_end_silence_ms",
    "vad_no_speech_timeout_ms",
    "max_command_seconds",
}


def settings_path(base: Path | None = None) -> Path:
    return (base or data_dir()) / "voice.json"


def load_settings(base: Path | None = None) -> VoiceSettings:
    path = settings_path(base)
    if path.is_file():
        return VoiceSettings.model_validate(json.loads(path.read_text(encoding="utf-8")))
    return VoiceSettings()


def save_settings(settings: VoiceSettings, base: Path | None = None) -> None:
    path = settings_path(base)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(settings.model_dump_json(indent=2), encoding="utf-8")


def apply_patch(settings: VoiceSettings, patch: dict) -> VoiceSettings:
    unknown = set(patch) - MUTABLE_FIELDS
    if unknown:
        raise ValueError(f"no se pueden cambiar por la API: {sorted(unknown)}")
    return VoiceSettings.model_validate({**settings.model_dump(), **patch})

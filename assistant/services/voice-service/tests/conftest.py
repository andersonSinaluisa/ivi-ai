from __future__ import annotations

import numpy as np
import pytest

from app.audio.devices import ManualInput, NullOutput
from app.config.settings import FRAME_SAMPLES, SAMPLE_RATE, VoiceSettings
from app.engine.events import EventBus
from app.engine.voice_engine import VoiceEngine
from app.speakers.profiles import SpeakerProfileStore
from app.stt.whisper import ScriptedStt
from app.tts.base import FallbackTts, SilentTts, TtsCache
from app.wake_word.detector import DisabledDetector


def speech_frame(amplitude: int = 6000) -> np.ndarray:
    t = np.arange(FRAME_SAMPLES) / SAMPLE_RATE
    return (amplitude * np.sin(2 * np.pi * 220 * t)).astype(np.int16)


def silence_frame() -> np.ndarray:
    return np.zeros(FRAME_SAMPLES, dtype=np.int16)


class FakeClock:
    def __init__(self) -> None:
        self.t = 1000.0

    def __call__(self) -> float:
        return self.t

    def advance(self, s: float) -> None:
        self.t += s


def make_engine(tmp_path, *, texts=None, detector=None, tts=None, settings=None, clock=None):
    settings = settings or VoiceSettings(vad_end_silence_ms=240, post_speech_cooldown_ms=700, tts_cache_max_mb=5)
    bus = EventBus()
    audio_in, audio_out = ManualInput(), NullOutput()
    stt = ScriptedStt(texts or [])
    engine = VoiceEngine(
        settings=settings,
        stt=stt,
        tts=tts or FallbackTts(SilentTts(), None),
        detector=detector or DisabledDetector(),
        audio_in=audio_in,
        audio_out=audio_out,
        bus=bus,
        profiles=SpeakerProfileStore(tmp_path / "voices"),
        cache=TtsCache(tmp_path / "cache", settings.tts_cache_max_mb),
        clock=clock or FakeClock(),
    )
    return engine, bus, audio_out, stt


def states(bus: EventBus) -> list[str]:
    return [e["state"] for e in bus.history if e["type"] == "state"]


@pytest.fixture
def clock() -> FakeClock:
    return FakeClock()

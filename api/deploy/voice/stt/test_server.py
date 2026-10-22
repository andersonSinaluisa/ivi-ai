"""Pruebas de jarvis-stt sin modelo real:  JARVIS_STT_NO_APP=1 python -m pytest test_server.py"""

import io
import os
import wave

import numpy as np

os.environ["JARVIS_STT_NO_APP"] = "1"

from fastapi.testclient import TestClient  # noqa: E402

import server  # noqa: E402


class FakeSegment:
    def __init__(self, text: str) -> None:
        self.text = text


class FakeModel:
    def __init__(self) -> None:
        self.calls: list[tuple[int, str | None]] = []

    def transcribe(self, audio, language=None, **kwargs):
        self.calls.append((audio.size, language))
        self.kwargs = kwargs
        return [FakeSegment(" hola "), FakeSegment("Jarvis ")], type("Info", (), {"language": language or "es"})()


def wav(seconds: float, rate: int = 16_000, channels: int = 1) -> bytes:
    n = int(seconds * rate)
    pcm = (np.sin(np.arange(n * channels) / 10) * 8000).astype(np.int16)
    buf = io.BytesIO()
    with wave.open(buf, "wb") as w:
        w.setnchannels(channels)
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes(pcm.tobytes())
    return buf.getvalue()


def client() -> tuple[TestClient, FakeModel]:
    model = FakeModel()
    return TestClient(server.create_app(server.Transcriber("base", loader=lambda _: model))), model


def test_transcribes_wav_with_openai_compatible_api():
    c, model = client()
    r = c.post("/v1/audio/transcriptions", files={"file": ("a.wav", wav(1.0), "audio/wav")}, data={"language": "es", "model": "whisper-1"})
    assert r.status_code == 200, r.text
    assert r.json() == {"text": "hola Jarvis", "language": "es", "duration": 1.0}
    assert model.calls == [(16_000, "es")]
    assert "Jarvis" in model.kwargs["initial_prompt"]  # vocabulario por defecto
    c.post("/v1/audio/transcriptions", files={"file": ("a.wav", wav(0.5), "audio/wav")}, data={"prompt": "Anderson, Guayaquil"})
    assert model.kwargs["initial_prompt"] == "Anderson, Guayaquil"
    assert c.get("/health").json()["ready"] is True


def test_resamples_and_downmixes():
    c, model = client()
    r = c.post("/v1/audio/transcriptions", files={"file": ("a.wav", wav(0.5, rate=48_000, channels=2), "audio/wav")})
    assert r.status_code == 200
    assert model.calls[0][0] == 8_000


def test_rejects_non_wav_long_audio_and_bad_language():
    c, _ = client()
    assert c.post("/v1/audio/transcriptions", files={"file": ("a.mp3", b"ID3....", "audio/mpeg")}).status_code == 415
    long = wav(server.MAX_SECONDS + 1)
    assert c.post("/v1/audio/transcriptions", files={"file": ("a.wav", long, "audio/wav")}).status_code == 413
    bad = c.post("/v1/audio/transcriptions", files={"file": ("a.wav", wav(0.2), "audio/wav")}, data={"language": "es; rm"})
    assert bad.status_code == 422


def test_model_failure_is_503_and_reported_in_health():
    def broken(_):
        raise RuntimeError("sin red")

    c = TestClient(server.create_app(server.Transcriber("base", loader=broken)))
    r = c.post("/v1/audio/transcriptions", files={"file": ("a.wav", wav(0.2), "audio/wav")})
    assert r.status_code == 503
    assert c.get("/health").json()["error"] == "sin red"

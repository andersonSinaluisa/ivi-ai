"""Pruebas de jarvis-tts sin voz real:  python -m pytest test_server.py"""

import io
import os
import wave

os.environ["JARVIS_TTS_NO_APP"] = "1"

from fastapi.testclient import TestClient  # noqa: E402

import server  # noqa: E402


def fake_wav(_voice, text: str, speed: float) -> bytes:
    buf = io.BytesIO()
    with wave.open(buf, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(22_050)
        w.writeframes(b"\0\0" * int(100 * len(text) / speed))
    return buf.getvalue()


def client(loader=lambda _: object()):
    return TestClient(server.create_app(server.Speaker("es_MX-claude-high", loader=loader, synth=fake_wav)))


def test_speech_returns_wav():
    c = client()
    r = c.post("/v1/audio/speech", json={"model": "tts-1", "input": "Hola, Anderson", "voice": "x", "response_format": "wav"})
    assert r.status_code == 200
    assert r.headers["content-type"] == "audio/wav"
    with wave.open(io.BytesIO(r.content)) as w:
        assert w.getframerate() == 22_050
        assert w.getnframes() == 1400
    fast = c.post("/v1/audio/speech", json={"input": "Hola, Anderson", "speed": 2.0})
    assert len(fast.content) < len(r.content)
    assert c.get("/health").json() == {"status": "ok", "voice": "es_MX-claude-high", "ready": True, "error": None}


def test_rejects_bad_requests():
    c = client()
    assert c.post("/v1/audio/speech", json={"input": "   "}).status_code == 422
    assert c.post("/v1/audio/speech", json={"input": "x", "response_format": "mp3"}).status_code == 422
    assert c.post("/v1/audio/speech", json={"input": "x" * (server.MAX_CHARS + 1)}).status_code == 413
    assert c.post("/v1/audio/speech", json={"input": "x", "speed": 9}).status_code == 422


def test_voice_failure_is_503():
    def broken(_):
        raise RuntimeError("sin voz")

    c = client(broken)
    assert c.post("/v1/audio/speech", json={"input": "hola"}).status_code == 503
    assert c.get("/health").json()["error"] == "sin voz"

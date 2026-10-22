"""API y WebSocket: autenticación, origen, binding y comandos."""

import pytest
from fastapi.testclient import TestClient
from starlette.websockets import WebSocketDisconnect

from app.audio.devices import ManualInput, NullOutput
from app.config.settings import VoiceSettings
from app.main import build_components, create_app
from app.stt.whisper import ScriptedStt
from app.tts.base import FallbackTts, SilentTts
from app.wake_word.detector import DisabledDetector


@pytest.fixture
def setup(tmp_path, monkeypatch):
    monkeypatch.delenv("JARVIS_VOICE_TOKEN", raising=False)
    c = build_components(
        VoiceSettings(),
        tmp_path,
        stt=ScriptedStt(["hola"]),
        tts=FallbackTts(SilentTts(), None),
        detector=DisabledDetector(),
        audio_in=ManualInput(),
        audio_out=NullOutput(),
        preload_stt=False,
    )
    with TestClient(create_app(c)) as client:
        yield client, c


def auth(c):
    return {"X-Jarvis-Token": c.token}


def test_token_is_persisted_and_required(setup, tmp_path):
    client, c = setup
    assert (tmp_path / "voice-token").read_text() == c.token
    assert len(c.token) >= 32
    assert client.get("/health").status_code == 401
    assert client.get("/health", headers={"X-Jarvis-Token": "x"}).status_code == 401
    r = client.get("/health", headers=auth(c))
    assert r.status_code == 200
    assert r.json()["state"] == "IDLE"
    assert r.json()["wakeWord"]["available"] is False


def test_foreign_origins_are_rejected(setup):
    client, c = setup
    r = client.get("/health", headers={**auth(c), "Origin": "https://evil.example"})
    assert r.status_code == 403
    assert client.get("/health", headers={**auth(c), "Origin": "http://tauri.localhost"}).status_code == 200


def test_config_patch_is_validated(setup):
    client, c = setup
    assert client.put("/config", headers=auth(c), json={"host": "0.0.0.0"}).status_code == 422
    assert client.put("/config", headers=auth(c), json={"volume": 3}).status_code == 422
    r = client.put("/config", headers=auth(c), json={"volume": 0.5, "tts_speed": 1.2})
    assert r.status_code == 200
    assert r.json()["volume"] == 0.5
    assert "host" not in r.json()


def test_refuses_to_bind_to_the_network():
    with pytest.raises(ValueError, match="red"):
        VoiceSettings(host="0.0.0.0").check_binding()
    VoiceSettings(host="0.0.0.0", allow_remote=True).check_binding()
    VoiceSettings(host="127.0.0.1").check_binding()


def test_websocket_requires_auth(setup):
    client, _ = setup
    with client.websocket_connect("/ws") as ws:
        ws.send_json({"type": "auth", "token": "malo"})
        with pytest.raises(WebSocketDisconnect) as exc:
            ws.receive_json()
        assert exc.value.code == 4401


def test_websocket_rejects_foreign_origin(setup):
    client, _ = setup
    with pytest.raises(WebSocketDisconnect) as exc:
        with client.websocket_connect("/ws", headers={"Origin": "https://evil.example"}) as ws:
            ws.receive_json()
    assert exc.value.code == 4403


def test_websocket_ptt_round_trip_and_reconnect(setup):
    from tests.conftest import speech_frame

    client, c = setup
    with client.websocket_connect("/ws") as ws:
        ws.send_json({"type": "auth", "token": c.token})
        hello = ws.receive_json()
        assert hello["type"] == "hello" and hello["state"] == "IDLE"
        ws.send_json({"type": "ptt_start"})
        assert ws.receive_json()["state"] == "RECORDING_COMMAND"
        for _ in range(6):
            c.engine.audio_in.push(speech_frame())
        ws.send_json({"type": "ptt_stop"})
        seen = []
        while True:
            m = ws.receive_json()
            seen.append(m["type"])
            if m["type"] == "transcript":
                assert m["text"] == "hola"
                break
        ws.send_json({"type": "speak", "text": "Hola, Anderson."})
        while True:
            m = ws.receive_json()
            if m["type"] == "state" and m["state"] == "IDLE":
                break
    # Reconexión: un cliente nuevo recibe el estado actual.
    with client.websocket_connect("/ws") as ws:
        ws.send_json({"type": "auth", "token": c.token})
        assert ws.receive_json() == {**c.engine.snapshot()}


def test_voice_profile_endpoints_enforce_consent(setup):
    from tests.test_profiles import wav

    client, c = setup
    files = {"file": ("voz.wav", wav(4), "audio/wav")}
    r = client.post("/voices", headers=auth(c), data={"name": "Ana", "is_self": "false", "consent_confirmed": "true"}, files=files)
    assert r.status_code == 403
    r = client.post("/voices", headers=auth(c), data={"name": "Yo", "is_self": "true", "consent_confirmed": "true"}, files=files)
    assert r.status_code == 201
    pid = r.json()["id"]
    assert client.put("/voices/active", headers=auth(c), json={"id": pid}).json()[0]["active"]
    assert client.delete(f"/voices/{pid}", headers=auth(c)).status_code == 204
    assert client.get("/voices", headers=auth(c)).json() == []


def test_transcribe_endpoint(setup):
    from app.audio.signal import encode_wav
    import numpy as np

    client, c = setup
    data = encode_wav(np.zeros(22_050, np.float32), 22_050)
    r = client.post("/stt/transcribe", headers=auth(c), files={"file": ("a.wav", data, "audio/wav")})
    assert r.status_code == 200
    assert r.json()["text"] == "hola"
    assert r.json()["durationMs"] == 1000

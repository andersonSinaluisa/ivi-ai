"""Transcripción y voz en el servidor: RemoteStt/RemoteTts contra jarvis-stt y jarvis-tts reales
(api/deploy/voice/*/server.py, con modelos simulados) detrás de un proxy que exige la credencial del device,
como nginx con auth_request."""

from __future__ import annotations

import importlib.util
import io
import os
import socket
import threading
import time
import urllib.error
import urllib.request
import wave
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import numpy as np
import pytest
from fastapi.testclient import TestClient

from app.audio.devices import ManualInput, NullOutput
from app.cloud import CloudCredential, validate_url
from app.config.settings import SAMPLE_RATE, VoiceSettings
from app.engine.voice_engine import _server_code
from app.main import build_components, create_app
from app.server_http import ServerError
from app.stt.remote import RemoteStt
from app.tts.base import TtsError
from app.tts.remote import RemoteTts
from app.wake_word.detector import DisabledDetector

DEPLOY = Path(__file__).resolve().parents[4] / "api" / "deploy" / "voice"
TOKEN = "jwt-del-device"


def load(name: str, path: Path):
    os.environ["JARVIS_STT_NO_APP"] = "1"
    os.environ["JARVIS_TTS_NO_APP"] = "1"
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)  # type: ignore[union-attr]
    return module


def free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class FakeWhisper:
    def transcribe(self, audio, language=None, **_):
        seg = type("S", (), {"text": " enciende la luz "})()
        return [seg], type("I", (), {"language": language})()


def fake_piper(_voice, text: str, speed: float) -> bytes:
    buf = io.BytesIO()
    with wave.open(buf, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(22_050)
        w.writeframes(b"\x10\x00" * int(200 * len(text) / speed))
    return buf.getvalue()


@pytest.fixture(scope="module")
def server():
    """jarvis-stt y jarvis-tts reales + proxy /stt y /tts que exige la credencial (como nginx)."""
    uvicorn = pytest.importorskip("uvicorn")
    stt_mod = load("jarvis_stt_server", DEPLOY / "stt" / "server.py")
    tts_mod = load("jarvis_tts_server", DEPLOY / "tts" / "server.py")
    stt_app = stt_mod.create_app(stt_mod.Transcriber("base", loader=lambda _: FakeWhisper()))
    tts_app = tts_mod.create_app(tts_mod.Speaker("es_MX-claude-high", loader=lambda _: object(), synth=fake_piper))
    ports = {"stt": free_port(), "tts": free_port()}
    servers = []
    for name, app in (("stt", stt_app), ("tts", tts_app)):
        srv = uvicorn.Server(uvicorn.Config(app, host="127.0.0.1", port=ports[name], log_level="warning"))
        threading.Thread(target=srv.run, daemon=True).start()
        servers.append(srv)

    class Proxy(BaseHTTPRequestHandler):
        def _forward(self, method: str) -> None:
            if self.headers.get("Authorization") != f"Bearer {TOKEN}":
                self.send_response(401)
                self.send_header("Content-Length", "0")
                self.end_headers()
                return
            prefix, _, rest = self.path.lstrip("/").partition("/")
            if prefix not in ports:
                self.send_response(404)
                self.end_headers()
                return
            body = self.rfile.read(int(self.headers.get("Content-Length") or 0)) if method == "POST" else None
            req = urllib.request.Request(f"http://127.0.0.1:{ports[prefix]}/{rest}", data=body, method=method)
            if self.headers.get("Content-Type"):
                req.add_header("Content-Type", self.headers["Content-Type"])
            try:
                with urllib.request.urlopen(req, timeout=10) as r:  # noqa: S310
                    status, data, ctype = r.status, r.read(), r.headers.get("Content-Type", "")
            except urllib.error.HTTPError as e:
                status, data, ctype = e.code, e.read(), "application/json"
            self.send_response(status)
            self.send_header("Content-Type", ctype)
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def do_GET(self):  # noqa: N802
            self._forward("GET")

        def do_POST(self):  # noqa: N802
            self._forward("POST")

        def log_message(self, *_):
            pass

    proxy_port = free_port()
    proxy = ThreadingHTTPServer(("127.0.0.1", proxy_port), Proxy)
    threading.Thread(target=proxy.serve_forever, daemon=True).start()
    for port in ports.values():
        for _ in range(100):
            try:
                urllib.request.urlopen(f"http://127.0.0.1:{port}/health", timeout=1)  # noqa: S310
                break
            except OSError:
                time.sleep(0.05)
    yield f"http://127.0.0.1:{proxy_port}"
    proxy.shutdown()
    for srv in servers:
        srv.should_exit = True


def speech(seconds: float = 1.0) -> np.ndarray:
    t = np.arange(int(seconds * SAMPLE_RATE)) / SAMPLE_RATE
    return (0.3 * np.sin(2 * np.pi * 220 * t)).astype(np.float32)


def credential(url: str, token: str = TOKEN) -> CloudCredential:
    c = CloudCredential()
    c.set(url, token)
    return c


def test_server_url_must_be_https_except_loopback():
    assert validate_url("https://jarvis.example.com/") == "https://jarvis.example.com"
    assert validate_url("http://127.0.0.1:9") == "http://127.0.0.1:9"
    with pytest.raises(ValueError):
        validate_url("http://jarvis.example.com")
    with pytest.raises(ValueError):
        CloudCredential().set("https://jarvis.example.com", "  ")


def test_without_credential_nothing_leaves_the_pc():
    stt, tts = RemoteStt(CloudCredential()), RemoteTts(CloudCredential())
    with pytest.raises(ServerError, match="empareja"):
        stt.transcribe(speech(0.3), "es")
    with pytest.raises(TtsError, match="empareja"):
        tts.synthesize("hola", 1.0)
    assert not stt.ready and not tts.ready


def test_transcribes_on_the_server_with_the_device_credential(server):
    stt = RemoteStt(credential(server))
    stt.load()
    assert stt.ready and "modelo base listo" in stt.detail
    t = stt.transcribe(speech(1.0), "es")
    assert (t.text, t.language, t.duration_ms) == ("enciende la luz", "es", 1000)


def test_speaks_with_the_server_voice(server):
    tts = RemoteTts(credential(server))
    tts.load()
    assert "es_MX-claude-high" in tts.detail and tts.ready
    out = tts.synthesize("Hola, Anderson", 1.0)
    assert out.sample_rate == 22_050 and out.samples.size == 2_800 and out.provider == "server"
    assert tts.synthesize("Hola, Anderson", 2.0).samples.size < out.samples.size
    assert "es_MX-claude-high" in tts.cache_id


def test_a_rejected_credential_is_flagged_for_renewal(server):
    stt = RemoteStt(credential(server, "caducado"))
    with pytest.raises(ServerError) as info:
        stt.transcribe(speech(0.3), "es")
    assert info.value.auth and _server_code(info.value, "stt_failed") == "server_auth"
    tts = RemoteTts(credential(server, "caducado"))
    with pytest.raises(TtsError) as tts_info:
        tts.synthesize("hola", 1.0)
    assert _server_code(tts_info.value, "tts_failed") == "server_auth"


def test_api_receives_the_credential_in_memory_and_checks_the_server(tmp_path, monkeypatch, server):
    monkeypatch.delenv("JARVIS_VOICE_TOKEN", raising=False)
    c = build_components(
        VoiceSettings(), tmp_path, detector=DisabledDetector(), audio_in=ManualInput(), audio_out=NullOutput()
    )
    h = {"X-Jarvis-Token": c.token}
    with TestClient(create_app(c)) as client:
        health = client.get("/health", headers=h).json()
        assert health["server"] == {"connected": False, "url": None}
        assert health["stt"]["provider"] == "server" and health["tts"]["provider"] == "server"
        assert client.put("/cloud", headers=h, json={"server_url": "http://evil.example", "token": TOKEN}).status_code == 422
        r = client.put("/cloud", headers=h, json={"server_url": server, "token": TOKEN})
        assert r.status_code == 200, r.text
        assert r.json()["check"]["stt"]["ok"] and r.json()["check"]["tts"]["ok"]
        assert client.post("/check", headers=h).json()["ok"] is True
        assert client.get("/health", headers=h).json()["server"]["connected"] is True
        assert TOKEN not in client.get("/config", headers=h).text
        assert not any(TOKEN in p.read_text(errors="ignore") for p in tmp_path.rglob("*") if p.is_file())
        assert client.delete("/cloud", headers=h).status_code == 204
        assert client.post("/check", headers=h).json()["ok"] is False


def test_old_voice_engines_in_voice_json_move_to_the_server():
    assert VoiceSettings(tts_provider="openvoice").tts_provider == "server"
    assert VoiceSettings(tts_provider="vibevoice").tts_provider == "server"
    assert VoiceSettings().tts_provider == "server"
    assert VoiceSettings.model_validate({"whisper_model": "base", "stt_provider": "faster-whisper"}).tts_provider == "server"
    with pytest.raises(ValueError):
        VoiceSettings(tts_provider="otra")

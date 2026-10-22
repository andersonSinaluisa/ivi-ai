"""Transcripción en el servidor: cliente RemoteStt contra el jarvis-stt real (api/deploy/optional/stt/server.py)
servido por uvicorn, con un proxy de token delante como hace nginx."""

from __future__ import annotations

import importlib.util
import os
import socket
import threading
import time
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import numpy as np
import pytest
from fastapi.testclient import TestClient

from app.audio.devices import ManualInput, NullOutput
from app.config.settings import SAMPLE_RATE, VoiceSettings
from app.main import build_components, build_stt, create_app
from app.stt.remote import RemoteStt, read_token, validate_url, write_token
from app.stt.whisper import FasterWhisperStt, ScriptedStt
from app.tts.base import FallbackTts, SilentTts
from app.wake_word.detector import DisabledDetector

SERVER = Path(__file__).resolve().parents[4] / "api" / "deploy" / "optional" / "stt" / "server.py"
TOKEN = "t0k3n-de-prueba"


def load_server_module():
    os.environ["JARVIS_STT_NO_APP"] = "1"
    spec = importlib.util.spec_from_file_location("jarvis_stt_server", SERVER)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)  # type: ignore[union-attr]
    return module


class FakeModel:
    def __init__(self) -> None:
        self.seen: list[tuple[int, str | None]] = []

    def transcribe(self, audio, language=None, **_):
        self.seen.append((audio.size, language))
        seg = type("S", (), {"text": " enciende la luz "})()
        return [seg], type("I", (), {"language": language})()


def free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


@pytest.fixture(scope="module")
def stt_server():
    """jarvis-stt real (modelo falso) + un proxy que exige el token, como nginx en el VPS."""
    pytest.importorskip("uvicorn")
    import uvicorn

    module = load_server_module()
    model = FakeModel()
    app = module.create_app(module.Transcriber("base", loader=lambda _: model))
    backend_port, proxy_port = free_port(), free_port()
    server = uvicorn.Server(uvicorn.Config(app, host="127.0.0.1", port=backend_port, log_level="warning"))
    threading.Thread(target=server.run, daemon=True).start()

    class Proxy(BaseHTTPRequestHandler):
        def _forward(self, method: str) -> None:
            if self.headers.get("Authorization") != f"Bearer {TOKEN}":
                self.send_response(401)
                self.end_headers()
                return
            prefix = "/stt"
            if not self.path.startswith(prefix + "/"):
                self.send_response(404)
                self.end_headers()
                return
            body = self.rfile.read(int(self.headers.get("Content-Length") or 0)) if method == "POST" else None
            req = urllib.request.Request(f"http://127.0.0.1:{backend_port}{self.path[len(prefix):]}", data=body, method=method)
            if self.headers.get("Content-Type"):
                req.add_header("Content-Type", self.headers["Content-Type"])
            try:
                with urllib.request.urlopen(req, timeout=10) as r:  # noqa: S310
                    status, data = r.status, r.read()
            except urllib.error.HTTPError as e:
                status, data = e.code, e.read()
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def do_GET(self):  # noqa: N802
            self._forward("GET")

        def do_POST(self):  # noqa: N802
            self._forward("POST")

        def log_message(self, *_):
            pass

    proxy = ThreadingHTTPServer(("127.0.0.1", proxy_port), Proxy)
    threading.Thread(target=proxy.serve_forever, daemon=True).start()
    for _ in range(100):
        try:
            urllib.request.urlopen(f"http://127.0.0.1:{backend_port}/health", timeout=1)  # noqa: S310
            break
        except OSError:
            time.sleep(0.05)
    yield f"http://127.0.0.1:{proxy_port}/stt", model
    proxy.shutdown()
    server.should_exit = True


def speech(seconds: float = 1.0) -> np.ndarray:
    t = np.arange(int(seconds * SAMPLE_RATE)) / SAMPLE_RATE
    return (0.3 * np.sin(2 * np.pi * 220 * t)).astype(np.float32)


def test_url_must_be_https_except_loopback():
    assert validate_url("https://jarvis.example.com/stt/") == "https://jarvis.example.com/stt"
    assert validate_url("http://127.0.0.1:9100") == "http://127.0.0.1:9100"
    with pytest.raises(ValueError):
        validate_url("http://jarvis.example.com/stt")
    with pytest.raises(ValueError):
        VoiceSettings(stt_remote_url="http://jarvis.example.com/stt")
    with pytest.raises(ValueError):
        VoiceSettings(stt_provider="nube")


def test_token_is_stored_apart_and_can_be_deleted(tmp_path):
    assert read_token(tmp_path) is None
    write_token("  abc  ", tmp_path)
    assert read_token(tmp_path) == "abc"
    assert "abc" not in VoiceSettings().model_dump_json()
    write_token(None, tmp_path)
    assert read_token(tmp_path) is None


def test_transcribes_through_the_server_with_token(stt_server):
    url, model = stt_server
    stt = RemoteStt(url, TOKEN, "base")
    stt.load()
    assert stt.ready and "listo" in stt.detail
    t = stt.transcribe(speech(1.0), "es")
    assert t.text == "enciende la luz"
    assert t.language == "es"
    assert t.duration_ms == 1000
    assert model.seen[-1] == (SAMPLE_RATE, "es")


def test_wrong_token_is_reported(stt_server):
    url, _ = stt_server
    stt = RemoteStt(url, "otro", "base")
    with pytest.raises(ConnectionError, match="401"):
        stt.load()
    assert not stt.ready
    assert "401" in stt.detail
    with pytest.raises(ConnectionError, match="401"):
        stt.transcribe(speech(0.3), "es")


def test_unreachable_server_falls_back_to_local_model_when_available():
    dead = f"http://127.0.0.1:{free_port()}/stt"
    local = ScriptedStt(["texto local"])
    stt = RemoteStt(dead, TOKEN, "base", timeout_s=2, fallback=local)
    assert stt.transcribe(speech(0.3), "es").text == "texto local"
    assert "no se pudo conectar" in stt.detail and "modelo local" in stt.detail
    with pytest.raises(ConnectionError):
        RemoteStt(dead, TOKEN, "base", timeout_s=2).transcribe(speech(0.3), "es")


def test_build_stt_uses_remote_only_when_configured(tmp_path):
    assert isinstance(build_stt(VoiceSettings(), tmp_path), FasterWhisperStt)
    write_token(TOKEN, tmp_path)
    remote = build_stt(VoiceSettings(stt_provider="remote", stt_remote_url="https://jarvis.example.com/stt"), tmp_path)
    assert isinstance(remote, RemoteStt)
    assert remote._token == TOKEN  # noqa: SLF001


def test_api_switches_to_the_server_and_never_returns_the_token(tmp_path, monkeypatch, stt_server):
    url, _ = stt_server
    monkeypatch.delenv("JARVIS_VOICE_TOKEN", raising=False)
    c = build_components(
        VoiceSettings(),
        tmp_path,
        stt=ScriptedStt([]),
        tts=FallbackTts(SilentTts(), None),
        detector=DisabledDetector(),
        audio_in=ManualInput(),
        audio_out=NullOutput(),
        preload_stt=False,
    )
    h = {"X-Jarvis-Token": c.token}
    with TestClient(create_app(c)) as client:
        r = client.put("/config", headers=h, json={"stt_provider": "remote", "stt_remote_url": url})
        assert r.status_code == 200, r.text
        assert r.json()["stt_remote_token_set"] is False
        assert client.post("/stt/check", headers=h).json()["ok"] is False  # sin token: 401
        r = client.put("/stt/remote-token", headers=h, json={"token": TOKEN})
        assert r.json()["stt_remote_token_set"] is True
        assert TOKEN not in r.text
        check = client.post("/stt/check", headers=h).json()
        assert check == {"ok": True, "provider": "remote", "detail": check["detail"]}
        assert TOKEN not in client.get("/config", headers=h).text
        assert TOKEN not in (tmp_path / "voice.json").read_text()
        assert client.put("/config", headers=h, json={"stt_remote_url": "http://evil.example/stt"}).status_code == 422

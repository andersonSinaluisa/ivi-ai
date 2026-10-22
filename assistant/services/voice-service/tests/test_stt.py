import os
from types import SimpleNamespace

import numpy as np
import pytest

from app.stt.whisper import FasterWhisperStt


class FakeModel:
    def __init__(self):
        self.kwargs = None

    def transcribe(self, audio, **kwargs):
        self.kwargs = kwargs
        segs = [SimpleNamespace(text=" abre el "), SimpleNamespace(text="bloc de notas ")]
        return iter(segs), SimpleNamespace(language="es")


def test_transcribes_in_spanish_with_vad():
    model = FakeModel()
    created = []

    def factory(*args):
        created.append(args)
        return model

    stt = FasterWhisperStt("base", factory=factory)
    assert not stt.ready
    t = stt.transcribe(np.zeros(16_000, np.float32), "es")
    assert t.text == "abre el bloc de notas"
    assert t.duration_ms == 1000
    assert model.kwargs["language"] == "es"
    assert model.kwargs["vad_filter"] is True
    assert created == [("base", "auto", "auto")]
    assert stt.ready


def test_load_errors_are_reported():
    def factory(*args):
        raise ImportError("No module named 'faster_whisper'")

    stt = FasterWhisperStt("base", factory=factory)
    with pytest.raises(ImportError):
        stt.load()
    assert "faster-whisper" in stt.detail


@pytest.mark.skipif(not os.environ.get("JARVIS_TEST_WHISPER"), reason="modelo real: define JARVIS_TEST_WHISPER=1")
def test_real_model_on_silence():  # pragma: no cover - requiere el modelo descargado
    stt = FasterWhisperStt("tiny")
    t = stt.transcribe(np.zeros(16_000, np.float32), "es")
    assert isinstance(t.text, str)


def test_local_model_folder_is_used_only_when_complete(tmp_path):
    from app.stt.whisper import MODEL_FILES, local_model_dir

    folder = tmp_path / "base"
    folder.mkdir()
    for name in MODEL_FILES[:-1]:
        (folder / name).write_bytes(b"x")
    assert local_model_dir("base", tmp_path) is None
    (folder / MODEL_FILES[-1]).write_bytes(b"x")
    assert local_model_dir("base", tmp_path) == folder
    assert local_model_dir("small", tmp_path) is None


def test_load_error_explains_network_failures():
    from app.stt.whisper import load_error_hint

    ssl = Exception("ConnectError: [SSL: SSLV3_ALERT_HANDSHAKE_FAILURE] sslv3 alert handshake failure")
    assert "models/whisper" in load_error_hint(ssl)
    assert "[stt]" in load_error_hint(ImportError("No module named 'faster_whisper'"))

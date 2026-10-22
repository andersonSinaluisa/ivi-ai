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

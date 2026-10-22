import base64

import numpy as np

from app.tts.base import Speech, TtsCache, split_sentences
from app.tts.sapi import SCRIPT, encoded_script, speed_to_rate


def test_split_sentences():
    assert split_sentences("Hola. ¿Cómo estás? Bien!") == ["Hola.", "¿Cómo estás?", "Bien!"]
    long = "palabra " * 100
    parts = split_sentences(long, max_chars=100)
    assert all(len(p) <= 100 for p in parts)
    assert " ".join(parts).split() == long.split()


def test_cache_round_trip_and_eviction(tmp_path):
    cache = TtsCache(tmp_path, max_mb=1)
    speech = Speech(np.zeros(16_000, np.float32), 16_000, "x")
    k = TtsCache.key("sapi", None, 1.0, "hola")
    assert cache.get(k) is None
    cache.put(k, speech)
    hit = cache.get(k)
    assert hit is not None and hit.sample_rate == 16_000 and hit.samples.size == 16_000
    for i in range(40):  # ~32 KB cada uno → supera 1 MB
        cache.put(TtsCache.key("sapi", None, 1.0, f"t{i}"), speech)
    total = sum(p.stat().st_size for p in tmp_path.glob("*.wav"))
    assert total <= 1024 * 1024
    assert TtsCache.key("sapi", "v1", 1.0, "a") != TtsCache.key("sapi", "v2", 1.0, "a")


def test_sapi_rate_and_script_never_contains_the_text():
    assert speed_to_rate(1.0) == 0
    assert speed_to_rate(2.0) == 10
    assert speed_to_rate(0.5) == -5
    assert speed_to_rate(9) == 10
    decoded = base64.b64decode(encoded_script()).decode("utf-16-le")
    assert decoded == SCRIPT
    assert "$env:JARVIS_TTS_TEXT" in SCRIPT


# ---------------------------------------------------------------- VibeVoice (comando externo, experimental)

import sys as _sys

import pytest

from app.tts.base import TtsError
from app.tts.vibevoice import VibeVoiceTts, vibevoice_status

_FAKE_VIBEVOICE = (
    "import sys, wave, struct\n"
    "text = open(sys.argv[1], encoding='utf-8').read()\n"
    "assert sys.argv[3] == 'Carter'\n"
    "with wave.open(sys.argv[2], 'wb') as w:\n"
    "    w.setnchannels(1); w.setsampwidth(2); w.setframerate(24000)\n"
    "    w.writeframes(struct.pack('<' + 'h' * 2400, *([1000] * 2400)))\n"
)


def test_vibevoice_runs_the_configured_command_without_a_shell(tmp_path):
    tts = VibeVoiceTts([_sys.executable, "-c", _FAKE_VIBEVOICE, "{text_file}", "{out_wav}", "{voice}"])
    assert tts.ready
    speech = tts.synthesize("Hola; $(rm -rf /) && echo 'no pasa nada'", None, 1.0)
    assert speech.provider == "vibevoice"
    assert speech.sample_rate == 24000
    assert len(speech.samples) == 2400


def test_vibevoice_reports_missing_configuration_and_failures():
    assert not VibeVoiceTts([]).ready
    assert "out_wav" in VibeVoiceTts([_sys.executable, "{text_file}"]).detail
    failing = VibeVoiceTts([_sys.executable, "-c", "import sys; sys.exit(3)", "{text_file}", "{out_wav}"])
    with pytest.raises(TtsError, match="código 3"):
        failing.synthesize("hola", None, 1.0)
    status = vibevoice_status([])
    assert status["configured"] is False
    assert "inglés" in status["detail"]
    assert vibevoice_status(["/no/existe/vibevoice"])["configured"] is False

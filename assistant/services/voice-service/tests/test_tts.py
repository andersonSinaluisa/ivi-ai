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
    k = TtsCache.key("sapi", 1.0, "hola")
    assert cache.get(k) is None
    cache.put(k, speech)
    hit = cache.get(k)
    assert hit is not None and hit.sample_rate == 16_000 and hit.samples.size == 16_000
    for i in range(40):  # ~32 KB cada uno → supera 1 MB
        cache.put(TtsCache.key("sapi", 1.0, f"t{i}"), speech)
    total = sum(p.stat().st_size for p in tmp_path.glob("*.wav"))
    assert total <= 1024 * 1024
    assert TtsCache.key("server:https://a:voz1", 1.0, "a") != TtsCache.key("server:https://a:voz2", 1.0, "a")


def test_sapi_rate_and_script_never_contains_the_text():
    assert speed_to_rate(1.0) == 0
    assert speed_to_rate(2.0) == 10
    assert speed_to_rate(0.5) == -5
    assert speed_to_rate(9) == 10
    decoded = base64.b64decode(encoded_script()).decode("utf-16-le")
    assert decoded == SCRIPT
    assert "$env:JARVIS_TTS_TEXT" in SCRIPT

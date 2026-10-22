import os
import time

import numpy as np
import pytest

from app.audio.signal import encode_wav
from app.speakers.profiles import ConsentError, SpeakerProfileStore


def wav(seconds: float) -> bytes:
    return encode_wav((np.random.default_rng(1).normal(0, 0.1, int(16_000 * seconds))).astype(np.float32), 16_000)


def test_own_voice_requires_confirmation(tmp_path):
    store = SpeakerProfileStore(tmp_path)
    with pytest.raises(ConsentError):
        store.create("Yo", True, False, None, wav(5))
    p = store.create("Yo", True, True, None, wav(5))
    assert p.isSelf and p.consentBy is None
    assert (tmp_path / p.id / "reference.wav").is_file()


def test_third_party_voice_requires_who_consented(tmp_path):
    store = SpeakerProfileStore(tmp_path)
    with pytest.raises(ConsentError):
        store.create("Ana", False, True, "", wav(5))
    p = store.create("Ana", False, True, "Ana Pérez", wav(5))
    assert p.consentBy == "Ana Pérez"


def test_sample_length_and_format_are_validated(tmp_path):
    store = SpeakerProfileStore(tmp_path)
    with pytest.raises(ValueError, match="entre"):
        store.create("Corta", True, True, None, wav(1))
    with pytest.raises(ValueError, match="WAV"):
        store.create("Mala", True, True, None, b"no soy un wav")


def test_active_profile_and_delete(tmp_path):
    store = SpeakerProfileStore(tmp_path)
    p = store.create("Yo", True, True, None, wav(4))
    assert store.active_voice() is None
    store.set_active(p.id)
    assert store.active_voice().id == p.id
    assert store.list()[0]["active"]
    store.delete(p.id)
    assert store.active_voice() is None
    with pytest.raises(ValueError):
        store.set_active("../etc")


def test_recordings_are_temporary(tmp_path):
    store = SpeakerProfileStore(tmp_path)
    rid = store.save_recording(np.zeros(16_000 * 4, np.int16))
    data = store.take_recording(rid)
    assert data[:4] == b"RIFF"
    with pytest.raises(ValueError):
        store.take_recording(rid)  # se consume al usarla
    old = store.save_recording(np.zeros(16_000, np.int16))
    path = tmp_path / "_recordings" / f"{old}.wav"
    past = time.time() - 3600
    os.utime(path, (past, past))
    assert store.purge_recordings() == 1

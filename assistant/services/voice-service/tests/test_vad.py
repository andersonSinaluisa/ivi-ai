import numpy as np

from app.audio.vad import EnergyVad
from tests.conftest import silence_frame, speech_frame


def test_detects_start_and_end_of_speech():
    vad = EnergyVad(frame_ms=80, min_rms=300, end_silence_ms=240, start_frames=2)
    results = [vad.update(f) for f in [silence_frame()] * 3 + [speech_frame()] * 4 + [silence_frame()] * 3]
    assert any(r.started for r in results)
    assert results[-1].ended
    assert not any(r.ended for r in results[:-1])


def test_adapts_to_background_noise():
    rng = np.random.default_rng(0)
    noise = lambda: (rng.normal(0, 900, 1280)).astype(np.int16)  # noqa: E731
    vad = EnergyVad(frame_ms=80, min_rms=300, noise_ratio=2.5)
    # Sin calibrar, un ruido de fondo alto parece voz…
    assert vad.update(noise()).is_speech
    # …por eso el motor calibra con el audio de fondo mientras espera la palabra de activación.
    for _ in range(60):
        vad.calibrate(noise())
    vad.reset()
    # El umbral subió por encima del ruido: el ruido ya no cuenta como voz.
    assert vad.threshold > 1500
    assert not vad.update(noise()).is_speech
    assert vad.update(speech_frame(9000)).is_speech


def test_a_single_click_is_not_speech():
    vad = EnergyVad(frame_ms=80, start_frames=2)
    vad.update(speech_frame())
    vad.update(silence_frame())
    assert not vad.speech_started

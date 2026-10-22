"""Flujos completos del VoiceEngine con dispositivos y modelos simulados."""

import asyncio

from app.engine.state_machine import VoiceState as S
from app.tts.base import FallbackTts, SilentTts
from app.wake_word.detector import ScriptedDetector
from app.config.settings import VoiceSettings
from tests.conftest import make_engine, silence_frame, speech_frame, states


def run(coro):
    return asyncio.run(coro)


def test_push_to_talk_full_cycle(tmp_path):
    async def main():
        engine, bus, out, stt = make_engine(tmp_path, texts=["¿qué hora es?"])
        await engine.start()
        assert engine.state == S.IDLE
        await engine.ptt_start()
        for f in [speech_frame()] * 8:
            engine.feed(f)
        await engine.settle()
        await engine.ptt_stop()
        await engine.settle()
        assert engine.state == S.THINKING
        transcript = [e for e in bus.history if e["type"] == "transcript"]
        assert transcript[0]["text"] == "¿qué hora es?"
        assert len(stt.calls[0]) == 8 * 1280

        await engine.speak("Son las diez. Que tengas buen día.")
        await engine.settle()
        assert engine.state == S.IDLE
        assert len(out.played) == 2, "una reproducción por frase"
        assert states(bus) == ["RECORDING_COMMAND", "TRANSCRIBING", "THINKING", "SPEAKING", "IDLE"]
        await engine.stop()

    run(main())


def test_wake_word_then_vad_end_of_speech(tmp_path, clock):
    async def main():
        settings = VoiceSettings(wake_word_enabled=True, vad_end_silence_ms=240)
        detector = ScriptedDetector([0.1, 0.9])
        engine, bus, out, _ = make_engine(tmp_path, texts=["abre el bloc de notas"], detector=detector, settings=settings, clock=clock)
        await engine.start()
        assert engine.state == S.LISTENING_WAKE_WORD
        engine.feed(silence_frame())
        engine.feed(speech_frame())  # aquí "detecta" la palabra
        await engine.settle()
        assert engine.state == S.RECORDING_COMMAND
        assert len(out.played) == 1, "suena la señal de activación"
        for f in [speech_frame()] * 5 + [silence_frame()] * 3:
            engine.feed(f)
        await engine.settle()
        assert engine.state == S.THINKING
        assert any(e["type"] == "wake" for e in bus.history)
        await engine.agent_done()
        assert engine.state == S.LISTENING_WAKE_WORD
        await engine.stop()

    run(main())


def test_silence_returns_to_rest(tmp_path, clock):
    async def main():
        settings = VoiceSettings(wake_word_enabled=True, vad_no_speech_timeout_ms=1000)
        engine, bus, _, stt = make_engine(tmp_path, detector=ScriptedDetector([0.95]), settings=settings, clock=clock)
        await engine.start()
        engine.feed(silence_frame())
        await engine.settle()
        assert engine.state == S.RECORDING_COMMAND
        clock.advance(1.2)
        engine.feed(silence_frame())
        await engine.settle()
        assert engine.state == S.LISTENING_WAKE_WORD
        assert stt.calls == []
        assert any(e.get("code") == "no_speech" for e in bus.history)
        await engine.stop()

    run(main())


def test_does_not_wake_on_its_own_voice(tmp_path, clock):
    async def main():
        settings = VoiceSettings(wake_word_enabled=True, post_speech_cooldown_ms=700)
        detector = ScriptedDetector([0.99, 0.99, 0.99])
        engine, _, _, _ = make_engine(tmp_path, detector=detector, settings=settings, clock=clock)
        await engine.start()
        await engine.speak("Hola, soy Jarvis.")
        # Mientras habla, el micrófono capta su propia voz: no debe procesarse.
        engine.feed(speech_frame())
        await engine.settle()
        assert engine.state == S.LISTENING_WAKE_WORD
        # Justo después de hablar (enfriamiento) tampoco.
        engine.feed(speech_frame())
        await engine.settle()
        assert detector.calls == 0
        assert engine.state == S.LISTENING_WAKE_WORD
        clock.advance(1.0)
        engine.feed(speech_frame())
        await engine.settle()
        assert detector.calls == 1
        assert engine.state == S.RECORDING_COMMAND
        await engine.stop()

    run(main())


def test_push_to_talk_interrupts_speech(tmp_path):
    class SlowOutput:
        def __init__(self):
            self.interrupted = False

        def play(self, samples, rate, volume, stop):
            if stop.wait(2.0):
                self.interrupted = True
                return False
            return True

    async def main():
        engine, bus, _, _ = make_engine(tmp_path, texts=["para"])
        engine.audio_out = SlowOutput()
        await engine.start()
        await engine.speak("Una respuesta larga que el usuario quiere cortar.")
        await asyncio.sleep(0.05)
        assert engine.state == S.SPEAKING
        await engine.ptt_start()
        assert engine.state == S.RECORDING_COMMAND
        await engine.settle()
        assert engine.audio_out.interrupted
        assert engine.state == S.RECORDING_COMMAND, "la respuesta interrumpida no le roba el estado a la grabación"
        await engine.stop()

    run(main())


def test_cancel_during_transcription_discards_result(tmp_path):
    async def main():
        engine, bus, _, stt = make_engine(tmp_path, texts=["texto"])
        original = stt.transcribe

        def slow(audio, language):
            import time

            time.sleep(0.2)
            return original(audio, language)

        stt.transcribe = slow
        await engine.start()
        await engine.ptt_start()
        for f in [speech_frame()] * 6:
            engine.feed(f)
        await engine.settle()
        await engine.ptt_stop()
        assert engine.state == S.TRANSCRIBING
        await engine.cancel()
        assert engine.state == S.IDLE
        await asyncio.sleep(0.3)
        assert not any(e["type"] == "transcript" for e in bus.history)
        await engine.stop()

    run(main())


def test_too_short_recordings_are_ignored(tmp_path):
    async def main():
        engine, bus, _, stt = make_engine(tmp_path)
        await engine.start()
        await engine.ptt_start()
        engine.feed(speech_frame())
        await engine.settle()
        await engine.ptt_stop()
        assert engine.state == S.IDLE
        assert stt.calls == []
        await engine.stop()

    run(main())


def test_stt_failure_goes_to_error_and_recovers(tmp_path):
    async def main():
        engine, bus, _, stt = make_engine(tmp_path)

        def boom(audio, language):
            raise RuntimeError("modelo no instalado")

        stt.transcribe = boom
        await engine.start()
        await engine.ptt_start()
        for f in [speech_frame()] * 6:
            engine.feed(f)
        await engine.settle()
        await engine.ptt_stop()
        await engine.settle()
        assert engine.state == S.ERROR
        assert any(e.get("code") == "stt_failed" for e in bus.history)
        await asyncio.sleep(1.7)
        assert engine.state == S.IDLE
        await engine.stop()

    run(main())


def test_tts_falls_back_and_caches(tmp_path):
    async def main():
        fallback = SilentTts()
        tts = FallbackTts(SilentTts(fail=True), fallback)
        tts.primary.name = "primary"
        engine, _, out, _ = make_engine(tmp_path, tts=tts)
        await engine.start()
        await engine.speak("Hola.")
        await engine.settle()
        assert len(out.played) == 1, "habló con la voz de respaldo"
        assert fallback.calls == ["Hola."]
        assert tts.last_error == "fallo simulado"

        # Con el proveedor principal sano, la segunda vez sale de caché.
        good = SilentTts()
        engine.tts = FallbackTts(good, None)
        await engine.speak("Hola.")
        await engine.settle()
        await engine.speak("Hola.")
        await engine.settle()
        assert good.calls == ["Hola."]
        assert engine.cache.hits == 1
        await engine.stop()

    run(main())


def test_listening_toggle_switches_resting_state(tmp_path):
    async def main():
        settings = VoiceSettings(wake_word_enabled=True)
        engine, bus, _, _ = make_engine(tmp_path, detector=ScriptedDetector([]), settings=settings)
        await engine.start()
        assert engine.state == S.LISTENING_WAKE_WORD
        engine.set_listening(False)
        assert engine.state == S.IDLE
        engine.set_listening(True)
        assert engine.state == S.LISTENING_WAKE_WORD
        await engine.stop()

    run(main())


def test_missing_microphone_does_not_crash(tmp_path):
    class Broken:
        def start(self, cb):
            raise OSError("no hay micrófono")

        def stop(self):
            pass

    async def main():
        engine, bus, out, _ = make_engine(tmp_path)
        engine.audio_in = Broken()
        await engine.start()
        assert engine.state == S.IDLE
        assert not engine.audio_ok
        await engine.ptt_start()
        assert engine.state == S.IDLE
        # Hablar sigue funcionando sin micrófono.
        await engine.speak("Sigo aquí.")
        await engine.settle()
        assert len(out.played) == 1
        await engine.stop()

    run(main())


def test_output_levels_follow_the_spoken_audio(tmp_path):
    import numpy as np

    from app.tts.base import Speech

    class LoudTts:
        name = "loud"
        ready = True
        detail = "tono"

        def synthesize(self, text, speed):
            t = np.arange(16_000 * 0.4) / 16_000
            return Speech((0.5 * np.sin(2 * np.pi * 220 * t)).astype(np.float32), 16_000, "loud")

    class TimedOutput:
        def play(self, samples, rate, volume, stop):
            import time

            time.sleep(samples.size / rate)
            return True

    async def main():
        engine, bus, _, _ = make_engine(tmp_path, tts=FallbackTts(LoudTts(), None))
        engine.audio_out = TimedOutput()
        await engine.start()
        await engine.speak("Hola.")
        await engine.settle()
        levels = [e["rms"] for e in bus.history if e["type"] == "output_level"]
        assert len(levels) >= 4, levels
        assert max(levels) > 0.5
        assert levels[-1] == 0.0, "al terminar vuelve a cero"
        await engine.stop()

    run(main())

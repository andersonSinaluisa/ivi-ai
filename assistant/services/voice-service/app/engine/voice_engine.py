"""VoiceEngine: coordina micrófono, wake word, VAD, STT, TTS y altavoz con la máquina de estados.

Flujo normal (pulsar para hablar):
    IDLE ─ptt_start→ RECORDING_COMMAND ─ptt_stop→ TRANSCRIBING ─texto→ THINKING
         ─speak(respuesta)→ SPEAKING ─fin→ IDLE

Con palabra de activación:
    LISTENING_WAKE_WORD ─"hey jarvis"→ WAKE_DETECTED ─señal→ RECORDING_COMMAND ─silencio (VAD)→ …

Protección contra su propia voz: mientras habla el micrófono no se procesa, y después de hablar
hay un tiempo de enfriamiento antes de volver a buscar la palabra de activación.
"""

from __future__ import annotations

import asyncio
import contextlib
import logging
import threading
import time
from collections.abc import Callable
from typing import Any

import numpy as np

from ..audio.devices import AudioInput, AudioOutput
from ..audio.signal import chime, int16_to_float, level
from ..audio.vad import EnergyVad
from ..config.settings import FRAME_SAMPLES, SAMPLE_RATE, VoiceSettings
from ..speakers.profiles import SpeakerProfileStore
from ..stt.whisper import SttProvider
from ..tts.base import FallbackTts, Speech, TtsCache, TtsProvider, split_sentences
from ..wake_word.detector import WakeWordDetector
from .events import EventBus
from .state_machine import RESTING, InvalidTransition, VoiceState, VoiceStateMachine

log = logging.getLogger(__name__)
S = VoiceState
FRAME_MS = 1000 * FRAME_SAMPLES // SAMPLE_RATE
MAX_SPEAK_CHARS = 4_000


class VoiceEngine:
    def __init__(
        self,
        settings: VoiceSettings,
        stt: SttProvider,
        tts: TtsProvider,
        detector: WakeWordDetector,
        audio_in: AudioInput,
        audio_out: AudioOutput,
        bus: EventBus,
        profiles: SpeakerProfileStore | None = None,
        cache: TtsCache | None = None,
        clock: Callable[[], float] = time.monotonic,
    ) -> None:
        self.settings = settings
        self.stt = stt
        self.tts = tts
        self.detector = detector
        self.audio_in = audio_in
        self.audio_out = audio_out
        self.bus = bus
        self.profiles = profiles
        self.cache = cache
        self.clock = clock

        self.sm = VoiceStateMachine()
        self.listening = settings.listening_enabled
        self.audio_ok = False
        self.audio_error: str | None = None

        self._loop: asyncio.AbstractEventLoop | None = None
        self._frames: asyncio.Queue[np.ndarray] | None = None
        self._runner: asyncio.Task[None] | None = None
        self._work: asyncio.Task[Any] | None = None
        self._recording: list[np.ndarray] = []
        self._rec_started = 0.0
        self._ptt = False
        self._stop_playback = threading.Event()
        self._cooldown_until = 0.0
        self._thinking_timer: asyncio.TimerHandle | None = None
        self._vad = self._make_vad()

    # ------------------------------------------------------------------ ciclo de vida

    def _make_vad(self) -> EnergyVad:
        s = self.settings
        return EnergyVad(FRAME_MS, s.vad_min_rms, s.vad_noise_ratio, s.vad_end_silence_ms)

    @property
    def state(self) -> VoiceState:
        return self.sm.state

    @property
    def wake_word_active(self) -> bool:
        return self.detector.available and self.settings.wake_word_enabled and self.listening and self.audio_ok

    @property
    def resting_state(self) -> VoiceState:
        return S.LISTENING_WAKE_WORD if self.wake_word_active else S.IDLE

    async def start(self) -> None:
        self._loop = asyncio.get_running_loop()
        self._frames = asyncio.Queue(maxsize=100)
        self._start_audio()
        self._to_rest("startup")
        self._runner = asyncio.create_task(self._run(), name="voice-engine")

    def _start_audio(self) -> None:
        try:
            self.audio_in.start(self._on_frame_threadsafe)
            self.audio_ok, self.audio_error = True, None
        except Exception as exc:  # noqa: BLE001
            self.audio_ok, self.audio_error = False, f"micrófono no disponible: {exc}"
            log.warning(self.audio_error)
            self.bus.publish({"type": "error", "code": "audio_unavailable", "message": self.audio_error})

    async def stop(self) -> None:
        self._stop_playback.set()
        for task in (self._work, self._runner):
            if task and not task.done():
                task.cancel()
                with contextlib.suppress(asyncio.CancelledError, Exception):
                    await task
        with contextlib.suppress(Exception):
            self.audio_in.stop()

    def reconfigure(
        self,
        settings: VoiceSettings,
        *,
        audio_in: AudioInput | None = None,
        audio_out: AudioOutput | None = None,
        stt: SttProvider | None = None,
        tts: TtsProvider | None = None,
        detector: WakeWordDetector | None = None,
    ) -> None:
        """Aplica una nueva configuración sin reiniciar el servicio."""
        self.settings = settings
        self._vad = self._make_vad()
        if stt is not None:
            self.stt = stt
        if tts is not None:
            self.tts = tts
        if detector is not None:
            self.detector = detector
        if audio_out is not None:
            self.audio_out = audio_out
        if audio_in is not None:
            with contextlib.suppress(Exception):
                self.audio_in.stop()
            self.audio_in = audio_in
            self._start_audio()
        if self.state in RESTING:
            self._to_rest("settings")

    def snapshot(self) -> dict[str, Any]:
        return {
            "type": "hello",
            "state": self.state.value,
            "listening": self.listening,
            "wakeWord": self.wake_word_status(),
        }

    def wake_word_status(self) -> dict[str, Any]:
        return {
            "enabled": self.settings.wake_word_enabled,
            "available": self.detector.available,
            "model": self.detector.model_name,
            "detail": self.detector.detail,
        }

    # ------------------------------------------------------------------ audio entrante

    def _on_frame_threadsafe(self, frame: np.ndarray) -> None:
        if self._loop is not None:
            self._loop.call_soon_threadsafe(self.feed, frame)

    def feed(self, frame: np.ndarray) -> None:
        """Entrega un bloque de 80 ms (int16). Si la cola está llena se descarta el más viejo."""
        if self._frames is None:
            return
        if self._frames.full():
            with contextlib.suppress(asyncio.QueueEmpty):
                self._frames.get_nowait()
                self._frames.task_done()
        self._frames.put_nowait(frame)

    def _drain_frames(self) -> None:
        if self._frames is None:
            return
        while not self._frames.empty():
            self._frames.get_nowait()
            self._frames.task_done()

    async def _run(self) -> None:
        assert self._frames is not None
        while True:
            frame = await self._frames.get()
            try:
                await self._process(frame)
            except asyncio.CancelledError:
                raise
            except Exception as exc:  # noqa: BLE001
                log.exception("error procesando audio")
                self._fail("engine_error", str(exc))
            finally:
                self._frames.task_done()

    async def _process(self, frame: np.ndarray) -> None:
        st = self.state
        if st != S.SPEAKING:
            rms, peak = level(frame)
            self.bus.publish({"type": "level", "rms": rms, "peak": peak})
        if st == S.LISTENING_WAKE_WORD:
            if self.clock() < self._cooldown_until:
                return  # eco de su propia voz
            self._vad.calibrate(frame)
            score = self.detector.process(frame)
            if score >= self.settings.wake_word_threshold:
                await self._on_wake(score)
        elif st == S.RECORDING_COMMAND:
            await self._on_record_frame(frame)
        # Resto de estados (incluido SPEAKING): el micrófono se ignora.

    async def _on_wake(self, score: float) -> None:
        self._set(S.WAKE_DETECTED, "wake_word")
        self.bus.publish({"type": "wake", "score": round(score, 3)})
        self.detector.reset()
        await self._play_blocking(chime(), SAMPLE_RATE)
        self._drain_frames()  # no grabar la propia señal
        self._begin_recording(ptt=False)

    def _begin_recording(self, ptt: bool) -> None:
        self._recording = []
        self._vad.reset()
        self._ptt = ptt
        self._rec_started = self.clock()
        self._set(S.RECORDING_COMMAND, "ptt" if ptt else "wake_word")

    async def _on_record_frame(self, frame: np.ndarray) -> None:
        self._recording.append(frame)
        result = self._vad.update(frame)
        elapsed = self.clock() - self._rec_started
        if elapsed >= self.settings.max_command_seconds:
            await self._finish_recording("max_duration")
            return
        if self._ptt:
            return
        if result.ended:
            await self._finish_recording("end_of_speech")
        elif not self._vad.speech_started and elapsed * 1000 >= self.settings.vad_no_speech_timeout_ms:
            self._info("no_speech", "No escuché ninguna instrucción.")
            self._to_rest("silence")

    async def _finish_recording(self, reason: str) -> None:
        audio = np.concatenate(self._recording) if self._recording else np.zeros(0, np.int16)
        self._recording = []
        if audio.size < int(0.3 * SAMPLE_RATE):
            self._info("too_short", "La grabación fue demasiado corta.")
            self._to_rest("too_short")
            return
        self._set(S.TRANSCRIBING, reason)
        self._work = asyncio.create_task(self._transcribe(audio))

    async def _transcribe(self, audio: np.ndarray) -> None:
        try:
            t = await asyncio.to_thread(self.stt.transcribe, int16_to_float(audio), self.settings.language)
        except asyncio.CancelledError:
            raise
        except Exception as exc:  # noqa: BLE001
            self._fail("stt_failed", f"No pude transcribir el audio: {exc}")
            return
        if self.state != S.TRANSCRIBING:
            return  # se canceló mientras transcribía
        if not t.text.strip():
            self._info("no_speech", "No entendí lo que dijiste.")
            self._to_rest("empty_transcript")
            return
        self._set(S.THINKING, "transcribed")
        self.bus.publish({"type": "transcript", "text": t.text, "language": t.language, "durationMs": t.duration_ms})
        self._arm_thinking_timeout()

    # ------------------------------------------------------------------ comandos del cliente

    async def ptt_start(self) -> None:
        if not self.audio_ok:
            self.bus.publish({"type": "error", "code": "audio_unavailable", "message": self.audio_error or "sin micrófono"})
            return
        st = self.state
        if st == S.SPEAKING:
            self._stop_playback.set()  # interrumpe la respuesta
        if st in (S.IDLE, S.LISTENING_WAKE_WORD, S.SPEAKING):
            self._drain_frames()
            self._begin_recording(ptt=True)
        else:
            self._info("busy", f"Ahora no puedo grabar (estado {st.value}).")

    async def ptt_stop(self) -> None:
        if self.state == S.RECORDING_COMMAND and self._ptt:
            await self._finish_recording("ptt_released")

    async def thinking(self) -> None:
        if self.state == S.THINKING:
            self._arm_thinking_timeout()

    async def agent_done(self) -> None:
        if self.state == S.THINKING:
            self._cancel_thinking_timeout()
            self._to_rest("agent_done")

    async def speak(self, text: str) -> None:
        text = text.strip()[:MAX_SPEAK_CHARS]
        if not text:
            await self.agent_done()
            return
        if self.state not in (S.IDLE, S.LISTENING_WAKE_WORD, S.THINKING):
            self._info("busy", f"Ahora no puedo hablar (estado {self.state.value}).")
            return
        self._cancel_thinking_timeout()
        self._stop_playback = threading.Event()
        self._set(S.SPEAKING, "speak")
        self._work = asyncio.create_task(self._speak(text, self._stop_playback))

    async def stop_speaking(self) -> None:
        if self.state == S.SPEAKING:
            self._stop_playback.set()

    async def cancel(self) -> None:
        self._stop_playback.set()
        self._cancel_thinking_timeout()
        self._recording = []
        if self._work and not self._work.done() and self.state == S.TRANSCRIBING:
            self._work.cancel()
        if self.state not in RESTING or self.state != self.resting_state:
            with contextlib.suppress(InvalidTransition):
                self._to_rest("cancelled")

    def set_listening(self, enabled: bool) -> None:
        self.listening = enabled
        self._info("listening", "on" if enabled else "off")
        if self.state in RESTING:
            self._to_rest("listening_toggle")

    # ------------------------------------------------------------------ habla

    def _synthesize(self, text: str) -> Speech:
        voice = self.profiles.active_voice() if self.profiles else None
        voice_id = voice.id if voice and self.tts.supports_cloning else None
        expected = self.tts.primary.name if isinstance(self.tts, FallbackTts) else self.tts.name
        key = TtsCache.key(expected, voice_id, self.settings.tts_speed, text) if self.cache else ""
        if self.cache and (hit := self.cache.get(key)):
            return hit
        speech = self.tts.synthesize(text, voice, self.settings.tts_speed)
        if self.cache and speech.provider == expected:
            self.cache.put(key, speech)
        return speech

    async def _speak(self, text: str, stop: threading.Event) -> None:
        chunks = split_sentences(text)
        interrupted = False
        pending: asyncio.Task[Speech] | None = None
        try:
            pending = asyncio.create_task(asyncio.to_thread(self._synthesize, chunks[0]))
            for i in range(len(chunks)):
                speech = await pending
                pending = None
                if i + 1 < len(chunks):
                    # Sintetiza la siguiente frase mientras suena esta.
                    pending = asyncio.create_task(asyncio.to_thread(self._synthesize, chunks[i + 1]))
                if stop.is_set():
                    interrupted = True
                    break
                self.bus.publish({"type": "speaking", "chunk": i + 1, "total": len(chunks)})
                finished = asyncio.Event()
                meter = asyncio.create_task(self._publish_output_levels(speech, stop, finished))
                try:
                    done = await asyncio.to_thread(
                        self.audio_out.play, speech.samples, speech.sample_rate, self.settings.volume, stop
                    )
                finally:
                    finished.set()
                    await meter
                if not done:
                    interrupted = True
                    break
        except asyncio.CancelledError:
            raise
        except Exception as exc:  # noqa: BLE001
            self._fail("tts_failed", f"No pude generar la voz: {exc}")
            return
        finally:
            if pending is not None:
                pending.cancel()
            self._cooldown_until = self.clock() + self.settings.post_speech_cooldown_ms / 1000
        if self.state == S.SPEAKING:
            self._to_rest("interrupted" if interrupted else "spoken")

    async def _publish_output_levels(self, speech: Speech, stop: threading.Event, finished: asyncio.Event) -> None:
        """Envía la amplitud del audio que se está reproduciendo (cada 50 ms) para la visualización."""
        window = max(1, speech.sample_rate // 20)
        samples = speech.samples.astype(np.float32) * float(self.settings.volume)
        for start in range(0, samples.size, window):
            if stop.is_set() or finished.is_set():
                break
            chunk = samples[start : start + window]
            rms = float(np.sqrt(np.mean(chunk * chunk))) if chunk.size else 0.0
            self.bus.publish({"type": "output_level", "rms": round(min(1.0, rms * 4.0), 3)})
            try:
                await asyncio.wait_for(finished.wait(), timeout=0.05)
                break
            except asyncio.TimeoutError:
                continue
        self.bus.publish({"type": "output_level", "rms": 0.0})

    async def _play_blocking(self, samples: np.ndarray, rate: int) -> None:
        try:
            await asyncio.to_thread(self.audio_out.play, samples, rate, self.settings.volume, threading.Event())
        except Exception as exc:  # noqa: BLE001
            log.warning("no se pudo reproducir la señal: %s", exc)

    # ------------------------------------------------------------------ utilidades

    def _set(self, dst: VoiceState, reason: str | None) -> None:
        prev = self.sm.transition(dst, reason)
        if prev != dst:
            self.bus.publish({"type": "state", "state": dst.value, "previous": prev.value, "reason": reason})

    def _to_rest(self, reason: str) -> None:
        target = self.resting_state
        if self.state == target:
            return
        if self.state in RESTING or self.sm.can(target):
            self._set(target, reason)

    def _info(self, code: str, message: str) -> None:
        self.bus.publish({"type": "info", "code": code, "message": message})

    def _fail(self, code: str, message: str) -> None:
        self.bus.publish({"type": "error", "code": code, "message": message})
        with contextlib.suppress(InvalidTransition):
            self._set(S.ERROR, code)
        if self._loop is not None:
            self._loop.call_later(1.5, self._recover)

    def _recover(self) -> None:
        if self.state == S.ERROR:
            self._set(self.resting_state, "recovered")

    def _arm_thinking_timeout(self) -> None:
        self._cancel_thinking_timeout()
        if self._loop is not None:
            self._thinking_timer = self._loop.call_later(self.settings.thinking_timeout_s, self._thinking_expired)

    def _cancel_thinking_timeout(self) -> None:
        if self._thinking_timer is not None:
            self._thinking_timer.cancel()
            self._thinking_timer = None

    def _thinking_expired(self) -> None:
        if self.state == S.THINKING:
            self._info("thinking_timeout", "Jarvis no respondió a tiempo.")
            self._to_rest("thinking_timeout")

    async def settle(self) -> None:
        """Espera a que se procese el audio pendiente y termine el trabajo en curso (pruebas)."""
        if self._frames is not None:
            await self._frames.join()
        while self._work is not None and not self._work.done():
            await asyncio.wait([self._work])

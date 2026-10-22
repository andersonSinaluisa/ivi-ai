"""Máquina de estados de la voz."""

from __future__ import annotations

from enum import Enum


class VoiceState(str, Enum):
    IDLE = "IDLE"
    LISTENING_WAKE_WORD = "LISTENING_WAKE_WORD"
    WAKE_DETECTED = "WAKE_DETECTED"
    RECORDING_COMMAND = "RECORDING_COMMAND"
    TRANSCRIBING = "TRANSCRIBING"
    THINKING = "THINKING"
    SPEAKING = "SPEAKING"
    ERROR = "ERROR"


S = VoiceState
RESTING = {S.IDLE, S.LISTENING_WAKE_WORD}

TRANSITIONS: dict[VoiceState, set[VoiceState]] = {
    S.IDLE: {S.LISTENING_WAKE_WORD, S.RECORDING_COMMAND, S.SPEAKING, S.ERROR},
    S.LISTENING_WAKE_WORD: {S.IDLE, S.WAKE_DETECTED, S.RECORDING_COMMAND, S.SPEAKING, S.ERROR},
    S.WAKE_DETECTED: {S.RECORDING_COMMAND, *RESTING, S.ERROR},
    S.RECORDING_COMMAND: {S.TRANSCRIBING, *RESTING, S.ERROR},
    S.TRANSCRIBING: {S.THINKING, *RESTING, S.ERROR},
    S.THINKING: {S.SPEAKING, *RESTING, S.ERROR},
    # Pulsar para hablar mientras habla interrumpe la respuesta y empieza a grabar.
    S.SPEAKING: {S.RECORDING_COMMAND, *RESTING, S.ERROR},
    S.ERROR: {*RESTING},
}


class InvalidTransition(RuntimeError):
    def __init__(self, src: VoiceState, dst: VoiceState) -> None:
        super().__init__(f"transición inválida {src.value} → {dst.value}")
        self.src, self.dst = src, dst


class VoiceStateMachine:
    def __init__(self, initial: VoiceState = S.IDLE) -> None:
        self.state = initial
        self.history: list[tuple[VoiceState, VoiceState, str | None]] = []

    def can(self, dst: VoiceState) -> bool:
        return dst in TRANSITIONS[self.state]

    def transition(self, dst: VoiceState, reason: str | None = None) -> VoiceState:
        """Cambia de estado. Devuelve el anterior. Una transición al mismo estado no hace nada."""
        src = self.state
        if dst == src:
            return src
        if not self.can(dst):
            raise InvalidTransition(src, dst)
        self.state = dst
        self.history.append((src, dst, reason))
        return src

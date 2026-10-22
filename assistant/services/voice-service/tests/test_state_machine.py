import pytest

from app.engine.state_machine import TRANSITIONS, InvalidTransition, VoiceState, VoiceStateMachine

S = VoiceState


def test_happy_path_ptt():
    sm = VoiceStateMachine()
    for dst in (S.RECORDING_COMMAND, S.TRANSCRIBING, S.THINKING, S.SPEAKING, S.IDLE):
        sm.transition(dst)
    assert [h[1] for h in sm.history] == [S.RECORDING_COMMAND, S.TRANSCRIBING, S.THINKING, S.SPEAKING, S.IDLE]


def test_wake_word_path():
    sm = VoiceStateMachine(S.LISTENING_WAKE_WORD)
    for dst in (S.WAKE_DETECTED, S.RECORDING_COMMAND, S.TRANSCRIBING, S.THINKING, S.LISTENING_WAKE_WORD):
        sm.transition(dst)
    assert sm.state == S.LISTENING_WAKE_WORD


@pytest.mark.parametrize(
    ("src", "dst"),
    [(S.IDLE, S.TRANSCRIBING), (S.IDLE, S.THINKING), (S.THINKING, S.RECORDING_COMMAND), (S.ERROR, S.SPEAKING), (S.TRANSCRIBING, S.SPEAKING)],
)
def test_invalid_transitions_are_rejected(src, dst):
    sm = VoiceStateMachine(src)
    with pytest.raises(InvalidTransition):
        sm.transition(dst)
    assert sm.state == src


def test_every_state_can_fail_and_recover():
    for state in S:
        if state is S.ERROR:
            continue
        assert S.ERROR in TRANSITIONS[state], state
    assert TRANSITIONS[S.ERROR] == {S.IDLE, S.LISTENING_WAKE_WORD}


def test_same_state_is_a_noop():
    sm = VoiceStateMachine()
    sm.transition(S.IDLE)
    assert sm.history == []

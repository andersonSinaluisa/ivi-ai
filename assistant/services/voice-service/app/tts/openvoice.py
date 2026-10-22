"""OpenVoice V2 (MIT) para voz clonada en español.

Flujo: MeloTTS genera la frase con un hablante base español y el ToneColorConverter le aplica el
timbre de la voz de referencia. El embedding de la voz se calcula una vez y se guarda junto al
perfil (``se.pth``).

ESTADO: adaptador escrito según la API pública de OpenVoice V2, **no ejecutado** en este entorno
(requiere PyTorch y los checkpoints, ~3–5 GB). Ver PROGRESS.md. Si falla, ``FallbackTts`` usa la voz
de Windows.

Instalación: scripts/install-openvoice.ps1 (clona el repo, instala MeloTTS y descarga
``checkpoints_v2``). Licencias: OpenVoice y MeloTTS son MIT.
"""

from __future__ import annotations

import logging
import tempfile
import threading
from pathlib import Path
from typing import Any

from ..audio.signal import decode_wav
from .base import Speech, TtsError, VoiceRef

log = logging.getLogger(__name__)


class OpenVoiceTts:
    name = "openvoice"
    supports_cloning = True

    def __init__(self, checkpoints_dir: Path, language: str = "ES") -> None:
        self.ckpt = checkpoints_dir
        self.language = language
        self._lock = threading.Lock()
        self._loaded = False
        self._error: str | None = None
        self._torch: Any = None
        self._se_extractor: Any = None
        self._converter: Any = None
        self._melo: Any = None
        self._speaker_id: Any = None
        self._source_se: Any = None
        if not (self.ckpt / "converter" / "checkpoint.pth").is_file():
            self._error = f"faltan los checkpoints en {self.ckpt}. Ejecuta scripts/install-openvoice.ps1."

    @property
    def ready(self) -> bool:
        return self._loaded

    @property
    def detail(self) -> str:
        if self._error:
            return self._error
        return "listo" if self._loaded else "se cargará en el primer uso"

    def _load(self) -> None:
        with self._lock:
            if self._loaded:
                return
            if self._error:
                raise TtsError(self._error)
            try:
                import torch  # type: ignore[import-not-found]
                from melo.api import TTS  # type: ignore[import-not-found]
                from openvoice import se_extractor  # type: ignore[import-not-found]
                from openvoice.api import ToneColorConverter  # type: ignore[import-not-found]
            except Exception as exc:  # noqa: BLE001
                self._error = f"OpenVoice no está instalado ({exc}). Ejecuta scripts/install-openvoice.ps1."
                raise TtsError(self._error) from exc

            device = "cuda:0" if torch.cuda.is_available() else "cpu"
            converter = ToneColorConverter(str(self.ckpt / "converter" / "config.json"), device=device)
            converter.load_ckpt(str(self.ckpt / "converter" / "checkpoint.pth"))
            melo = TTS(language=self.language, device=device)
            speakers = melo.hps.data.spk2id
            key = next(iter(speakers.keys()))
            self._speaker_id = speakers[key]
            se_file = self.ckpt / "base_speakers" / "ses" / f"{key.lower().replace('_', '-')}.pth"
            self._source_se = torch.load(str(se_file), map_location=device)
            self._torch, self._se_extractor, self._converter, self._melo = torch, se_extractor, converter, melo
            self._loaded = True
            log.info("OpenVoice V2 cargado en %s (hablante base %s)", device, key)

    def _target_se(self, voice: VoiceRef) -> Any:
        cached = voice.cache_dir / "se.pth"
        if cached.is_file():
            return self._torch.load(str(cached), map_location=self._converter.device)
        voice.cache_dir.mkdir(parents=True, exist_ok=True)
        se, _ = self._se_extractor.get_se(
            str(voice.reference_wav), self._converter, target_dir=str(voice.cache_dir / "processed"), vad=True
        )
        self._torch.save(se, str(cached))
        return se

    def synthesize(self, text: str, voice: VoiceRef | None, speed: float) -> Speech:
        if voice is None:
            raise TtsError("no hay una voz clonada activa")
        self._load()
        target_se = self._target_se(voice)
        with tempfile.TemporaryDirectory(prefix="jarvis-ov-") as tmp:
            base = Path(tmp) / "base.wav"
            out = Path(tmp) / "out.wav"
            self._melo.tts_to_file(text, self._speaker_id, str(base), speed=speed)
            self._converter.convert(
                audio_src_path=str(base),
                src_se=self._source_se,
                tgt_se=target_se,
                output_path=str(out),
                message="@MyShell",
            )
            samples, rate = decode_wav(out.read_bytes())
        return Speech(samples, rate, self.name)

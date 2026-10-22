"""Perfiles de voz para clonación, con consentimiento obligatorio.

- Una voz propia exige confirmar que es tuya.
- La voz de otra persona exige confirmar su consentimiento explícito y su nombre.
- Todo se guarda en ``<data_dir>/voices`` y nunca se envía fuera de la PC.
- Las grabaciones hechas desde la UI antes de dar el consentimiento son temporales: se borran al
  crear el perfil o a los 10 minutos.
"""

from __future__ import annotations

import json
import shutil
import time
import uuid
from dataclasses import asdict, dataclass
from datetime import datetime, timezone
from pathlib import Path

import numpy as np

from ..audio.signal import decode_wav, encode_wav
from ..config.settings import SAMPLE_RATE
from ..tts.base import VoiceRef

MIN_SECONDS = 3.0
MAX_SECONDS = 90.0
MAX_BYTES = 25 * 1024 * 1024
RECORDING_TTL_S = 600


class ConsentError(ValueError):
    pass


@dataclass
class VoiceProfile:
    id: str
    name: str
    isSelf: bool  # noqa: N815 - contrato JSON con la UI
    consentConfirmed: bool  # noqa: N815
    consentBy: str | None  # noqa: N815
    consentAt: str  # noqa: N815
    createdAt: str  # noqa: N815


class SpeakerProfileStore:
    def __init__(self, root: Path) -> None:
        self.root = root
        self.recordings = root / "_recordings"
        self.root.mkdir(parents=True, exist_ok=True)
        self.purge_recordings()

    # ---- grabaciones temporales ----

    def save_recording(self, samples: np.ndarray) -> str:
        self.recordings.mkdir(parents=True, exist_ok=True)
        rid = uuid.uuid4().hex
        (self.recordings / f"{rid}.wav").write_bytes(encode_wav(samples, SAMPLE_RATE))
        return rid

    def take_recording(self, rid: str) -> bytes:
        if not rid.isalnum():
            raise ValueError("id de grabación inválido")
        path = self.recordings / f"{rid}.wav"
        if not path.is_file():
            raise ValueError("la grabación ya no existe; graba de nuevo")
        data = path.read_bytes()
        path.unlink(missing_ok=True)
        return data

    def purge_recordings(self, now: float | None = None) -> int:
        if not self.recordings.is_dir():
            return 0
        now = now or time.time()
        removed = 0
        for p in self.recordings.glob("*.wav"):
            if now - p.stat().st_mtime > RECORDING_TTL_S:
                p.unlink(missing_ok=True)
                removed += 1
        return removed

    # ---- perfiles ----

    def _dir(self, pid: str) -> Path:
        if not pid or not all(c in "0123456789abcdef" for c in pid):
            raise ValueError("id de perfil inválido")
        return self.root / pid

    def create(self, name: str, is_self: bool, consent_confirmed: bool, consent_by: str | None, wav: bytes) -> VoiceProfile:
        name = name.strip()
        if not name:
            raise ValueError("el perfil necesita un nombre")
        if not consent_confirmed:
            raise ConsentError("falta confirmar el consentimiento")
        if not is_self and not (consent_by and consent_by.strip()):
            raise ConsentError("para clonar la voz de otra persona indica quién dio su consentimiento")
        if len(wav) > MAX_BYTES:
            raise ValueError("el archivo es demasiado grande (máximo 25 MB)")
        try:
            samples, rate = decode_wav(wav)
        except Exception as exc:  # noqa: BLE001
            raise ValueError(f"no es un WAV válido: {exc}") from exc
        seconds = samples.size / rate
        if not MIN_SECONDS <= seconds <= MAX_SECONDS:
            raise ValueError(f"la muestra debe durar entre {MIN_SECONDS:.0f} y {MAX_SECONDS:.0f} s (dura {seconds:.1f} s)")

        now = datetime.now(timezone.utc).isoformat()
        profile = VoiceProfile(
            id=uuid.uuid4().hex,
            name=name[:80],
            isSelf=is_self,
            consentConfirmed=True,
            consentBy=None if is_self else consent_by.strip()[:120],  # type: ignore[union-attr]
            consentAt=now,
            createdAt=now,
        )
        d = self._dir(profile.id)
        d.mkdir(parents=True)
        (d / "reference.wav").write_bytes(encode_wav(samples, rate))
        (d / "profile.json").write_text(json.dumps(asdict(profile), ensure_ascii=False, indent=2), encoding="utf-8")
        return profile

    def list(self) -> list[dict]:
        active = self.active_id()
        out = []
        for f in sorted(self.root.glob("*/profile.json")):
            try:
                p = json.loads(f.read_text(encoding="utf-8"))
            except (OSError, json.JSONDecodeError):
                continue
            p["active"] = p["id"] == active
            out.append(p)
        return out

    def delete(self, pid: str) -> None:
        d = self._dir(pid)
        if not d.is_dir():
            raise KeyError(pid)
        shutil.rmtree(d)
        if self.active_id() == pid:
            self.set_active(None)

    def active_id(self) -> str | None:
        f = self.root / "active.json"
        if not f.is_file():
            return None
        pid = json.loads(f.read_text(encoding="utf-8")).get("id")
        return pid if pid and (self.root / pid / "profile.json").is_file() else None

    def set_active(self, pid: str | None) -> None:
        if pid is not None and not (self._dir(pid) / "profile.json").is_file():
            raise KeyError(pid)
        (self.root / "active.json").write_text(json.dumps({"id": pid}), encoding="utf-8")

    def active_voice(self) -> VoiceRef | None:
        pid = self.active_id()
        if not pid:
            return None
        d = self.root / pid
        return VoiceRef(id=pid, reference_wav=d / "reference.wav", cache_dir=d / "cache")

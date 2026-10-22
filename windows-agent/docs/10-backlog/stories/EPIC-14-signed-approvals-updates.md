# EPIC-14 — Firmas, aprobaciones remotas y actualizaciones

Fase **P8**. Depende de **Cloud Phase 3** (firma de tareas y mensaje `task.approval_decision`, [C8](../../00-overview/cloud-compatibility.md) y [C10](../../00-overview/cloud-compatibility.md)). DoD común en [User Stories](../user-stories.md#definition-of-done-común).

> **Estado: implementada en el agente.** El Cloud todavía no emite tareas firmadas ni `task.approval_decision`; el agente ya las acepta y las rechaza cuando no cuadran. El updater no se ha probado con un release firmado real (depende de HU-WIN-051). Ver [Estado de implementación](../implementation-status.md).

---

## HU-WIN-061 — Verificación de firmas de tareas

**Goal**: que la identidad del solicitante deje de ser solo afirmada.

**Description**: etapa 3 del pipeline con `signature` Ed25519 del originador (mitiga [WT-01](../../04-security/threat-model.md#wt-01-compromised-cloud) y [WT-05](../../04-security/threat-model.md#wt-05-task-tampering)).

**Acceptance Criteria**
1. Claves públicas de principals en `policy.toml` (`[[principals]] public_keys = [...]`).
2. Firma sobre la forma canónica acordada con el Cloud (`taskId`, `type`, hash del payload, `requestedBy`, `expiresAt`).
3. Política `require_signature_above = "R1"` (ejemplo): sin firma válida → `DENIED: SIGNATURE_REQUIRED`.
4. Firma inválida → `task.rejected NOT_PERMITTED` y `WARN`.

**Dependencies**: HU-WIN-053; contrato de firma publicado por el Cloud.

**Technical Considerations**: `ed25519-dalek`; canonicalización JSON (RFC 8785) acordada y con fixtures compartidos.

**Tests**: fixtures firmados por el Cloud; alteración de cada campo firmado.

**Definition of Done**: DoD común.

---

## HU-WIN-062 — Aprobación remota firmada

**Goal**: aprobar tareas R3 desde el móvil u Owner Console sin estar frente a la PC.

**Description**: [Futuro (P8): aprobación remota firmada](../../04-security/approvals.md#futuro-p8-aprobación-remota-firmada) (mitiga [WT-15](../../04-security/threat-model.md#wt-15-approval-spoofing)).

**Acceptance Criteria**
1. `task.approval_decision` aceptado solo con firma válida de una clave de aprobador registrada localmente.
2. Firma vinculada a `taskId` + hash del payload + `nonce` + `decidedAt`; nonce no reutilizable.
3. Decisión sin firma o con firma inválida → ignorada, la aprobación sigue `Pending`.
4. La aprobación local por CLI sigue funcionando.

**Dependencies**: HU-WIN-061, HU-WIN-041.

**Technical Considerations**: la clave del aprobador vive en el dispositivo del owner (WebAuthn/Passkey o clave del móvil), nunca en el Cloud.

**Tests**: aprobación falsificada por un "Cloud malicioso" de prueba → rechazada.

**Definition of Done**: DoD común.

---

## HU-WIN-063 — Actualizador firmado con rollback

**Goal**: actualizar el agente sin permitir binarios maliciosos.

**Description**: [Actualizaciones](../../05-windows/startup-model.md#actualizaciones) (mitiga [WT-13](../../04-security/threat-model.md#wt-13-agent-update-compromise)).

**Acceptance Criteria**
1. Manifiesto firmado con clave offline Ed25519 embebida; SHA-256 y Authenticode verificados.
2. Anti-downgrade.
3. Instalación en `versions\<semver>\` conservando la anterior; cambio de la Scheduled Task sin tareas en `Running`.
4. Rollback automático si la nueva versión no llega a `RUNNING` + conexión en 120 s o cae 3 veces en 10 min; condición `UPDATE_ROLLED_BACK`.
5. Backup de `agent.db` antes de migrar.

**Dependencies**: HU-WIN-051, HU-WIN-050.

**Technical Considerations**: el Cloud solo puede sugerir versión; el origen de descarga está fijado en configuración.

**Tests**: manifiesto con firma inválida, hash distinto, versión menor, binario que no arranca (rollback).

**Definition of Done**: DoD común.

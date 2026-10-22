# EPIC-06 — Permission Engine

Fase **P4**. Objetivo: la autoridad local que decide qué se ejecuta. DoD común en [User Stories](../user-stories.md#definition-of-done-común).

---

## HU-WIN-024 — Modelo y carga de `policy.toml`

**Goal**: una política local válida, recargable y que falla cerrada.

**Description**: crate `jarvis-policy`: modelo y carga según [Permission Engine](../../04-security/permission-engine.md#política-policytoml).

**Acceptance Criteria**
1. Secciones: `version`, `risk`, `principals`, `deny`, `apps`, `filesystem`, `commands`, `privacy`, `session`.
2. Validación estricta (claves desconocidas, capabilities inexistentes, `appId` con formato inválido, ejecutables prohibidos en `commands`) con mensajes con línea.
3. Sin archivo → política por defecto (solo `local:owner`; ningún principal remoto).
4. Política inválida al arrancar → fail-closed (solo R0) + `POLICY_INVALID`; inválida al recargar → se mantiene la anterior + `WARN`.
5. Recarga por `jarvis-agent policy reload` y por cambio del archivo (debounce 1 s); hash anterior/nuevo en log `INFO`.

**Dependencies**: HU-WIN-001, HU-WIN-005.

**Technical Considerations**: modelo portable (sin Windows); la vigilancia del archivo vive en `jarvis-agent`.

**Tests**: fixtures de políticas válidas/inválidas; recarga con archivo inválido.

**Definition of Done**: DoD común.

---

## HU-WIN-025 — Evaluación de riesgo

**Goal**: cada invocación tiene un riesgo efectivo explícito.

**Description**: [Risk Model](../../04-security/risk-model.md).

**Acceptance Criteria**
1. `riesgoEfectivo = max(base, modificadores(input), riesgoDelComando)`; los modificadores nunca bajan el riesgo.
2. Umbrales `auto_approve_max` y `approval_required_max` globales y por principal.
3. R4 remoto no configurable: siempre `DENIED`.
4. `policy check` advierte si un principal remoto tiene `auto_approve_max = R3`.

**Dependencies**: HU-WIN-020, HU-WIN-024.

**Technical Considerations**: `RiskLevel` como enum ordenado (`Ord`).

**Tests**: tabla de tools × inputs → riesgo.

**Definition of Done**: DoD común.

---

## HU-WIN-026 — Evaluación de permisos y `policy check`

**Goal**: decisiones `APPROVED` / `REQUIRES_APPROVAL` / `DENIED` deterministas y explicables.

**Description**: algoritmo del diagrama de [Permission Engine](../../04-security/permission-engine.md#algoritmo-de-evaluación), integrado en las etapas 8–9 del pipeline.

**Acceptance Criteria**
1. Orden: política válida → R4 remoto → deny → principal (exacto, luego `cloud:*`, sin sumar capabilities) → capabilities del input → firma (placeholder P8) → umbrales.
2. Cada decisión devuelve `reason` con código (`MISSING_CAPABILITY`, `EXPLICIT_DENY`, `UNKNOWN_PRINCIPAL`, `CRITICAL_REMOTE`, `RISK_TOO_HIGH`, `POLICY_INVALID`) y la regla aplicada; se persiste en `tasks`.
3. `DENIED` → `task.rejected NOT_PERMITTED` con `message` que nombra lo que falta sin revelar la política.
4. `jarvis-agent policy check --principal … --tool … --input …` imprime decisión, riesgo y regla, sin necesidad del proceso residente.
5. Cambio de política con tareas `Accepted`/`AwaitingApproval` → re-evaluación antes de ejecutar; si ahora es `DENIED` → `task.cancelled LOCAL_POLICY`.

**Dependencies**: HU-WIN-024, HU-WIN-025, HU-WIN-038.

**Technical Considerations**: función pura `evaluate(policy, invocation, risk) -> Decision`.

**Tests**: tabla exhaustiva (≥ 40 casos) incluyendo deny wins y comodín; property test: nunca `APPROVED` para R4 remoto.

**Definition of Done**: DoD común + [WIN-E2E-05](../../11-testing/integration-scenarios.md#win-e2e-05-permiso-denegado) verde.

---

## HU-WIN-027 — Aprobaciones locales

**Goal**: que las acciones R3 solo ocurran con decisión humana en la PC.

**Description**: [Approvals](../../04-security/approvals.md) (modo MVP: solo local).

**Acceptance Criteria**
1. `REQUIRES_APPROVAL` → aprobación `Pending` con `payload_hash` y expiración `min(expiresAt, now + approval_ttl_seconds)`; mensajes `task.ack` + `task.approval_required { reason }`.
2. `jarvis-agent approvals list` muestra taskId, tool, input resumido (sin campos sensibles), riesgo, solicitante y tiempo restante.
3. `approve <taskId>` → re-evaluar política → `Accepted` → ejecución (`task.started`); `deny` → `task.cancelled APPROVAL_DENIED`.
4. Expiración → `task.cancelled APPROVAL_EXPIRED`; `task.cancel` → `task.cancelled USER_REQUEST`.
5. Las aprobaciones sobreviven reinicios (HU-WIN-018).
6. Toast opcional sin botones de acción que indica el comando a ejecutar.

**Dependencies**: HU-WIN-026, HU-WIN-005, HU-WIN-017.

**Technical Considerations**: temporizador de expiración con reloj corregido; comandos del pipe con principal `local:owner`.

**Tests**: integración: `close_app force` desde el Cloud → `WaitingForApproval` en el Cloud → approve local → `Completed`; deny; expiración con reloj falso.

**Definition of Done**: DoD común + [WIN-E2E-13](../../11-testing/integration-scenarios.md#win-e2e-13-aprobación-local-de-una-acción-r3) verde.

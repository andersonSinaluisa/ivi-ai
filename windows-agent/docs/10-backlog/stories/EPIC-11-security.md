# EPIC-11 — Security

Fase **P6** (con tareas continuas desde P2). Objetivo: evidencias automatizadas de las mitigaciones del [Threat Model](../../04-security/threat-model.md). DoD común en [User Stories](../user-stories.md#definition-of-done-común).

---

## HU-WIN-046 — Canarios de datos sensibles

**Goal**: garantizar que ningún dato sensible llega a logs, diagnósticos ni al Cloud indebidamente.

**Description**: suite de canarios de [Security Tests](../../11-testing/security-tests.md#datos-sensibles) (mitiga [WT-14](../../04-security/threat-model.md#wt-14-sensitive-logs)).

**Acceptance Criteria**
1. Valores canario en: device secret, título de ventana, contenido de archivo (P7), salida de comando (P7), payload con campo sensible.
2. Tras ejecutar los flujos, el canario no aparece en ningún archivo de `logs\`, en el zip de `diagnostics`, en la BD tras *settled* (salvo hash) ni en mensajes MQTT distintos del `result` autorizado.
3. Se ejecuta en CI en cada PR.

**Dependencies**: HU-WIN-003, HU-WIN-019, HU-WIN-052.

**Technical Considerations**: búsqueda binaria (UTF-8 y UTF-16LE) en archivos.

**Tests**: la propia suite; un test negativo que introduce una fuga a propósito y comprueba que la suite falla.

**Definition of Done**: DoD común.

---

## HU-WIN-047 — Fuzzing del parser

**Goal**: ningún mensaje entrante puede tumbar el agente.

**Description**: `cargo-fuzz` sobre la deserialización del envelope, `task.dispatch` y la validación de input de cada tool (mitiga [WT-03](../../04-security/threat-model.md#wt-03-malicious-mqtt-message)).

**Acceptance Criteria**
1. Targets: `envelope`, `dispatch`, `tool_input::<tool>` para cada tool registrada.
2. Corpus inicial con los fixtures de HU-WIN-006.
3. Ejecución nocturna de 30 min por target; los crashes encontrados se añaden como tests de regresión.
4. Sin pánicos ni consumos de memoria > 100 MB por entrada.

**Dependencies**: HU-WIN-006, HU-WIN-022.

**Technical Considerations**: los targets viven en crates portables para ejecutarse en Linux.

**Tests**: la propia campaña.

**Definition of Done**: DoD común + 0 crashes abiertos.

---

## HU-WIN-048 — Endurecimiento del proceso

**Goal**: aplicar las mitigaciones de privilegios y superficie local.

**Description**: [Privilege Model](../../04-security/privilege-model.md) y [WT-08](../../04-security/threat-model.md#wt-08-privilege-escalation), [WT-11](../../04-security/threat-model.md#wt-11-local-attacker), [WT-12](../../04-security/threat-model.md#wt-12-credential-extraction).

**Acceptance Criteria**
1. Manifiesto `asInvoker`; rechazo de arranque elevado (HU-WIN-004) probado.
2. `SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32)` y `SetDllDirectoryW("")` al arrancar.
3. Mitigaciones de proceso: `SetProcessMitigationPolicy` (DEP, ASLR estricto, `ProcessImageLoadPolicy` sin imágenes remotas ni de baja integridad).
4. ACLs verificadas de `%LOCALAPPDATA%\Jarvis\Agent\` (sin escritura para otros usuarios) y del named pipe.
5. El agente se excluye de Windows Error Reporting con volcado completo (`WerAddExcludedApplication`).
6. Ningún puerto TCP/UDP en escucha (test con `GetExtendedTcpTable`).

**Dependencies**: HU-WIN-004, HU-WIN-005.

**Technical Considerations**: algunas mitigaciones deben aplicarse antes de cargar DLLs no esenciales.

**Tests**: integración Windows que comprueba cada punto.

**Definition of Done**: DoD común.

---

## HU-WIN-049 — Suite de seguridad del threat model

**Goal**: una prueba automatizada por amenaza mitigable.

**Description**: implementar los casos de [Security Tests](../../11-testing/security-tests.md) para WT-01…WT-15 aplicables al MVP.

**Acceptance Criteria**
1. Cloud comprometido simulado: dispatch R3 sin aprobación → nunca se ejecuta; R4 → `NOT_PERMITTED`; `requestedBy` desconocido → `NOT_PERMITTED`.
2. Replay de un dispatch antiguo → no re-ejecución.
3. Inyección en `appId` (`chrome & calc`, `../`, unicode) → `INVALID_PAYLOAD`.
4. Mensaje con `deviceId` ajeno → descartado.
5. Aprobación sin pipe (intento vía MQTT) → ignorada.
6. Proceso protegido en `close_app` → rechazado.
7. Cada caso enlaza la amenaza que cubre.

**Dependencies**: HU-WIN-042, HU-WIN-027.

**Technical Considerations**: un "Cloud malicioso" de prueba que publica directamente en el broker con la credencial `jarvis-cloud` del entorno de test.

**Tests**: la propia suite.

**Definition of Done**: DoD común + matriz amenaza → test completa en [Security Tests](../../11-testing/security-tests.md).

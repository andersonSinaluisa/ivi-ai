# EPIC-12 — Packaging

Fase **P6**. Objetivo: instalar, arrancar, diagnosticar y cerrar el MVP. DoD común en [User Stories](../user-stories.md#definition-of-done-común).

---

## HU-WIN-050 — `install` / `uninstall`

**Goal**: instalación por usuario sin admin, con arranque automático al iniciar sesión.

**Description**: [Startup Model](../../05-windows/startup-model.md#decisión).

**Acceptance Criteria**
1. `install` copia el binario a `%LOCALAPPDATA%\Programs\Jarvis\Agent\`, crea directorios de datos y registra la Scheduled Task con la configuración documentada (`ITaskService`).
2. Idempotente: reinstalar actualiza binario y tarea sin perder datos.
3. `uninstall` para el agente, borra la tarea, la credencial y los binarios; `--purge` borra también config, política, BD y logs (con confirmación).
4. No requiere admin; con token elevado avisa y se niega salvo `--allow-elevated`.
5. `install --start` arranca la tarea inmediatamente.

**Dependencies**: HU-WIN-004, HU-WIN-012.

**Technical Considerations**: el binario en uso no puede sobrescribirse: parar el agente, copiar y relanzar.

**Tests**: integración Windows: install → la tarea existe con los parámetros; logoff/logon (runner interactivo) → el agente arranca; uninstall limpia.

**Definition of Done**: DoD común.

---

## HU-WIN-051 — Build de release firmada

**Goal**: binarios reproducibles, versionados y firmados.

**Description**: pipeline de release.

**Acceptance Criteria**
1. Tag `vX.Y.Z` → build `x86_64-pc-windows-msvc` release con LTO, `panic = "unwind"` (necesario para aislar pánicos de tools con `catch_unwind`, HU-WIN-023) y símbolos (PDB) separados.
2. Firma Authenticode con timestamp; verificación de la firma en el propio pipeline.
3. Artefactos: `jarvis-agent.exe`, `SHA256SUMS`, notas de versión; `version` muestra semver y commit.
4. SBOM (`cargo cyclonedx`) y `cargo deny` sin advisories abiertos.
5. Recursos de versión de Windows (`FileVersion`, `ProductName`) embebidos.

**Dependencies**: HU-WIN-001.

**Technical Considerations**: los PDB se conservan en el almacenamiento de releases para analizar crashes; nunca se distribuyen con el binario.

**Tests**: job de CI que verifica firma y hashes.

**Definition of Done**: DoD común.

---

## HU-WIN-052 — Comando `diagnostics`

**Goal**: obtener un paquete de soporte útil sin exponer datos sensibles.

**Description**: [Diagnostics](../../07-observability/diagnostics.md).

**Acceptance Criteria**
1. Zip con health, config, política, logs de 3 días (redactados de nuevo), resumen de las últimas 200 tareas sin payloads ni outputs, versión e información mínima del sistema.
2. Opciones `--no-logs`, `--days`, `--print`, `--recover-outbox`.
3. Sin hostname, usuario, IP ni lista de software.
4. Pasa la suite de canarios (HU-WIN-046).

**Dependencies**: HU-WIN-003, HU-WIN-015.

**Technical Considerations**: generar en un directorio temporal con ACL del usuario y mover al escritorio.

**Tests**: unit de la redacción; inspección del zip en integración.

**Definition of Done**: DoD común.

---

## HU-WIN-053 — Aceptación del MVP

**Goal**: declarar el MVP completo con evidencia.

**Description**: ejecutar y registrar todos los criterios de [MVP](../../00-overview/mvp.md#criterios-obligatorios).

**Acceptance Criteria**
1. WIN-E2E-01…14 verdes en el runner interactivo contra el Cloud de referencia ([Integration Scenarios](../../11-testing/integration-scenarios.md)).
2. Suites de fallo (HU-WIN-044) y seguridad (HU-WIN-049) verdes.
3. Checklist del MVP con enlace a la evidencia de cada criterio (logs de CI, vídeos o capturas del vertical slice).
4. Documentación actualizada: estado de implementación, desviaciones y cambios propuestos al Cloud pendientes.

**Dependencies**: HU-WIN-042, HU-WIN-044, HU-WIN-049, HU-WIN-050, HU-WIN-051, HU-WIN-052.

**Technical Considerations**: la evidencia se guarda en `docs/` (fuera del alcance de esta ejecución: se crea al implementar).

**Tests**: —

**Definition of Done**: todos los criterios del MVP cumplidos y revisados.

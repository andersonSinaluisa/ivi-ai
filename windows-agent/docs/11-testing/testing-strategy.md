# Testing Strategy

## Pirámide

| Nivel | Qué prueba | Dónde corre | Herramientas |
|---|---|---|---|
| **Unit** | Lógica pura: protocolo, pipeline con fakes, política, riesgo, verificador, backoff, redacción, escapado | CI Linux y Windows | `cargo test`, `proptest`, reloj pausado de Tokio |
| **Integration** | Agente real contra **Mosquitto y Jarvis Cloud reales** (contenedores), sin tools de escritorio (`noop`, tools de observación) | CI Linux (crates portables + binario con plataforma falsa) y Windows | `docker compose` del repo `api/`, Testcontainers |
| **Windows Integration** | Adaptadores Win32/COM/UIA, tools reales con apps reales, Credential Manager, Scheduled Task, sesión | Runner Windows **interactivo** (self-hosted, auto-logon) | Apps de prueba propias + Notepad/Calculadora |
| **Security** | Mitigaciones del threat model, canarios, fuzzing | CI + campaña nocturna | [Security Tests](security-tests.md), `cargo-fuzz` |
| **Protocol Contract** | Compatibilidad con el contrato MQTT del Cloud | CI | Fixtures JSON del Cloud en `jarvis-protocol/tests/fixtures` |
| **Failure / Chaos** | Comportamiento ante fallos de red, proceso, disco y reloj | CI (parcial) + runner interactivo | Proxy TCP con cortes, kill de procesos, BD corrupta ([Failure Scenarios](failure-scenarios.md)) |

## Principios

- **Todo lo que decide es testeable sin Windows**: `jarvis-protocol`, `jarvis-tooling`, `jarvis-policy` y `jarvis-state` compilan y se prueban en Linux. `jarvis-windows` expone traits con fakes.
- **Reloj inyectable** en todo lo que depende del tiempo (expiración, backoff, aprobaciones, retención).
- **Sin mocks del Cloud en integración**: se usa el Cloud real del repo `api/` para detectar incompatibilidades pronto. El simulador `Windows Simulator` del Cloud sirve como referencia de comportamiento.
- **Apps de prueba propias** (`jarvis-test-app.exe`): ventana configurable por argumentos (salir con código N, tardar X ms en mostrar ventana, pedir confirmación al cerrar, título canario). Evitan depender de apps de terceros en la CI.
- **Tests de contrato bidireccionales**: los fixtures se generan a partir de la documentación y de los tests del Cloud; un cambio de contrato en el Cloud rompe un test del agente.

## Escenarios mínimos (pedidos)

| Escenario | Dónde se cubre |
|---|---|
| valid task | [WIN-E2E-01](integration-scenarios.md#win-e2e-01-tarea-válida-open_app) |
| invalid task | [WIN-E2E-02](integration-scenarios.md#win-e2e-02-tarea-inválida) |
| duplicate task | [WIN-E2E-03](integration-scenarios.md#win-e2e-03-tarea-duplicada) |
| revoked device | [WIN-E2E-04](integration-scenarios.md#win-e2e-04-device-revocado) |
| permission denied | [WIN-E2E-05](integration-scenarios.md#win-e2e-05-permiso-denegado) |
| tool not found | [WIN-E2E-06](integration-scenarios.md#win-e2e-06-tool-inexistente) |
| execution success | WIN-E2E-01, 14 |
| execution failed | [WIN-E2E-07](integration-scenarios.md#win-e2e-07-ejecución-fallida) |
| verification failed | [WIN-E2E-08](integration-scenarios.md#win-e2e-08-verificación-fallida) |
| MQTT disconnect | [WIN-E2E-09](integration-scenarios.md#win-e2e-09-desconexión-durante-la-ejecución) |
| result pending | WIN-E2E-09, 10 |
| agent restart | [WIN-E2E-10](integration-scenarios.md#win-e2e-10-reinicio-del-agente-con-trabajo-pendiente) |
| task cancellation | [WIN-E2E-11](integration-scenarios.md#win-e2e-11-cancelación) |
| timeout | [WIN-E2E-12](integration-scenarios.md#win-e2e-12-timeout) |

## CI

| Job | Disparador | Contenido |
|---|---|---|
| `lint` | PR | fmt, clippy `-D warnings`, `cargo deny` |
| `test-linux` | PR | unit + integración con contenedores (Mosquitto + Cloud) |
| `test-windows` | PR | workspace completo en `windows-latest` (sin escritorio interactivo) |
| `test-interactive` | merge a `main` y nightly | Windows Integration + E2E en el runner self-hosted |
| `fuzz` | nightly | 30 min por target |
| `release` | tag | build firmada, SBOM, hashes |

## Cobertura

Objetivo orientativo: ≥ 80 % de líneas en `jarvis-policy`, `jarvis-tooling` y `jarvis-state` (donde viven las decisiones). En `jarvis-windows` la métrica útil es la suite de integración, no la cobertura.

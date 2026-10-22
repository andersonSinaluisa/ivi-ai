# Testing Strategy

## Pirámide

| Nivel | Proyecto | Qué cubre | Infra | Velocidad |
|---|---|---|---|---|
| Unit | `Jarvis.Domain.Tests` | Máquina de estados, `DeviceStatus`, value objects, validaciones | Ninguna | ms |
| Unit/aplicación | `Jarvis.Application.Tests` | Casos de uso con PostgreSQL real (no mocks de `DbSet`), puertos externos con fakes (`IDeviceMessagePublisher`, `IBrokerCredentialAdmin`, `TimeProvider`) | Testcontainers PostgreSQL | s |
| Integración | `Jarvis.IntegrationTests` | API completa (`WebApplicationFactory`) + PostgreSQL + Mosquitto reales: REST, MQTT, ACL, dynsec, SignalR, health | Testcontainers PostgreSQL + Mosquitto | s-min |
| E2E | `Jarvis.E2ETests` | Escenarios de [Integration Scenarios](integration-scenarios.md) con simuladores reales | Testcontainers + procesos de simuladores | min |
| Aceptación | Manual guiado + scripts | [Acceptance Tests](acceptance-tests.md) contra el VPS | Producción | — |

## Principios

1. **Dominio sin mocks**: las reglas de estado se prueban con tests puros y exhaustivos (tabla de transiciones completa).
2. **Nada de mocks de EF Core**: PostgreSQL real vía Testcontainers; cada test en su propia base o con limpieza por transacción.
3. **Broker real para todo lo de MQTT**: ACL, dynsec, LWT y sesiones no se pueden simular con fidelidad.
4. **Tiempo controlado**: `TimeProvider` falso para sweepers y expiraciones en unit/integración; en E2E, timeouts reducidos por configuración.
5. **Dos owners siempre**: todo test de endpoint incluye un segundo owner para detectar IDOR.
6. **Pruebas de no-duplicación con contador**: el Windows Simulator expone un contador de ejecuciones por `taskId`; los tests de redelivery lo verifican.
7. **Pruebas de seguridad como tests normales**: ACL cruzada, redacción de secretos, rate limits, tokens revocados.

## Herramientas

| Necesidad | Herramienta |
|---|---|
| Framework | xUnit v3 |
| Aserciones | Assertions nativas de xUnit o Shouldly (evitar librerías con licencia comercial) |
| Contenedores | Testcontainers for .NET (módulos PostgreSQL; Mosquitto con contenedor genérico) |
| Host de API | `Microsoft.AspNetCore.Mvc.Testing` |
| MQTT en tests | MQTTnet |
| SignalR en tests | `Microsoft.AspNetCore.SignalR.Client` |
| Arquitectura | NetArchTest.Rules o test por reflexión |
| Cobertura | coverlet (objetivo orientativo: Domain ≥ 95 %, Application ≥ 80 %; no bloqueante) |

## Ejecución

```bash
dotnet test --solution Jarvis.slnx     # xUnit v3 sobre Microsoft.Testing.Platform (global.json)
dotnet test --project tests/Jarvis.E2ETests -- --filter-method "*E2E_01*"
```

Las suites de integración y E2E comparten un fixture (`JarvisTestEnvironment`) que levanta PostgreSQL 17 y Mosquitto 2.0.22 con la configuración de `deploy/` (TLS con CA generada en el momento), ejecuta el mismo `broker-bootstrap` que producción y hospeda la API en TestServer con timeouts reducidos (ACK 3 s, presencia 4/8 s). Sin Docker se usan **procesos nativos** (`initdb`/`pg_ctl` con pgvector, `mosquitto` con Dynamic Security) derivando la configuración de `deploy/mosquitto/mosquitto.conf`; se fuerza con `JARVIS_TEST_SERVICES=native` o `containers`. Si no hay ninguno de los dos, se omiten con el motivo. PostgreSQL usa la imagen `pgvector/pgvector:0.8.1-pg17` porque el esquema de memoria requiere la extensión `vector`.

## CI

Implementado en `.github/workflows/ci.yml` (asume `api/` como raíz del repositorio).

## CI (diseño original)

Pipeline por push/PR: `restore` → `build` (warnings as errors) → unit → integración → E2E → build de imagen → escaneo de vulnerabilidades. Requiere runner con Docker.

## Datos de prueba

- Builders por entidad (`DeviceBuilder`, `TaskBuilder`) en un proyecto compartido de tests.
- Secretos de prueba generados por test; nunca fijos en el repo salvo claves de desarrollo evidentes.

## Qué NO se prueba en MVP

Carga/estrés formal (se hace un smoke de 50 devices simulados como verificación de recursos, no como SLA), UI, firmware.

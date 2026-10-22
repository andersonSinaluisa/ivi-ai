# Jarvis Windows Agent: documentación

Fuente de verdad del diseño. El protocolo de transporte lo define [Jarvis Cloud](../../api/docs/README.md); las diferencias están en [Cloud Compatibility](00-overview/cloud-compatibility.md).

## Cómo leer

1. [Vision](00-overview/vision.md) → [Scope](00-overview/scope.md) → [MVP](00-overview/mvp.md)
2. [Windows Agent Architecture](01-architecture/windows-agent-architecture.md) → [Execution Pipeline](01-architecture/execution-pipeline.md)
3. [Tool System](03-tools/tool-system.md) → [Tool Contract](03-tools/tool-contract.md) → [Verification](03-tools/verification.md)
4. [Security Architecture](04-security/security-architecture.md) → [Permission Engine](04-security/permission-engine.md) → [Threat Model](04-security/threat-model.md)
5. [Implementation Order](10-backlog/implementation-order.md) para empezar a construir

## Índice

| Sección | Documentos |
|---|---|
| **00 Overview** | [Vision](00-overview/vision.md) · [Scope](00-overview/scope.md) · [Glossary](00-overview/glossary.md) · [Roadmap](00-overview/roadmap.md) · [MVP](00-overview/mvp.md) · [Cloud Compatibility](00-overview/cloud-compatibility.md) |
| **01 Architecture** | [System Context](01-architecture/system-context.md) · [Windows Agent Architecture](01-architecture/windows-agent-architecture.md) · [Component Model](01-architecture/component-model.md) · [Execution Pipeline](01-architecture/execution-pipeline.md) · [Concurrency Model](01-architecture/concurrency-model.md) · [Trust Boundaries](01-architecture/trust-boundaries.md) |
| **02 Cloud Integration** | [Jarvis Cloud Contract](02-cloud-integration/jarvis-cloud-contract.md) · [MQTT Client](02-cloud-integration/mqtt-client.md) · [Task Protocol](02-cloud-integration/task-protocol.md) · [Presence](02-cloud-integration/presence.md) · [Reconnection](02-cloud-integration/reconnection.md) |
| **03 Tools** | [Tool System](03-tools/tool-system.md) · [Tool Contract](03-tools/tool-contract.md) · [Tool Registry](03-tools/tool-registry.md) · [Process Tools](03-tools/process-tools.md) · [Filesystem Tools](03-tools/filesystem-tools.md) · [Command Execution](03-tools/command-execution.md) · [Verification](03-tools/verification.md) |
| **04 Security** | [Security Architecture](04-security/security-architecture.md) · [Threat Model](04-security/threat-model.md) · [Permission Engine](04-security/permission-engine.md) · [Risk Model](04-security/risk-model.md) · [Approvals](04-security/approvals.md) · [Credential Storage](04-security/credential-storage.md) · [Privilege Model](04-security/privilege-model.md) |
| **05 Windows** | [Windows Integration](05-windows/windows-integration.md) · [Process Management](05-windows/process-management.md) · [Active Window](05-windows/active-window.md) · [UI Automation](05-windows/ui-automation.md) · [Session Model](05-windows/session-model.md) · [Startup Model](05-windows/startup-model.md) |
| **06 State** | [Local State](06-state/local-state.md) · [Idempotency](06-state/idempotency.md) · [Crash Recovery](06-state/crash-recovery.md) |
| **07 Observability** | [Logging](07-observability/logging.md) · [Health](07-observability/health.md) · [Diagnostics](07-observability/diagnostics.md) |
| **08 Future** | [Agent Core Integration](08-future/agent-core-integration.md) · [Browser Automation](08-future/browser-automation.md) · [Computer Vision](08-future/computer-vision.md) · [Voice Integration](08-future/voice-integration.md) |
| **09 Decisions** | [ADR index](09-decisions/README.md) (ADR-001 … ADR-016) |
| **10 Backlog** | [Epics](10-backlog/epics.md) · [User Stories](10-backlog/user-stories.md) · [Implementation Order](10-backlog/implementation-order.md) · [Implementation Status](10-backlog/implementation-status.md) · [stories/](10-backlog/stories/) |
| **11 Testing** | [Testing Strategy](11-testing/testing-strategy.md) · [Integration Scenarios](11-testing/integration-scenarios.md) · [Failure Scenarios](11-testing/failure-scenarios.md) · [Security Tests](11-testing/security-tests.md) |

## Diagramas

| Diagrama | Documento |
|---|---|
| System context | [System Context](01-architecture/system-context.md) |
| Componentes | [Windows Agent Architecture](01-architecture/windows-agent-architecture.md) |
| Secuencia de ejecución de tarea | [Execution Pipeline](01-architecture/execution-pipeline.md#secuencia-caso-normal) |
| Secuencia de verificación | [Verification](03-tools/verification.md#secuencia) |
| Reconexión MQTT | [Reconnection](02-cloud-integration/reconnection.md#secuencia-de-reconexión) |
| Crash recovery | [Crash Recovery](06-state/crash-recovery.md#secuencia) |
| Evaluación de permisos | [Permission Engine](04-security/permission-engine.md#algoritmo-de-evaluación) |
| Pipeline de tools | [Tool System](03-tools/tool-system.md#flujo-de-una-invocación) y [Execution Pipeline](01-architecture/execution-pipeline.md#etapas) |
| Concurrencia y locks | [Concurrency Model](01-architecture/concurrency-model.md) |
| Trust boundaries | [Trust Boundaries](01-architecture/trust-boundaries.md) |
| Local Tool API para el Agent Core | [Agent Core Integration](08-future/agent-core-integration.md) |
| Estados locales | [Execution Pipeline](01-architecture/execution-pipeline.md#estados-locales) |
| Esquema SQLite | [Local State](06-state/local-state.md#esquema) |
| Actualización firmada | [Startup Model](05-windows/startup-model.md#actualizaciones) |

## Convenciones

- Español, con términos técnicos en inglés donde son el uso habitual.
- **MVP** vs **futuro** se indica en cada documento (fases P0–P12 del [Roadmap](00-overview/roadmap.md)).
- Enlaces relativos; ningún documento depende de rutas absolutas.
- Esta documentación describe el diseño; el código vive en [crates/](../crates/) y su estado en [Implementation Status](10-backlog/implementation-status.md).
- Los escenarios de aceptación se ejecutan con [`scripts/e2e.ps1`](../scripts/e2e.ps1) contra el Cloud real y dejan evidencia en `scripts/e2e-evidence/`.

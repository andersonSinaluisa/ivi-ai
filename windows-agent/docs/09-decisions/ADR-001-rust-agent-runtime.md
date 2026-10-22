# ADR-001: Rust como runtime del agente

## Status

Accepted, 2026-09-19.

## Context

El agente es un proceso residente en la PC personal del usuario que recibe órdenes desde Internet, parsea mensajes no confiables, llama a APIs Win32/COM y debe funcionar durante semanas sin reinicios. Requisitos: seguridad de memoria (superficie remota), bajo consumo en reposo, binario único sin runtime que instalar, acceso completo a Win32/COM/UIA, concurrencia (MQTT + varias tools + verificaciones), y ningún runtime de Python.

## Decision

Rust (edición 2024, toolchain estable fijado en `rust-toolchain.toml`) con:

| Pieza | Elección |
|---|---|
| Async | Tokio (multi-thread, `spawn_blocking` para Win32 bloqueante) |
| Windows | `windows-rs` (bindings oficiales de Microsoft) |
| MQTT | `rumqttc` (MQTT v5) sobre `rustls` + `rustls-platform-verifier` (almacén de certificados de Windows) |
| Serialización | `serde` / `serde_json` con `deny_unknown_fields`; `jsonschema` para validar inputs de tool |
| Logging | `tracing` |
| Estado | `rusqlite` (`bundled`) |
| Config | `toml` |
| CLI | `clap` |

Detalle en [Windows Agent Architecture](../01-architecture/windows-agent-architecture.md).

## Alternatives

| Alternativa | Motivo de descarte |
|---|---|
| C# / .NET (como el Cloud) | Excelente para Windows y compartiría modelos con el Cloud; requiere runtime o self-contained de ~70 MB, GC en proceso residente, memoria en reposo mayor. Opción razonable; se prefiere Rust por seguridad de memoria sin runtime, binario pequeño y control de `unsafe` confinado |
| C++ | Máximo control; sin seguridad de memoria para un parser expuesto a la red |
| Go | Runtime ligero, pero interop COM/UIA pobre y sin bindings oficiales de Windows |
| Python | Excluido por requisito (runtime, distribución, rendimiento, tipado) |

## Consequences

- ✅ Seguridad de memoria en el parser y la lógica; `unsafe` confinado al crate `jarvis-windows`.
- ✅ Binario único firmado, sin dependencias de runtime.
- ⚠️ No se comparten tipos con el Cloud (.NET): el contrato se mantiene por documentación + tests de contrato con fixtures JSON compartidos ([Testing Strategy](../11-testing/testing-strategy.md)).
- ⚠️ COM/UIA desde Rust es más verboso que desde C#; se mitiga con un hilo STA dedicado y wrappers.
- ⚠️ Curva de aprendizaje; se mitiga con HUs pequeñas y patrones fijados.

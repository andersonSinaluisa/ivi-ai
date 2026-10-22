# Jarvis (ivi)

Asistente personal **local-first**: la PC Windows funciona sola, sin Internet; el **Cloud Core** en el VPS guarda la
memoria sincronizada, coordina devices y ejecuta jobs programados. Ningún modelo de IA corre en el VPS.

| Carpeta | Qué es | Dónde se ejecuta |
|---|---|---|
| [`api/`](api/README.md) | **Jarvis Cloud Core** (.NET 10): identidad, devices, tareas MQTT, memoria y sincronización, jobs, `jarvis-worker`. Incluye `deploy/` | **VPS** (Docker + PostgreSQL y nginx del host) |
| [`assistant/`](assistant/README.md) | App de escritorio (Tauri + React + Rust), agente con Qwen vía Ollama, memoria SQLite, servicio de voz (Python) | PC Windows |
| [`windows-agent/`](windows-agent/README.md) | Agente de herramientas de Windows (Rust); solo conexiones salientes | PC Windows |
| [`docs/`](docs/PROGRESS.md) | Auditoría de la arquitectura y estado de la refactorización Local-First + Cloud Core | — |
| [`tests/`](tests/) | Pruebas de extremo a extremo escritorio ↔ Cloud Core y del aprovisionamiento | Linux de desarrollo |

```text
PC Windows (assistant + windows-agent + Ollama)  ──HTTPS/MQTT salientes──►  VPS: Cloud Core (api/)  ──►  PostgreSQL
ESP32 / móvil / web  ─────────────────────────────────────────────────────►
```

Despliegue en el VPS: [`api/docs/06-infrastructure/contabo-deployment.md`](api/docs/06-infrastructure/contabo-deployment.md).
Decisiones de arquitectura: [`api/docs/08-decisions/`](api/docs/08-decisions/README.md).

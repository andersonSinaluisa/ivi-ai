# System Context

```mermaid
flowchart TB
    OWNER(["Owner"])
    subgraph Internet
        CLOUD["Jarvis Cloud<br/>REST · MQTT (Mosquitto) · PostgreSQL"]
        ESP["ESP32 / Mobile / Web<br/>(originan tareas)"]
    end
    subgraph PC["PC Windows — sesión interactiva del owner"]
        AGENT["jarvis-windows-agent<br/>(Rust, usuario estándar)"]
        CORE["jarvis-agent-core<br/>(futuro: razonamiento, Claude CLI u otro provider)"]
        BROWSER["Browser Tool Provider<br/>(futuro: Playwright, proceso aparte)"]
        WIN["Windows<br/>procesos · ventanas · UIA · filesystem"]
        CM["Credential Manager"]
        STATE[("SQLite local")]
    end
    ESP -- "MQTT/HTTPS" --> CLOUD
    OWNER -- "UI futura / REST" --> CLOUD
    CLOUD -- "MQTT/TLS 8883: task.dispatch, task.cancel" --> AGENT
    AGENT -- "MQTT/TLS: hello, heartbeat, task.ack/started/completed…" --> CLOUD
    AGENT --> WIN
    AGENT --> CM
    AGENT --> STATE
    CORE -. "Local Tool API (named pipe)" .-> AGENT
    AGENT -. "Tool Provider protocol" .-> BROWSER
    OWNER -- "CLI local: status, approvals, provision" --> AGENT
```

| Elemento | Relación con el agente | Confianza |
|---|---|---|
| Jarvis Cloud | Entrega tareas y recibe estados por MQTT v1 | **Parcial**: autentica el canal, pero su contenido se valida y autoriza localmente |
| Originadores (ESP32, mobile, owner) | Aparecen como `requestedBy` en la tarea | Identidad afirmada por el Cloud (hasta firma end-to-end) |
| Windows | Único lugar donde el agente produce efectos | Aplica sus propias barreras (UAC, UIPI, ACLs) |
| Credential Manager | Almacena el secreto del device | Protegido por DPAPI con la cuenta del usuario |
| SQLite local | Idempotencia, outbox, aprobaciones | Local, mismo usuario |
| Owner local | Configura política, aprueba, diagnostica por CLI | Autoridad máxima (controla la PC) |
| Agent Core (futuro) | Invoca tools por API local | Principal local sujeto a la misma política |
| Browser provider (futuro) | Proceso separado que implementa tools de navegador | Proveedor registrado localmente |

El agente **no** conoce PostgreSQL, REST (salvo opcionalmente `device-token`), SignalR ni ningún LLM.

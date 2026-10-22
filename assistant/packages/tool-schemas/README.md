# @jarvis/tool-schemas

Esquemas JSON (draft 2020-12) de los parámetros de las **herramientas locales** de Jarvis.

- El backend Rust los incrusta en compilación (`include_str!`), así que este directorio es la única fuente de verdad.
- Las herramientas del windows-agent **no** están aquí: sus esquemas llegan en vivo por `tools/list` de la Local Tool API.

| Herramienta | Riesgo | Origen |
|---|---|---|
| `get_current_time` | READ_ONLY | local |
| `get_system_info` | READ_ONLY | local |
| `save_memory` | LOW_RISK | local (Memory Engine; lo guardado por el agente queda sin verificar) |
| `search_memory` | READ_ONLY | local (Memory Engine) |

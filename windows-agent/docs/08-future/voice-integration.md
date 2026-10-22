# Voice Integration (fuera de alcance)

Voz (Whisper, TTS, wake word) está **explícitamente excluida** de este proyecto ([Scope](../00-overview/scope.md)).

## Dónde encaja cuando exista

La voz es un **frontend de entrada/salida** del Agent Core, no una capacidad del Windows Agent:

```text
Micrófono → STT (Whisper, local o remoto) → texto → jarvis-agent-core (goal) → tool calls → jarvis-windows-agent
jarvis-windows-agent → ToolResult → jarvis-agent-core → respuesta → TTS → altavoz
```

## Qué implica para el Windows Agent

| Tema | Decisión |
|---|---|
| Captura de audio | No la hace el Windows Agent. Si algún día se necesitara una tool `audio.*`, sería un provider separado con capability explícita (el micrófono es un sensor altamente sensible) |
| Reproducción de TTS | Tampoco; como mucho una tool futura `play_sound` R1 |
| Origen de la orden | Una orden por voz llega como invocación de `local:agent-core` (o como tarea del Cloud si la voz viene del ESP32/móvil). La política es la misma; **no** hay un principal "voz" con más privilegios |
| Confirmaciones de voz | No sustituyen a la aprobación local de R3 hasta que exista un mecanismo verificable (la voz puede venir de un altavoz, un vídeo o una llamada) |

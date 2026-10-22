# ADR-A02: Pulsar para hablar por defecto; palabra de activación opcional

## Estado

Aceptado, 2026-10-22.

## Contexto

No existe un modelo de palabra de activación verificado para «Jarvis» en español. openWakeWord trae `hey_jarvis` en inglés, con licencia no comercial.

## Decisión

- El modo funcional es **pulsar para hablar** (botón o barra espaciadora).
- El detector queda detrás de la interfaz `WakeWordDetector`, desactivado por defecto, con un adaptador de openWakeWord que acepta el modelo preentrenado o un `.onnx` propio.
- La activación por voz no se declara funcional hasta probarla con un procedimiento definido (docs/voice-wake-word.md).

## Consecuencias

- Nada se activa sin acción del usuario, lo que también evita grabar audio sin querer.
- Para manos libres hace falta entrenar o adquirir un modelo.

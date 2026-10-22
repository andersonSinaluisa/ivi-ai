# ADR-A03: Voz en un proceso Python separado

## Estado

Aceptado, 2026-10-22.

## Contexto

faster-whisper, openWakeWord y OpenVoice V2 son librerías de Python, y OpenVoice además depende de PyTorch.

## Decisión

- La voz es un servicio FastAPI en `127.0.0.1:8765`, con REST y un WebSocket de eventos y control, autenticado con un token local.
- La UI habla directamente con el servicio de voz. La transcripción se envía al agente como una instrucción más y la respuesta final vuelve al servicio para leerse en voz alta (`features/voice/voiceBridge.ts`).
- La voz no tiene privilegios propios: una orden hablada pasa por la misma política que una escrita.

## Consecuencias

- Un fallo de un modelo de voz no tumba la app: la UI muestra el estado y el chat sigue funcionando.
- Hay que arrancar dos procesos (`scripts/dev.ps1` lo hace).
- Pendiente: que Tauri arranque y supervise el servicio de voz (sidecar).

# Palabra de activación «Jarvis»

## Estado

**No activa.** Jarvis funciona con pulsar para hablar (botón o barra espaciadora). La activación por voz se considerará completa solo cuando haya un detector probado con tu voz, en tu micrófono y en español.

## Qué se investigó (octubre de 2026)

Fuente: [openwakeword en PyPI](https://pypi.org/project/openwakeword).

| Hecho | Consecuencia |
|---|---|
| openWakeWord 0.6.0 trae un modelo preentrenado **`hey_jarvis`** | Detecta «Hey Jarvis», no «Jarvis» a secas |
| Sus modelos preentrenados están entrenados con voces sintéticas en **inglés** | Con acento hispano puede fallar más; no está medido |
| Licencia de los modelos preentrenados: **CC BY-NC-SA 4.0** (no comercial). El código es Apache 2.0 | Sirve para uso personal, no para un producto comercial |
| En Windows solo funciona con **onnxruntime** (tflite no está soportado) | El adaptador usa `inference_framework="onnx"` |
| Se pueden entrenar modelos propios con los notebooks de Colab del proyecto (voz sintética + datos negativos, miles de ejemplos) | Es el camino para «Jarvis» en español |

## Lo que ya está preparado

- `app/wake_word/detector.py`: interfaz `WakeWordDetector`, `OpenWakeWordDetector` (modelo por nombre o ruta a `.onnx`) y `DisabledDetector`.
- El motor pasa a `LISTENING_WAKE_WORD` solo si el detector cargó, está habilitado y hay micrófono.
- Umbral configurable (`wake_word_threshold`), enfriamiento tras hablar y señal sonora al activarse.
- Pantalla de voz: muestra si el detector está disponible y por qué no.

## Probar «Hey Jarvis» (inglés)

```powershell
powershell -ExecutionPolicy Bypass -File scripts\install-wakeword.ps1
# Luego, en %LOCALAPPDATA%\Jarvis\voice.json: "wake_word_enabled": true   y reinicia el servicio
```

Pruébalo 20 veces en condiciones normales y otras 20 con ruido, y anota los aciertos y las falsas activaciones. Con menos de 90 % de aciertos o más de una falsa activación por hora, sube o baja `wake_word_threshold`, o entrena un modelo propio.

## Entrenar «Jarvis» en español

1. Abre el notebook de entrenamiento de openWakeWord (enlazado en su README) y usa como frase objetivo `jarvis`.
2. Genera las muestras positivas con un TTS que tenga voces en español. Las del notebook son inglesas: añade variantes como «yarvis» y «jarbis» para cubrir la pronunciación hispana.
3. Añade grabaciones tuyas reales: unas 50, de distintas distancias.
4. Exporta el modelo a `.onnx` y cópialo a `services/voice-service/models/wakeword/jarvis_es.onnx`.
5. En `voice.json`: `"wake_word_model": "models/wakeword/jarvis_es.onnx"` y `"wake_word_enabled": true`.
6. Repite la prueba de 20 + 20 de arriba antes de darlo por bueno.

Alternativas si openWakeWord no rinde: Porcupine de Picovoice (palabras personalizadas en español, pero requiere una clave de acceso y tiene licencia comercial) o un clasificador KWS pequeño entrenado aparte. Ambas encajan en el mismo `WakeWordDetector`.

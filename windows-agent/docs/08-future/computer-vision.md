# Computer Vision (P12, futuro)

Fuera de alcance del MVP. Visión **no** es la primera opción: es el penúltimo nivel de la [jerarquía de integración](../05-windows/windows-integration.md).

## Fallback

```text
Native API
   ↓ unavailable
UI Automation
   ↓ unavailable
(Application-specific provider)
   ↓ unavailable
Vision
   ↓
mouse/keyboard
```

## Por qué al final

| Problema | Detalle |
|---|---|
| Privacidad | Una captura de pantalla contiene **todo** lo visible: mensajes, notificaciones, documentos, contraseñas en claro de otras ventanas. Enviarla a un modelo remoto es exfiltración |
| Fiabilidad | Temas, DPI, idioma, animaciones y ventanas superpuestas cambian lo que "se ve" |
| Coste y latencia | Inferencia por cada paso |
| Verificación débil | "Parece que se pulsó" no es un estado observable semántico |

## Diseño previsto

- Provider externo (`jarvis-vision-provider`), mismo contrato de tool.
- Captura limitada a **una ventana** concreta (`Windows.Graphics.Capture` con el `HWND` objetivo), nunca pantalla completa por defecto; el borde amarillo de captura de Windows queda visible (transparencia para el usuario).
- Modelo **local** por defecto; envío a un modelo remoto solo con capability explícita y por principal.
- Toda acción derivada de visión (click en coordenadas) es **R3** con aprobación y lock `DESKTOP_INPUT`.
- Verificación posterior por un medio distinto a la misma visión cuando sea posible (UIA, proceso, archivo).
- Imágenes nunca se persisten en la BD ni en logs; solo hash.
- Sesión bloqueada → `SESSION_UNAVAILABLE`.

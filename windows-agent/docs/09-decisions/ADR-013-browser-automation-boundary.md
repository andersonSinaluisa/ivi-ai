# ADR-013: Navegador mediante provider Playwright separado

## Status

Accepted, 2026-09-19. Implementación en P11.

## Context

Buena parte del trabajo del usuario ocurre en el navegador. UIA ve poco del contenido web; mouse/teclado es frágil. Playwright ofrece control por DOM, pero trae su runtime (Node.js) y navegadores propios.

## Decision

- La automatización de navegador es un **Tool Provider separado** (`jarvis-browser-provider`), proceso propio, que habla con el Tool Host por named pipe con el mismo contrato de tool.
- Usa **Playwright** sobre un **perfil de navegador dedicado** a Jarvis, no el perfil personal.
- Política, riesgo, locks (`BROWSER:<profile>`), verificación por DOM y `ToolResult` los aplica el Tool Host.
- El Windows Agent no incluye Node.js, Playwright ni navegadores.

Detalle en [Browser Automation](../08-future/browser-automation.md).

## Alternatives

| Alternativa | Motivo de descarte |
|---|---|
| Playwright/CDP embebido en el agente | Arrastra un runtime; un crash del navegador afectaría al agente |
| UIA sobre el navegador | Árbol web limitado e inestable |
| Perfil personal del usuario | Expone cookies, sesiones y contraseñas a la automatización |
| Extensión del navegador | Canal adicional con permisos amplios; distribución por tienda |

## Consequences

- ✅ Core del agente pequeño; provider reemplazable.
- ⚠️ El usuario debe iniciar sesión en sitios dentro del perfil de Jarvis si quiere automatizarlos.
- ⚠️ Un proceso más que instalar y supervisar.

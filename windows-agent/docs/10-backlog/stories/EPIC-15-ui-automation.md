# EPIC-15 — UI Automation

Fase **P9**. Diseño en [UI Automation](../../05-windows/ui-automation.md) y [ADR-012](../../09-decisions/ADR-012-ui-automation-strategy.md). DoD común en [User Stories](../user-stories.md#definition-of-done-común).

> **Estado: implementada.** Selectores por ruta estructural descartados; el resto según lo diseñado. Ver [UI Automation](../../05-windows/ui-automation.md#diferencias-con-el-diseño-original).

---

## HU-WIN-064 — Host UIA en hilo STA

**Goal**: acceso a UI Automation seguro para el resto del agente.

**Description**: hilo STA dedicado con `IUIAutomation`, API de mensajes y DTOs.

**Acceptance Criteria**
1. `CoInitializeEx(COINIT_APARTMENTTHREADED)` en un hilo propio; los objetos COM nunca salen de él.
2. API asíncrona basada en canales (request + oneshot).
3. `CacheRequest` para propiedades comunes.
4. Timeout por llamada; si el hilo queda bloqueado, se abandona y se crea uno nuevo; la llamada falla con `TOOL_TIMEOUT`.
5. Sesión bloqueada → `SESSION_UNAVAILABLE` sin llamar a UIA.

**Dependencies**: HU-WIN-053.

**Technical Considerations**: bombeo de mensajes en el STA (`MsgWaitForMultipleObjectsEx`) para eventos UIA futuros.

**Tests**: integración con Notepad y Calculadora; app de prueba que se cuelga.

**Definition of Done**: DoD común.

---

## HU-WIN-065 — `find_window` y `find_element`

**Goal**: localizar ventanas y elementos por selectores semánticos.

**Description**: [Selectores](../../05-windows/ui-automation.md#selectores).

**Acceptance Criteria**
1. Selector con `automationId`, `controlType`, `name`, `className` y ruta estructural (con advertencia).
2. Sin coordenadas.
3. Más de una coincidencia → `ELEMENT_AMBIGUOUS`; ninguna → `ELEMENT_NOT_FOUND` tras `timeoutMs`.
4. Output con `ElementRef` (runtimeId, propiedades, bounds informativos).
5. R1, `windows.ui.read`, sin locks.

**Dependencies**: HU-WIN-064.

**Technical Considerations**: `FindFirst`/`FindAll` con condiciones combinadas y `TreeScope` acotado.

**Tests**: selectores contra Notepad (menú, documento) y Calculadora (botones por AutomationId).

**Definition of Done**: DoD común.

---

## HU-WIN-066 — `invoke_element`, `set_text`, `get_text`

**Goal**: actuar y leer sobre elementos con patrones UIA y verificar el efecto.

**Description**: [Tools previstas](../../05-windows/ui-automation.md#tools-previstas).

**Acceptance Criteria**
1. `invoke_element` usa `Invoke`/`Toggle`/`SelectionItem`/`ExpandCollapse` según el elemento; nunca `SendInput`.
2. `set_text` con `ValuePattern.SetValue`; verificación `Value == texto`.
3. `get_text` R1 sensible (redacción tras *settled*, nunca en logs).
4. Acciones con `DESKTOP_INPUT`; elementos en lista de acciones destructivas → R3.
5. Verificación por estado observable del elemento o de la ventana.

**Dependencies**: HU-WIN-065.

**Technical Considerations**: algunos controles exponen `LegacyIAccessible` en lugar de patrones modernos: tratarlo como fallback explícito.

**Tests**: Calculadora: `2 + 2 =` con invoke y lectura del resultado; Notepad: `set_text` y verificación.

**Definition of Done**: DoD común.

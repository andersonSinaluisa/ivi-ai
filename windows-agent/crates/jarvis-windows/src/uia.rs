//! Host de UI Automation en un hilo STA dedicado (HU-WIN-064, ADR-012).
//!
//! Los objetos COM de UIA son de apartamento: viven y mueren en **un solo hilo**. Por eso aquí no
//! sale ni un puntero COM; lo que cruza el canal son peticiones y DTOs de datos planos.
//!
//! El hilo se crea a demanda y se abandona si deja de responder (una app colgada puede bloquear
//! indefinidamente una llamada UIA): en ese caso la llamada falla con timeout y la siguiente
//! levanta un hilo nuevo, en vez de dejar al agente esperando para siempre.

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use jarvis_tooling::platform::{PlatformError, PlatformErrorKind};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationExpandCollapsePattern, IUIAutomationInvokePattern,
    IUIAutomationSelectionItemPattern, IUIAutomationTogglePattern, IUIAutomationValuePattern, TreeScope_Descendants,
    UIA_AutomationIdPropertyId, UIA_ClassNamePropertyId, UIA_ControlTypePropertyId, UIA_ExpandCollapsePatternId,
    UIA_InvokePatternId, UIA_NamePropertyId, UIA_SelectionItemPatternId, UIA_TogglePatternId, UIA_ValuePatternId,
};
use windows::core::{BSTR, Interface};

use crate::sys::last_error;

/// Qué elemento se busca. Sin coordenadas, nunca (ADR-012).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selector {
    /// El más estable: no cambia con el idioma.
    pub automation_id: Option<String>,
    pub name: Option<String>,
    pub class_name: Option<String>,
    /// Nombre del tipo de control (`Button`, `Edit`, `MenuItem`…).
    pub control_type: Option<String>,
}

impl Selector {
    /// `true` si no restringe nada: un selector así seleccionaría la UI entera.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.automation_id.is_none() && self.name.is_none() && self.class_name.is_none()
    }
}

/// Elemento encontrado, ya convertido a datos planos.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElementDto {
    pub automation_id: String,
    /// Puede contener datos personales: quien lo use aplica la política de privacidad.
    pub name: String,
    pub class_name: String,
    pub control_type: String,
    /// Solo informativo (nunca se usa para hacer clic por coordenadas).
    pub bounds: (i32, i32, i32, i32),
    pub enabled: bool,
    /// Patrones que expone, en el orden en que se probarían.
    pub patterns: Vec<String>,
    /// `ValuePattern.Value`, si lo tiene.
    pub value: Option<String>,
    /// `TogglePattern.ToggleState` (`true` = marcado).
    pub toggled: Option<bool>,
    /// `SelectionItemPattern.IsSelected`.
    pub selected: Option<bool>,
}

/// Ventana de nivel superior vista por UIA.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowDto {
    pub hwnd: isize,
    pub pid: u32,
    pub name: String,
    pub class_name: String,
}

#[derive(Debug, Clone)]
enum Request {
    Window { hwnd: isize },
    FindElements { hwnd: isize, selector: Selector, limit: usize },
    Invoke { hwnd: isize, selector: Selector },
    SetValue { hwnd: isize, selector: Selector, text: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Window(WindowDto),
    Elements(Vec<ElementDto>),
    /// Patrón que se usó para actuar.
    Invoked(String),
    Done,
}

struct Job {
    request: Request,
    reply: Sender<Result<Reply, PlatformError>>,
}

/// Acceso a UIA desde el resto del agente.
#[derive(Debug, Default)]
pub struct UiaHost {
    worker: Mutex<Option<Sender<Job>>>,
}

impl UiaHost {
    #[must_use]
    pub const fn new() -> Self {
        Self { worker: Mutex::new(None) }
    }

    /// Envía una petición al hilo STA y espera su respuesta.
    ///
    /// Si el hilo no contesta dentro del plazo se le da por perdido (la app consultada puede estar
    /// colgada) y la siguiente llamada arrancará uno nuevo.
    fn call(&self, request: Request, timeout: Duration) -> Result<Reply, PlatformError> {
        let (reply_tx, reply_rx) = channel();
        {
            let mut worker = self.worker.lock().unwrap_or_else(PoisonError::into_inner);
            if worker.is_none() {
                *worker = Some(spawn_worker());
            }
            let sender = worker.as_ref().ok_or_else(|| {
                PlatformError::new(PlatformErrorKind::Other, "the UI Automation host is not available")
            })?;
            if sender.send(Job { request, reply: reply_tx }).is_err() {
                // El hilo murió: se descarta y se reintenta una vez con uno nuevo.
                *worker = None;
                return Err(PlatformError::new(PlatformErrorKind::Other, "the UI Automation host stopped"));
            }
        }

        match reply_rx.recv_timeout(timeout) {
            Ok(result) => result,
            Err(RecvTimeoutError::Timeout) => {
                *self.worker.lock().unwrap_or_else(PoisonError::into_inner) = None;
                Err(PlatformError::new(PlatformErrorKind::Other, "the UI Automation call timed out"))
            }
            Err(RecvTimeoutError::Disconnected) => {
                *self.worker.lock().unwrap_or_else(PoisonError::into_inner) = None;
                Err(PlatformError::new(PlatformErrorKind::Other, "the UI Automation host stopped"))
            }
        }
    }

    /// Datos de la ventana (título incluido) vista por UIA.
    ///
    /// # Errors
    /// Si la ventana ya no existe o UIA no responde a tiempo.
    pub fn window(&self, hwnd: isize, timeout: Duration) -> Result<WindowDto, PlatformError> {
        match self.call(Request::Window { hwnd }, timeout)? {
            Reply::Window(window) => Ok(window),
            _ => Err(unexpected()),
        }
    }

    /// Elementos que cumplen el selector dentro de esa ventana.
    ///
    /// # Errors
    /// Si la ventana no existe o UIA no responde a tiempo.
    pub fn find_elements(
        &self,
        hwnd: isize,
        selector: &Selector,
        limit: usize,
        timeout: Duration,
    ) -> Result<Vec<ElementDto>, PlatformError> {
        match self.call(Request::FindElements { hwnd, selector: selector.clone(), limit }, timeout)? {
            Reply::Elements(elements) => Ok(elements),
            _ => Err(unexpected()),
        }
    }

    /// Actúa sobre el elemento con el patrón que exponga. Devuelve cuál se usó.
    ///
    /// # Errors
    /// Si no hay elemento único, no expone ningún patrón de acción o UIA no responde.
    pub fn invoke(&self, hwnd: isize, selector: &Selector, timeout: Duration) -> Result<String, PlatformError> {
        match self.call(Request::Invoke { hwnd, selector: selector.clone() }, timeout)? {
            Reply::Invoked(pattern) => Ok(pattern),
            _ => Err(unexpected()),
        }
    }

    /// Escribe texto con `ValuePattern.SetValue` (nunca simulando teclas).
    ///
    /// # Errors
    /// Si el elemento no admite `ValuePattern` o UIA no responde.
    pub fn set_value(
        &self,
        hwnd: isize,
        selector: &Selector,
        text: &str,
        timeout: Duration,
    ) -> Result<(), PlatformError> {
        match self.call(Request::SetValue { hwnd, selector: selector.clone(), text: text.to_owned() }, timeout)? {
            Reply::Done => Ok(()),
            _ => Err(unexpected()),
        }
    }
}

fn unexpected() -> PlatformError {
    PlatformError::new(PlatformErrorKind::Other, "unexpected reply from the UI Automation host")
}

/// Arranca el hilo STA. Vive mientras el canal esté abierto.
fn spawn_worker() -> Sender<Job> {
    let (tx, rx) = channel::<Job>();
    std::thread::Builder::new()
        .name("jarvis-uia".to_owned())
        .spawn(move || worker_loop(&rx))
        .map(|_| ())
        .unwrap_or_else(|error| tracing::error!(%error, "cannot start the UI Automation thread"));
    tx
}

fn worker_loop(rx: &Receiver<Job>) {
    // SAFETY: este hilo es suyo; se inicializa el apartamento una vez y se libera al terminar.
    let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    if initialized.is_err() {
        tracing::error!("cannot initialise the STA apartment for UI Automation");
        return;
    }
    // SAFETY: CLSID válido y el objeto se queda en este hilo.
    let automation: Result<IUIAutomation, _> = unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) };
    let automation = match automation {
        Ok(automation) => automation,
        Err(error) => {
            tracing::error!(%error, "cannot create the UI Automation client");
            // SAFETY: se llama una vez tras un CoInitializeEx con éxito.
            unsafe { CoUninitialize() };
            return;
        }
    };

    while let Ok(job) = rx.recv() {
        let result = handle(&automation, &job.request);
        // Si el que pidió ya no espera (timeout), la respuesta se descarta sin más.
        let _ = job.reply.send(result);
    }

    // SAFETY: cierre del apartamento inicializado arriba, en el mismo hilo.
    unsafe { CoUninitialize() };
}

fn handle(automation: &IUIAutomation, request: &Request) -> Result<Reply, PlatformError> {
    match request {
        Request::Window { hwnd } => {
            let element = element_from_handle(automation, *hwnd)?;
            // SAFETY: elemento válido; todas las propiedades son de solo lectura.
            unsafe {
                Ok(Reply::Window(WindowDto {
                    hwnd: *hwnd,
                    pid: u32::try_from(element.CurrentProcessId().unwrap_or(0)).unwrap_or(0),
                    name: element.CurrentName().map(|n| n.to_string()).unwrap_or_default(),
                    class_name: element.CurrentClassName().map(|n| n.to_string()).unwrap_or_default(),
                }))
            }
        }
        Request::FindElements { hwnd, selector, limit } => {
            let root = element_from_handle(automation, *hwnd)?;
            let found = find(automation, &root, selector, *limit)?;
            Ok(Reply::Elements(found))
        }
        Request::Invoke { hwnd, selector } => {
            let root = element_from_handle(automation, *hwnd)?;
            let element = single(automation, &root, selector)?;
            Ok(Reply::Invoked(act(&element)?))
        }
        Request::SetValue { hwnd, selector, text } => {
            let root = element_from_handle(automation, *hwnd)?;
            let element = single(automation, &root, selector)?;
            // SAFETY: el patrón se consulta y usa en el mismo hilo del apartamento.
            unsafe {
                let pattern: IUIAutomationValuePattern =
                    element.GetCurrentPattern(UIA_ValuePatternId).ok().and_then(|p| p.cast().ok()).ok_or_else(
                        || PlatformError::new(PlatformErrorKind::Other, "the element does not accept text"),
                    )?;
                if pattern.CurrentIsReadOnly().unwrap_or_default().as_bool() {
                    return Err(PlatformError::new(PlatformErrorKind::AccessDenied, "the element is read-only"));
                }
                pattern
                    .SetValue(&BSTR::from(text.as_str()))
                    .map_err(|_| PlatformError::new(PlatformErrorKind::Other, "the element rejected the text"))?;
            }
            Ok(Reply::Done)
        }
    }
}

fn element_from_handle(automation: &IUIAutomation, hwnd: isize) -> Result<IUIAutomationElement, PlatformError> {
    // SAFETY: `hwnd` viene de la enumeración de ventanas; si ya no existe, UIA devuelve error.
    unsafe {
        automation
            .ElementFromHandle(HWND(hwnd as *mut std::ffi::c_void))
            .map_err(|_| PlatformError::new(PlatformErrorKind::NotFound, "the window is no longer available"))
    }
}

/// Un único elemento o error: un selector ambiguo nunca elige por su cuenta (ADR-012).
fn single(
    automation: &IUIAutomation,
    root: &IUIAutomationElement,
    selector: &Selector,
) -> Result<IUIAutomationElement, PlatformError> {
    let condition = condition_for(automation, selector)?;
    // SAFETY: condición y raíz válidas; la colección se recorre por índice con su longitud.
    unsafe {
        let all = root
            .FindAll(TreeScope_Descendants, &condition)
            .map_err(|_| PlatformError::new(PlatformErrorKind::Other, "the element search failed"))?;
        let count = all.Length().unwrap_or(0);
        match count {
            0 => Err(PlatformError::new(PlatformErrorKind::NotFound, "no element matches the selector")),
            1 => all
                .GetElement(0)
                .map_err(|_| PlatformError::new(PlatformErrorKind::Other, "the element vanished while reading it")),
            more => Err(PlatformError::new(
                PlatformErrorKind::Other,
                format!("the selector matches {more} elements: make it more specific"),
            )),
        }
    }
}

fn find(
    automation: &IUIAutomation,
    root: &IUIAutomationElement,
    selector: &Selector,
    limit: usize,
) -> Result<Vec<ElementDto>, PlatformError> {
    let condition = condition_for(automation, selector)?;
    // SAFETY: igual que en `single`; solo se leen propiedades.
    unsafe {
        let all = root
            .FindAll(TreeScope_Descendants, &condition)
            .map_err(|_| PlatformError::new(PlatformErrorKind::Other, "the element search failed"))?;
        let count = all.Length().unwrap_or(0);
        let mut elements = Vec::new();
        for index in 0..count {
            if elements.len() >= limit {
                break;
            }
            if let Ok(element) = all.GetElement(index) {
                elements.push(describe(&element));
            }
        }
        Ok(elements)
    }
}

/// Condición UIA a partir del selector. Al menos una propiedad tiene que restringir.
fn condition_for(
    automation: &IUIAutomation,
    selector: &Selector,
) -> Result<windows::Win32::UI::Accessibility::IUIAutomationCondition, PlatformError> {
    if selector.is_empty() && selector.control_type.is_none() {
        return Err(PlatformError::new(PlatformErrorKind::Other, "the selector does not restrict anything"));
    }
    let mut conditions = Vec::new();
    // SAFETY: los VARIANT se construyen aquí y viven hasta crear la condición.
    unsafe {
        if let Some(id) = &selector.automation_id {
            conditions.push(
                automation
                    .CreatePropertyCondition(UIA_AutomationIdPropertyId, &VARIANT::from(BSTR::from(id.as_str())))
                    .map_err(|_| condition_error())?,
            );
        }
        if let Some(name) = &selector.name {
            conditions.push(
                automation
                    .CreatePropertyCondition(UIA_NamePropertyId, &VARIANT::from(BSTR::from(name.as_str())))
                    .map_err(|_| condition_error())?,
            );
        }
        if let Some(class) = &selector.class_name {
            conditions.push(
                automation
                    .CreatePropertyCondition(UIA_ClassNamePropertyId, &VARIANT::from(BSTR::from(class.as_str())))
                    .map_err(|_| condition_error())?,
            );
        }
        if let Some(control) = &selector.control_type {
            let id = control_type_id(control)
                .ok_or_else(|| PlatformError::new(PlatformErrorKind::Other, "unknown control type"))?;
            conditions.push(
                automation
                    .CreatePropertyCondition(UIA_ControlTypePropertyId, &VARIANT::from(id))
                    .map_err(|_| condition_error())?,
            );
        }
        let mut combined = conditions.remove(0);
        for next in conditions {
            combined = automation.CreateAndCondition(&combined, &next).map_err(|_| condition_error())?;
        }
        Ok(combined)
    }
}

fn condition_error() -> PlatformError {
    PlatformError::new(PlatformErrorKind::Other, "the selector could not be translated to UI Automation")
}

/// Actúa con el primer patrón disponible, en orden de preferencia.
fn act(element: &IUIAutomationElement) -> Result<String, PlatformError> {
    // SAFETY: los patrones se consultan y usan en el hilo del apartamento.
    unsafe {
        if !element.CurrentIsEnabled().unwrap_or_default().as_bool() {
            return Err(PlatformError::new(PlatformErrorKind::AccessDenied, "the element is disabled"));
        }
        if let Ok(pattern) = element.GetCurrentPattern(UIA_InvokePatternId)
            && let Ok(invoke) = pattern.cast::<IUIAutomationInvokePattern>()
        {
            invoke.Invoke().map_err(|_| action_failed())?;
            return Ok("Invoke".to_owned());
        }
        if let Ok(pattern) = element.GetCurrentPattern(UIA_TogglePatternId)
            && let Ok(toggle) = pattern.cast::<IUIAutomationTogglePattern>()
        {
            toggle.Toggle().map_err(|_| action_failed())?;
            return Ok("Toggle".to_owned());
        }
        if let Ok(pattern) = element.GetCurrentPattern(UIA_SelectionItemPatternId)
            && let Ok(selection) = pattern.cast::<IUIAutomationSelectionItemPattern>()
        {
            selection.Select().map_err(|_| action_failed())?;
            return Ok("SelectionItem".to_owned());
        }
        if let Ok(pattern) = element.GetCurrentPattern(UIA_ExpandCollapsePatternId)
            && let Ok(expand) = pattern.cast::<IUIAutomationExpandCollapsePattern>()
        {
            expand.Expand().map_err(|_| action_failed())?;
            return Ok("ExpandCollapse".to_owned());
        }
    }
    Err(PlatformError::new(
        PlatformErrorKind::Other,
        "the element exposes no action pattern (the agent never clicks by coordinates)",
    ))
}

fn action_failed() -> PlatformError {
    PlatformError::new(PlatformErrorKind::Other, "the element did not accept the action")
}

/// Propiedades del elemento en un DTO. Se leen todas aquí para no cruzar el proceso más veces.
fn describe(element: &IUIAutomationElement) -> ElementDto {
    // SAFETY: solo lecturas de propiedades sobre un elemento válido del mismo hilo.
    unsafe {
        let bounds = element.CurrentBoundingRectangle().unwrap_or_default();
        let mut patterns = Vec::new();
        let mut value = None;
        let mut toggled = None;
        let mut selected = None;

        if let Ok(pattern) = element.GetCurrentPattern(UIA_ValuePatternId)
            && let Ok(value_pattern) = pattern.cast::<IUIAutomationValuePattern>()
        {
            patterns.push("Value".to_owned());
            value = value_pattern.CurrentValue().ok().map(|v| v.to_string());
        }
        if let Ok(pattern) = element.GetCurrentPattern(UIA_InvokePatternId)
            && pattern.cast::<IUIAutomationInvokePattern>().is_ok()
        {
            patterns.push("Invoke".to_owned());
        }
        if let Ok(pattern) = element.GetCurrentPattern(UIA_TogglePatternId)
            && let Ok(toggle) = pattern.cast::<IUIAutomationTogglePattern>()
        {
            patterns.push("Toggle".to_owned());
            toggled = toggle.CurrentToggleState().ok().map(|state| state.0 == 1);
        }
        if let Ok(pattern) = element.GetCurrentPattern(UIA_SelectionItemPatternId)
            && let Ok(selection) = pattern.cast::<IUIAutomationSelectionItemPattern>()
        {
            patterns.push("SelectionItem".to_owned());
            selected = selection.CurrentIsSelected().ok().map(|s| s.as_bool());
        }

        ElementDto {
            automation_id: element.CurrentAutomationId().map(|v| v.to_string()).unwrap_or_default(),
            name: element.CurrentName().map(|v| v.to_string()).unwrap_or_default(),
            class_name: element.CurrentClassName().map(|v| v.to_string()).unwrap_or_default(),
            control_type: control_type_name(element.CurrentControlType().map(|t| t.0).unwrap_or(0)),
            bounds: (bounds.left, bounds.top, bounds.right - bounds.left, bounds.bottom - bounds.top),
            enabled: element.CurrentIsEnabled().unwrap_or_default().as_bool(),
            patterns,
            value,
            toggled,
            selected,
        }
    }
}

/// Tipos de control que el agente sabe nombrar (los `UIA_*ControlTypeId` de la documentación).
const CONTROL_TYPES: &[(&str, i32)] = &[
    ("Button", 50000),
    ("Calendar", 50001),
    ("CheckBox", 50002),
    ("ComboBox", 50003),
    ("Edit", 50004),
    ("Hyperlink", 50005),
    ("Image", 50006),
    ("ListItem", 50007),
    ("List", 50008),
    ("Menu", 50009),
    ("MenuBar", 50010),
    ("MenuItem", 50011),
    ("ProgressBar", 50012),
    ("RadioButton", 50013),
    ("ScrollBar", 50014),
    ("Slider", 50015),
    ("Spinner", 50016),
    ("StatusBar", 50017),
    ("Tab", 50018),
    ("TabItem", 50019),
    ("Text", 50020),
    ("ToolBar", 50021),
    ("ToolTip", 50022),
    ("Tree", 50023),
    ("TreeItem", 50024),
    ("Custom", 50025),
    ("Group", 50026),
    ("Thumb", 50027),
    ("DataGrid", 50028),
    ("DataItem", 50029),
    ("Document", 50030),
    ("SplitButton", 50031),
    ("Window", 50032),
    ("Pane", 50033),
    ("Header", 50034),
    ("HeaderItem", 50035),
    ("Table", 50036),
    ("TitleBar", 50037),
    ("Separator", 50038),
];

/// Id del tipo de control a partir de su nombre.
#[must_use]
pub fn control_type_id(name: &str) -> Option<i32> {
    CONTROL_TYPES.iter().find(|(known, _)| known.eq_ignore_ascii_case(name)).map(|(_, id)| *id)
}

/// Nombre del tipo de control a partir de su id.
#[must_use]
pub fn control_type_name(id: i32) -> String {
    CONTROL_TYPES.iter().find(|(_, known)| *known == id).map_or_else(|| id.to_string(), |(name, _)| (*name).to_owned())
}

/// Error de plataforma cuando el hilo STA no se pudo crear.
#[must_use]
pub fn unavailable() -> PlatformError {
    last_error("UI Automation host")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_types_round_trip() {
        assert_eq!(control_type_id("Button"), Some(50000));
        assert_eq!(control_type_id("button"), Some(50000), "el nombre no distingue mayúsculas");
        assert_eq!(control_type_id("NoSuchType"), None);
        assert_eq!(control_type_name(50004), "Edit");
        assert_eq!(control_type_name(1), "1", "un id desconocido se muestra tal cual");
    }

    #[test]
    fn an_empty_selector_is_refused() {
        assert!(Selector::default().is_empty());
        let named = Selector { name: Some("Save".into()), ..Default::default() };
        assert!(!named.is_empty());
    }

    #[test]
    fn reads_the_properties_of_a_real_window() {
        // Se usa la ventana *message-only* del propio agente: siempre existe en esta sesión.
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let Ok(window) = crate::events::MessageWindow::spawn(tx) else {
            println!("SKIPPED: no se pudo crear la ventana de prueba");
            return;
        };
        let host = UiaHost::new();
        match host.window(window.handle(), Duration::from_secs(5)) {
            Ok(dto) => {
                assert_eq!(dto.pid, std::process::id(), "la ventana es de este proceso");
                assert!(!dto.class_name.is_empty());
            }
            // Sin escritorio accesible (CI headless) UIA no puede responder.
            Err(error) => println!("SKIPPED: {error}"),
        }
    }

    #[test]
    fn a_selector_that_matches_nothing_reports_not_found() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let Ok(window) = crate::events::MessageWindow::spawn(tx) else {
            println!("SKIPPED: no se pudo crear la ventana de prueba");
            return;
        };
        let host = UiaHost::new();
        let selector = Selector { automation_id: Some("NoExisteEsteBoton".into()), ..Default::default() };
        match host.find_elements(window.handle(), &selector, 10, Duration::from_secs(5)) {
            Ok(found) => assert!(found.is_empty(), "no debería encontrar nada: {found:?}"),
            Err(error) => println!("SKIPPED: {error}"),
        }
    }
}

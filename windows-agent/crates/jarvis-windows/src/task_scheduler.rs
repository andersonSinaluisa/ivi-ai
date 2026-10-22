//! Arranque del agente mediante una **Scheduled Task por usuario** disparada al iniciar sesión
//! (ADR-010). No se usa un Windows Service: en Session 0 no hay escritorio del usuario.
//!
//! La tarea se registra con `LeastPrivilege` e `InteractiveToken`, así que la crea el propio usuario
//! sin necesidad de administrador y nunca eleva.

use std::path::Path;

use jarvis_tooling::platform::{PlatformError, PlatformErrorKind};
use windows::Win32::Foundation::{VARIANT_FALSE, VARIANT_TRUE};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::Win32::System::TaskScheduler::{
    IExecAction, ILogonTrigger, ITaskService, TASK_ACTION_EXEC, TASK_CREATE_OR_UPDATE, TASK_INSTANCES_IGNORE_NEW,
    TASK_LOGON_INTERACTIVE_TOKEN, TASK_RUNLEVEL_LUA, TASK_TRIGGER_LOGON, TaskScheduler,
};
use windows::Win32::System::Variant::VARIANT;
use windows::core::{BSTR, Interface};

/// Nombre de la tarea en la biblioteca del Programador de tareas.
pub const TASK_NAME: &str = "Jarvis Windows Agent";

/// Retraso tras el inicio de sesión: deja arrancar primero al escritorio del usuario.
const LOGON_DELAY: &str = "PT15S";
/// Reinicios automáticos si el proceso cae.
const RESTART_INTERVAL: &str = "PT1M";
const RESTART_COUNT: i32 = 3;

/// Datos de la tarea a registrar.
#[derive(Debug, Clone)]
pub struct LogonTask<'a> {
    pub executable: &'a Path,
    pub arguments: &'a str,
    pub working_directory: &'a Path,
    /// SID o `DOMINIO\usuario` del usuario que la ejecutará (el propio).
    pub user_id: &'a str,
}

/// Inicializa COM en el hilo durante la llamada y lo libera al salir.
struct ComScope(bool);

impl ComScope {
    fn enter() -> Self {
        // SAFETY: se equilibra con CoUninitialize en Drop cuando la inicialización tuvo éxito.
        let result = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        Self(result.is_ok())
    }
}

impl Drop for ComScope {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: solo se llama si CoInitializeEx tuvo éxito en este hilo.
            unsafe { CoUninitialize() };
        }
    }
}

fn com_error(context: &str, error: &windows::core::Error) -> PlatformError {
    let kind =
        if error.code().0 == -2_147_024_891 { PlatformErrorKind::AccessDenied } else { PlatformErrorKind::Other };
    PlatformError::new(kind, format!("{context}: {error}"))
}

fn service() -> Result<ITaskService, PlatformError> {
    // SAFETY: COM ya está inicializado por el llamante (ComScope); el CLSID es constante.
    unsafe {
        let service: ITaskService = CoCreateInstance(&TaskScheduler, None, CLSCTX_INPROC_SERVER)
            .map_err(|e| com_error("CoCreateInstance(TaskScheduler)", &e))?;
        let empty = VARIANT::default();
        service.Connect(&empty, &empty, &empty, &empty).map_err(|e| com_error("ITaskService::Connect", &e))?;
        Ok(service)
    }
}

/// Registra (o actualiza) la tarea de inicio de sesión.
///
/// # Errors
/// `AccessDenied` si el usuario no puede registrar tareas; `Other` para el resto de fallos COM.
pub fn register_logon_task(task: &LogonTask<'_>) -> Result<(), PlatformError> {
    let _com = ComScope::enter();
    let service = service()?;
    // SAFETY: todos los objetos COM provienen del servicio y viven hasta el final de la función.
    unsafe {
        let folder = service.GetFolder(&BSTR::from("\\")).map_err(|e| com_error("GetFolder", &e))?;
        let definition = service.NewTask(0).map_err(|e| com_error("NewTask", &e))?;

        let info = definition.RegistrationInfo().map_err(|e| com_error("RegistrationInfo", &e))?;
        info.SetAuthor(&BSTR::from("Jarvis")).map_err(|e| com_error("SetAuthor", &e))?;
        info.SetDescription(&BSTR::from(
            "Ejecuta el Jarvis Windows Agent en la sesión del usuario (ADR-010: nunca como servicio ni elevado).",
        ))
        .map_err(|e| com_error("SetDescription", &e))?;

        // Principal: el propio usuario, con privilegios mínimos.
        let principal = definition.Principal().map_err(|e| com_error("Principal", &e))?;
        principal.SetUserId(&BSTR::from(task.user_id)).map_err(|e| com_error("SetUserId", &e))?;
        principal.SetLogonType(TASK_LOGON_INTERACTIVE_TOKEN).map_err(|e| com_error("SetLogonType", &e))?;
        principal.SetRunLevel(TASK_RUNLEVEL_LUA).map_err(|e| com_error("SetRunLevel", &e))?;

        // Disparador: inicio de sesión de este usuario, con retraso.
        let triggers = definition.Triggers().map_err(|e| com_error("Triggers", &e))?;
        let trigger = triggers.Create(TASK_TRIGGER_LOGON).map_err(|e| com_error("Triggers::Create", &e))?;
        let logon: ILogonTrigger = trigger.cast().map_err(|e| com_error("cast ILogonTrigger", &e))?;
        logon.SetUserId(&BSTR::from(task.user_id)).map_err(|e| com_error("trigger SetUserId", &e))?;
        logon.SetDelay(&BSTR::from(LOGON_DELAY)).map_err(|e| com_error("trigger SetDelay", &e))?;

        // Acción: el binario instalado, sin elevación.
        let actions = definition.Actions().map_err(|e| com_error("Actions", &e))?;
        let action = actions.Create(TASK_ACTION_EXEC).map_err(|e| com_error("Actions::Create", &e))?;
        let exec: IExecAction = action.cast().map_err(|e| com_error("cast IExecAction", &e))?;
        exec.SetPath(&BSTR::from(task.executable.to_string_lossy().as_ref())).map_err(|e| com_error("SetPath", &e))?;
        if !task.arguments.is_empty() {
            exec.SetArguments(&BSTR::from(task.arguments)).map_err(|e| com_error("SetArguments", &e))?;
        }
        exec.SetWorkingDirectory(&BSTR::from(task.working_directory.to_string_lossy().as_ref()))
            .map_err(|e| com_error("SetWorkingDirectory", &e))?;

        // Ajustes: sin límite de duración, reinicio ante caídas y sin depender de la batería.
        let settings = definition.Settings().map_err(|e| com_error("Settings", &e))?;
        settings.SetEnabled(VARIANT_TRUE).map_err(|e| com_error("SetEnabled", &e))?;
        settings.SetStartWhenAvailable(VARIANT_TRUE).map_err(|e| com_error("SetStartWhenAvailable", &e))?;
        settings
            .SetDisallowStartIfOnBatteries(VARIANT_FALSE)
            .map_err(|e| com_error("SetDisallowStartIfOnBatteries", &e))?;
        settings.SetStopIfGoingOnBatteries(VARIANT_FALSE).map_err(|e| com_error("SetStopIfGoingOnBatteries", &e))?;
        settings.SetExecutionTimeLimit(&BSTR::from("PT0S")).map_err(|e| com_error("SetExecutionTimeLimit", &e))?;
        settings.SetMultipleInstances(TASK_INSTANCES_IGNORE_NEW).map_err(|e| com_error("SetMultipleInstances", &e))?;
        // Intervalo antes que contador: es el orden que documenta Microsoft para `RestartOnFailure`
        // (ambos valores acaban en `<RestartOnFailure><Count>3</Count><Interval>PT1M</Interval>`).
        settings.SetRestartInterval(&BSTR::from(RESTART_INTERVAL)).map_err(|e| com_error("SetRestartInterval", &e))?;
        settings.SetRestartCount(RESTART_COUNT).map_err(|e| com_error("SetRestartCount", &e))?;

        let empty = VARIANT::default();
        folder
            .RegisterTaskDefinition(
                &BSTR::from(TASK_NAME),
                &definition,
                TASK_CREATE_OR_UPDATE.0,
                &empty,
                &empty,
                TASK_LOGON_INTERACTIVE_TOKEN,
                &empty,
            )
            .map_err(|e| com_error("RegisterTaskDefinition", &e))?;
        Ok(())
    }
}

/// Borra la tarea. Devuelve `false` si no existía.
///
/// # Errors
/// Fallos COM distintos de "no existe".
pub fn unregister_task() -> Result<bool, PlatformError> {
    let _com = ComScope::enter();
    let service = service()?;
    // SAFETY: objetos COM del propio servicio.
    unsafe {
        let folder = service.GetFolder(&BSTR::from("\\")).map_err(|e| com_error("GetFolder", &e))?;
        if folder.GetTask(&BSTR::from(TASK_NAME)).is_err() {
            return Ok(false);
        }
        folder.DeleteTask(&BSTR::from(TASK_NAME), 0).map_err(|e| com_error("DeleteTask", &e))?;
        Ok(true)
    }
}

/// `true` si la tarea está registrada.
#[must_use]
pub fn task_exists() -> bool {
    let _com = ComScope::enter();
    let Ok(service) = service() else { return false };
    // SAFETY: objetos COM del propio servicio.
    unsafe { service.GetFolder(&BSTR::from("\\")).and_then(|folder| folder.GetTask(&BSTR::from(TASK_NAME))).is_ok() }
}

/// Arranca la tarea recién registrada (equivalente a `install --start`).
///
/// # Errors
/// Si la tarea no existe o el Programador de tareas rechaza la ejecución.
pub fn run_task() -> Result<(), PlatformError> {
    let _com = ComScope::enter();
    let service = service()?;
    // SAFETY: objetos COM del propio servicio.
    unsafe {
        let folder = service.GetFolder(&BSTR::from("\\")).map_err(|e| com_error("GetFolder", &e))?;
        let task = folder.GetTask(&BSTR::from(TASK_NAME)).map_err(|e| com_error("GetTask", &e))?;
        task.Run(&VARIANT::default()).map_err(|e| com_error("Run", &e))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Registra y borra una tarea real: comprueba que un usuario estándar puede hacerlo (ADR-010).
    #[test]
    fn registers_and_removes_the_logon_task() {
        let sid = crate::sys::user_sid_string().expect("sid");
        let executable = std::path::PathBuf::from(crate::sys::expand_env(r"%SystemRoot%\System32\notepad.exe"));
        let directory = executable.parent().expect("parent").to_path_buf();
        let task = LogonTask {
            executable: &executable,
            arguments: "--test-only",
            working_directory: &directory,
            user_id: &sid,
        };

        // Si ya existe una tarea real instalada, el test no debe tocarla.
        if task_exists() {
            eprintln!("SKIPPED: ya hay una tarea '{TASK_NAME}' instalada");
            return;
        }
        register_logon_task(&task).expect("register");
        assert!(task_exists());
        assert!(unregister_task().expect("unregister"));
        assert!(!task_exists());
        assert!(!unregister_task().expect("unregister twice"), "borrar dos veces no es un error");
    }
}

//! Ejecución de comandos **del catálogo local** (HU-WIN-058).
//!
//! No existe ningún camino desde el Cloud hasta una línea de comandos: aquí solo llegan un
//! ejecutable absoluto y una lista de argumentos ya construida por la política a partir de una
//! plantilla con parámetros validados. Este módulo se ocupa de lo demás:
//!
//! - el hijo nace **suspendido**, se mete en un Job Object propio y solo entonces se reanuda, así
//!   que no puede lanzar nietos fuera del job ni siquiera durante un instante;
//! - el job muere con el handle (`KILL_ON_JOB_CLOSE`) y con el timeout, arrastrando a toda la
//!   descendencia;
//! - la salida se lee acotada: pasado el límite se sigue drenando el pipe (para no bloquear al
//!   hijo) pero no se guarda nada más.

use std::ffi::c_void;
use std::io::Read;
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use jarvis_tooling::platform::{CancelFlag, CommandOutput, CommandSpec, PlatformError, PlatformErrorKind};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_ACTIVE_PROCESS,
    JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_PROCESS_MEMORY,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation, SetInformationJobObject,
    TerminateJobObject,
};
use windows::Win32::System::Threading::{
    CREATE_NO_WINDOW, CREATE_SUSPENDED, OpenThread, ResumeThread, THREAD_SUSPEND_RESUME,
};
use windows::core::PCWSTR;

use crate::sys::{OwnedHandle, last_error};

/// Cuántos procesos como mucho puede haber vivos dentro del job (el comando y sus ayudantes).
const MAX_ACTIVE_PROCESSES: u32 = 16;
/// Memoria máxima por proceso dentro del job.
const MEMORY_LIMIT_BYTES: usize = 512 * 1024 * 1024;
/// Cada cuánto se comprueba si el hijo terminó.
const POLL_INTERVAL: Duration = Duration::from_millis(20);

/// Ejecuta un comando ya resuelto por la política.
///
/// # Errors
/// `NotFound` si el ejecutable no existe, `AccessDenied` si el sistema lo impide,
/// `ElevationRequired` si pide elevación (el agente nunca eleva) u `Other`.
pub fn run(spec: &CommandSpec, cancel: &CancelFlag) -> Result<CommandOutput, PlatformError> {
    if !spec.executable.is_file() {
        return Err(PlatformError::new(PlatformErrorKind::NotFound, "the configured executable does not exist"));
    }

    let mut command = Command::new(&spec.executable);
    command.args(&spec.arguments).stdin(Stdio::null());
    if spec.capture_output {
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
    } else {
        command.stdout(Stdio::null()).stderr(Stdio::null());
    }
    if let Some(directory) = &spec.working_directory {
        command.current_dir(directory);
    }
    // El hijo no necesita saber dónde vive el agente ni con qué está configurado.
    for (key, _) in std::env::vars() {
        if key.to_ascii_uppercase().starts_with("JARVIS") {
            command.env_remove(key);
        }
    }
    // Suspendido: nada se ejecuta hasta que el proceso esté dentro del job.
    command.creation_flags(CREATE_NO_WINDOW.0 | CREATE_SUSPENDED.0);

    let mut child = command.spawn().map_err(spawn_error)?;

    let job = match confine(&child) {
        Ok(job) => job,
        Err(error) => {
            // Sigue suspendido: no ha ejecutado ni una instrucción.
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };

    if let Err(error) = resume(child.id()) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }

    let limit = spec.max_output_bytes;
    let stdout = child.stdout.take().map(|mut pipe| std::thread::spawn(move || read_capped(&mut pipe, limit)));
    let stderr = child.stderr.take().map(|mut pipe| std::thread::spawn(move || read_capped(&mut pipe, limit)));

    let deadline = Instant::now() + spec.timeout;
    let mut timed_out = false;
    let mut cancelled = false;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {}
            Err(error) => return Err(PlatformError::new(PlatformErrorKind::Other, error.to_string())),
        }
        // Cancelación pedida: se mata igual que en el timeout, pero se distingue en el resultado.
        if cancel.load(Ordering::Relaxed) {
            cancelled = true;
            // SAFETY: el job lo creó esta función y sigue vivo.
            unsafe {
                let _ = TerminateJobObject(job.0, 1);
            }
            break;
        }
        if Instant::now() >= deadline {
            timed_out = true;
            // Mata al comando y a todo lo que haya lanzado.
            // SAFETY: el job lo creó esta función y sigue vivo.
            unsafe {
                let _ = TerminateJobObject(job.0, 1);
            }
            break;
        }
        std::thread::sleep(POLL_INTERVAL);
    }

    let status = child.wait().map_err(|error| PlatformError::new(PlatformErrorKind::Other, error.to_string()))?;

    let (out, out_truncated) = join(stdout);
    let (err, err_truncated) = join(stderr);

    Ok(CommandOutput {
        exit_code: status.code().unwrap_or(-1),
        stdout: sanitize(&out),
        stderr: sanitize(&err),
        truncated: out_truncated || err_truncated,
        timed_out,
        cancelled,
    })
}

/// Crea el job con sus límites y mete al hijo dentro.
fn confine(child: &Child) -> Result<OwnedHandle, PlatformError> {
    // SAFETY: el job se cierra al soltar el handle; la estructura se rellena entera con su tamaño.
    unsafe {
        let job = CreateJobObjectW(None, PCWSTR::null()).map_err(|_| last_error("CreateJobObjectW"))?;
        let job = OwnedHandle(job);

        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
            | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION
            | JOB_OBJECT_LIMIT_ACTIVE_PROCESS
            | JOB_OBJECT_LIMIT_PROCESS_MEMORY;
        info.BasicLimitInformation.ActiveProcessLimit = MAX_ACTIVE_PROCESSES;
        info.ProcessMemoryLimit = MEMORY_LIMIT_BYTES;
        SetInformationJobObject(
            job.0,
            JobObjectExtendedLimitInformation,
            (&raw const info).cast::<c_void>(),
            u32::try_from(size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>()).unwrap_or(0),
        )
        .map_err(|_| last_error("SetInformationJobObject"))?;

        AssignProcessToJobObject(job.0, HANDLE(child.as_raw_handle()))
            .map_err(|_| last_error("AssignProcessToJobObject"))?;
        Ok(job)
    }
}

/// Reanuda los hilos del proceso recién creado (nace con uno solo).
fn resume(pid: u32) -> Result<(), PlatformError> {
    // SAFETY: el snapshot se cierra con el `OwnedHandle`; `THREADENTRY32` se rellena con su tamaño.
    unsafe {
        let snapshot =
            CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0).map_err(|_| last_error("CreateToolhelp32Snapshot"))?;
        let snapshot = OwnedHandle(snapshot);
        let mut entry =
            THREADENTRY32 { dwSize: u32::try_from(size_of::<THREADENTRY32>()).unwrap_or(0), ..Default::default() };
        let mut resumed = 0usize;
        if Thread32First(snapshot.0, &raw mut entry).is_ok() {
            loop {
                if entry.th32OwnerProcessID == pid
                    && let Ok(thread) = OpenThread(THREAD_SUSPEND_RESUME, false, entry.th32ThreadID)
                {
                    let thread = OwnedHandle(thread);
                    if ResumeThread(thread.0) != u32::MAX {
                        resumed += 1;
                    }
                }
                if Thread32Next(snapshot.0, &raw mut entry).is_err() {
                    break;
                }
            }
        }
        if resumed == 0 {
            return Err(PlatformError::new(PlatformErrorKind::Other, "the command could not be resumed"));
        }
        Ok(())
    }
}

/// Lee el pipe hasta que se cierre, guardando como mucho `limit` bytes.
fn read_capped(pipe: &mut impl Read, limit: usize) -> (Vec<u8>, bool) {
    let mut collected = Vec::new();
    let mut truncated = false;
    let mut chunk = [0u8; 8192];
    loop {
        match pipe.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(read) => {
                if collected.len() < limit {
                    let room = limit - collected.len();
                    let take = room.min(read);
                    collected.extend_from_slice(&chunk[..take]);
                    truncated |= take < read;
                } else {
                    // Se sigue drenando: si no, el hijo se bloquearía al llenar el pipe.
                    truncated = true;
                }
            }
        }
    }
    (collected, truncated)
}

fn join(handle: Option<std::thread::JoinHandle<(Vec<u8>, bool)>>) -> (Vec<u8>, bool) {
    handle.and_then(|h| h.join().ok()).unwrap_or_default()
}

/// Texto apto para JSON: sin caracteres de control salvo salto de línea y tabulador.
fn sanitize(raw: &[u8]) -> String {
    String::from_utf8_lossy(raw)
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect::<String>()
        .trim_end()
        .to_owned()
}

fn spawn_error(error: std::io::Error) -> PlatformError {
    let kind = match error.raw_os_error() {
        Some(2 | 3) => PlatformErrorKind::NotFound,
        Some(5) => PlatformErrorKind::AccessDenied,
        Some(740) => PlatformErrorKind::ElevationRequired,
        _ => PlatformErrorKind::Other,
    };
    PlatformError::new(kind, format!("the command could not be started (error {})", error.raw_os_error().unwrap_or(0)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn system32(exe: &str) -> PathBuf {
        PathBuf::from(crate::sys::expand_env(r"%SystemRoot%\System32")).join(exe)
    }

    fn spec(exe: &str, args: &[&str], timeout_ms: u64, max_output: usize) -> CommandSpec {
        CommandSpec {
            executable: system32(exe),
            arguments: args.iter().map(|a| (*a).to_owned()).collect(),
            working_directory: None,
            capture_output: true,
            max_output_bytes: max_output,
            timeout: Duration::from_millis(timeout_ms),
        }
    }

    #[test]
    fn runs_a_command_and_captures_its_output() {
        let output = run(&spec("hostname.exe", &[], 10_000, 4096), &CancelFlag::new(false)).expect("run");
        assert_eq!(output.exit_code, 0);
        assert!(!output.timed_out && !output.truncated);
        assert!(!output.stdout.is_empty(), "hostname debe imprimir algo");
        assert!(!output.stdout.contains('\r'), "la salida va limpia de caracteres de control");
    }

    #[test]
    fn reports_the_exit_code_without_treating_it_as_an_error() {
        // `ping` a una dirección inalcanzable termina con código != 0: es un resultado, no un fallo.
        let output =
            run(&spec("ping.exe", &["-n", "1", "-w", "200", "192.0.2.1"], 10_000, 4096), &CancelFlag::new(false))
                .expect("run");
        assert_ne!(output.exit_code, 0);
        assert!(!output.timed_out);
    }

    #[test]
    fn a_running_command_stops_when_the_task_is_cancelled() {
        use std::sync::Arc;
        // `ping` de 30 s: si la cancelación no matara al hijo, el test tardaría media hora.
        let flag = Arc::new(CancelFlag::new(false));
        let raised = flag.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            raised.store(true, Ordering::Relaxed);
        });

        let started = Instant::now();
        let output = run(&spec("ping.exe", &["-n", "30", "127.0.0.1"], 60_000, 4096), &flag).expect("run");
        assert!(output.cancelled, "el resultado dice que se canceló");
        assert!(!output.timed_out, "no fue el timeout");
        assert!(started.elapsed() < Duration::from_secs(10), "terminó al cancelar, no al acabar");
    }

    #[test]
    fn kills_the_command_when_it_runs_out_of_time() {
        let started = Instant::now();
        let output =
            run(&spec("ping.exe", &["-n", "30", "127.0.0.1"], 400, 4096), &CancelFlag::new(false)).expect("run");
        assert!(output.timed_out, "debe marcarse como agotado");
        assert!(started.elapsed() < Duration::from_secs(5), "y cortarse pronto: {:?}", started.elapsed());
    }

    #[test]
    fn caps_the_captured_output() {
        let output = run(&spec("ipconfig.exe", &["/all"], 15_000, 64), &CancelFlag::new(false)).expect("run");
        assert!(output.truncated, "la salida larga se marca como truncada");
        assert!(output.stdout.len() <= 64, "y no se guarda más de lo permitido: {}", output.stdout.len());
    }

    #[test]
    fn missing_executables_are_reported_as_not_found() {
        let error =
            run(&spec("definitely-not-here.exe", &[], 1000, 16), &CancelFlag::new(false)).expect_err("no existe");
        assert_eq!(error.kind, PlatformErrorKind::NotFound);
    }
}

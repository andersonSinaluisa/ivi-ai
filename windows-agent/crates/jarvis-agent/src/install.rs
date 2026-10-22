//! Instalación por usuario (HU-WIN-050): binarios en `%LOCALAPPDATA%\Programs\Jarvis\Agent` y una
//! Scheduled Task al iniciar sesión. Sin administrador y sin elevación (ADR-010, ADR-011).

use std::path::{Path, PathBuf};

use crate::paths::Paths;

/// Carpeta de binarios (instalación por usuario, como Chrome o VS Code).
///
/// # Errors
/// Si no se puede resolver `%LOCALAPPDATA%`.
pub fn programs_dir() -> anyhow::Result<PathBuf> {
    #[cfg(windows)]
    {
        Ok(jarvis_windows::local_app_data()
            .map_err(|e| anyhow::anyhow!("cannot resolve %LOCALAPPDATA%: {e}"))?
            .join("Programs")
            .join("Jarvis")
            .join("Agent"))
    }
    #[cfg(not(windows))]
    {
        anyhow::bail!("installation is only available on Windows")
    }
}

#[derive(Debug, Clone)]
pub struct InstallOptions {
    /// Raíz de datos alternativa (se pasa como `--root` a la tarea).
    pub root_override: Option<PathBuf>,
    pub start: bool,
    pub allow_elevated: bool,
}

/// Copia el binario y registra la tarea de inicio de sesión. Es idempotente.
///
/// # Errors
/// Si el agente está elevado, si no se puede copiar el binario o si el Programador de tareas rechaza
/// el registro.
#[cfg(windows)]
pub fn install(paths: &Paths, options: &InstallOptions) -> anyhow::Result<()> {
    if jarvis_windows::is_elevated() && !options.allow_elevated {
        anyhow::bail!(
            "run the installation as a standard user: the agent must not run elevated (ADR-011). \
             Use --allow-elevated only for testing."
        );
    }

    let target_dir = programs_dir()?;
    std::fs::create_dir_all(&target_dir)?;
    paths.ensure()?;

    let current = std::env::current_exe()?;
    let installed = target_dir.join("jarvis-agent.exe");
    if same_file(&current, &installed) {
        println!("Binary already installed at {}", installed.display());
    } else {
        // Un binario en uso no se puede sobrescribir: se aparta antes de copiar.
        if installed.exists() {
            let previous = installed.with_extension("exe.old");
            let _ = std::fs::remove_file(&previous);
            std::fs::rename(&installed, &previous)
                .map_err(|e| anyhow::anyhow!("stop the agent before reinstalling ({e})"))?;
        }
        std::fs::copy(&current, &installed)?;
        println!("Binary installed at {}", installed.display());
    }

    let mut arguments = String::from("run");
    if let Some(root) = &options.root_override {
        arguments = format!("--root \"{}\" run", root.display());
    }
    let sid = jarvis_windows::user_sid_string().map_err(|e| anyhow::anyhow!("cannot read the user SID: {e}"))?;
    let task = jarvis_windows::task_scheduler::LogonTask {
        executable: &installed,
        arguments: &arguments,
        working_directory: &target_dir,
        user_id: &sid,
    };
    jarvis_windows::task_scheduler::register_logon_task(&task)
        .map_err(|e| anyhow::anyhow!("cannot register the scheduled task: {e}"))?;
    println!(
        "Scheduled task '{}' registered (at logon, 15 s delay, least privilege).",
        jarvis_windows::task_scheduler::TASK_NAME
    );
    println!("Data directory: {}", paths.root.display());

    if options.start {
        jarvis_windows::task_scheduler::run_task().map_err(|e| anyhow::anyhow!("cannot start the task: {e}"))?;
        println!("Agent started.");
    } else {
        println!("Start it now with `jarvis-agent install --start`, or it will start at your next logon.");
    }
    Ok(())
}

/// Reapunta la Scheduled Task a otro ejecutable (cambio de versión y rollback, HU-WIN-063).
///
/// # Errors
/// Si el ejecutable no existe o el Programador de tareas rechaza el registro.
#[cfg(windows)]
pub fn point_task_to(executable: &Path, root_override: Option<&Path>) -> anyhow::Result<()> {
    if !executable.is_file() {
        anyhow::bail!("{} does not exist", executable.display());
    }
    let working_directory = executable.parent().unwrap_or(executable).to_path_buf();
    let arguments = match root_override {
        Some(root) => format!("--root \"{}\" run", root.display()),
        None => String::from("run"),
    };
    let sid = jarvis_windows::user_sid_string().map_err(|e| anyhow::anyhow!("cannot read the user SID: {e}"))?;
    let task = jarvis_windows::task_scheduler::LogonTask {
        executable,
        arguments: &arguments,
        working_directory: &working_directory,
        user_id: &sid,
    };
    jarvis_windows::task_scheduler::register_logon_task(&task)
        .map_err(|e| anyhow::anyhow!("cannot update the scheduled task: {e}"))
}

#[derive(Debug, Clone, Copy)]
pub struct UninstallOptions {
    /// Borra también configuración, política, base de datos y logs.
    pub purge: bool,
    /// No preguntar (automatización).
    pub yes: bool,
}

/// Quita la tarea, la credencial y los binarios. Con `--purge`, también los datos.
///
/// # Errors
/// Si el Programador de tareas falla o no se pueden borrar los archivos.
#[cfg(windows)]
pub async fn uninstall(paths: &Paths, options: UninstallOptions) -> anyhow::Result<()> {
    // Parar el agente si está corriendo (si no responde, se continúa igualmente).
    if let Ok(sid) = jarvis_windows::user_sid_string()
        && crate::ipc::send(&crate::paths::pipe_name(&sid), &crate::ipc::Command::Stop).await.is_ok()
    {
        println!("Running agent stopped.");
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }

    match jarvis_windows::task_scheduler::unregister_task() {
        Ok(true) => println!("Scheduled task removed."),
        Ok(false) => println!("No scheduled task was registered."),
        Err(error) => println!("Could not remove the scheduled task: {error}"),
    }

    // Credencial del device: se borra siempre (se vuelve a provisionar si se reinstala).
    if let Ok(config) = crate::config::Config::load(&paths.config())
        && let Some(device_id) = config.device.device_id
    {
        use jarvis_windows::CredentialStore;
        match jarvis_windows::WindowsCredentialStore.delete(&device_id.hyphenated().to_string()) {
            Ok(()) => println!("Device credential removed from Credential Manager."),
            Err(_) => println!("No device credential was stored."),
        }
    }

    if let Ok(dir) = programs_dir()
        && dir.exists()
    {
        let _ = std::fs::remove_file(dir.join("jarvis-agent.exe.old"));
        match std::fs::remove_file(dir.join("jarvis-agent.exe")) {
            Ok(()) => println!("Binary removed from {}", dir.display()),
            Err(error) => println!("Could not remove the binary ({error}); it is probably still running."),
        }
    }

    if options.purge {
        if !options.yes && !confirm(&format!("Delete ALL local data in {}?", paths.root.display()))? {
            println!("Data kept.");
            return Ok(());
        }
        std::fs::remove_dir_all(&paths.root)?;
        println!("Local data deleted ({}).", paths.root.display());
    } else {
        println!("Local data kept in {} (use --purge to delete it).", paths.root.display());
    }
    Ok(())
}

fn confirm(question: &str) -> anyhow::Result<bool> {
    use std::io::Write;
    print!("{question} [y/N] ");
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    Ok(matches!(answer.trim().to_lowercase().as_str(), "y" | "yes" | "s" | "si" | "sí"))
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

#[cfg(not(windows))]
pub fn install(_paths: &Paths, _options: &InstallOptions) -> anyhow::Result<()> {
    anyhow::bail!("installation is only available on Windows")
}

#[cfg(not(windows))]
pub async fn uninstall(_paths: &Paths, _options: UninstallOptions) -> anyhow::Result<()> {
    anyhow::bail!("installation is only available on Windows")
}

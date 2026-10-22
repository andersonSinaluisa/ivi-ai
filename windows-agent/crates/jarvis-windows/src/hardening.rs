//! Endurecimiento del proceso (HU-WIN-048; WT-08 y WT-12 del threat model).
//!
//! Se aplica al arrancar, antes de cargar nada opcional:
//! - búsqueda de DLL restringida a `System32` (nada de cargar desde el directorio actual);
//! - prohibición de cargar imágenes remotas o de baja integridad;
//! - Job Object propio con límite de memoria, para que una tool desbocada no ahogue la PC;
//! - exclusión de Windows Error Reporting, para que un volcado no acabe con el secreto en disco.

use std::ffi::c_void;

use jarvis_tooling::platform::{PlatformError, PlatformErrorKind};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_PROCESS_MEMORY, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectExtendedLimitInformation, SetInformationJobObject,
};
use windows::Win32::System::LibraryLoader::{LOAD_LIBRARY_SEARCH_SYSTEM32, SetDefaultDllDirectories, SetDllDirectoryW};
use windows::Win32::System::SystemServices::PROCESS_MITIGATION_IMAGE_LOAD_POLICY;
use windows::Win32::System::Threading::{GetCurrentProcess, ProcessImageLoadPolicy, SetProcessMitigationPolicy};
use windows::core::PCWSTR;

use crate::sys::{OwnedHandle, last_error};

/// Qué mitigaciones se pudieron aplicar (algunas dependen de la versión de Windows).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Report {
    pub dll_search_restricted: bool,
    pub remote_images_blocked: bool,
    pub wer_excluded: bool,
    pub memory_limited: bool,
}

/// Job Object del propio proceso; mantenerlo vivo conserva los límites.
#[derive(Debug)]
pub struct ProcessLimits {
    _job: OwnedHandle,
}

/// Aplica las mitigaciones que no dependen de configuración.
#[must_use]
pub fn apply() -> Report {
    let mut report = Report::default();

    // SAFETY: llamadas sobre el proceso actual con parámetros constantes.
    unsafe {
        report.dll_search_restricted = SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32).is_ok();
        // Quita el directorio actual del orden de búsqueda heredado.
        let _ = SetDllDirectoryW(PCWSTR::from_raw(windows::core::w!("").as_ptr()));

        let mut policy = PROCESS_MITIGATION_IMAGE_LOAD_POLICY::default();
        // Sin imágenes desde recursos de red ni con etiqueta de baja integridad (descargas).
        // NoRemoteImages | NoLowMandatoryLabelImages (bits 0 y 1 del campo de bits).
        policy.Anonymous.Anonymous._bitfield = 0x0000_0003;
        report.remote_images_blocked = SetProcessMitigationPolicy(
            ProcessImageLoadPolicy,
            (&raw const policy).cast::<c_void>(),
            size_of::<PROCESS_MITIGATION_IMAGE_LOAD_POLICY>(),
        )
        .is_ok();
    }

    report.wer_excluded = exclude_from_error_reporting();
    report
}

/// Un volcado de memoria del agente podría contener el secreto del device: se excluye de WER.
fn exclude_from_error_reporting() -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    let Some(name) = exe.file_name() else { return false };
    let name = windows::core::HSTRING::from(name);
    // SAFETY: cadena válida terminada en NUL; `false` = solo para este usuario (no requiere admin).
    unsafe { windows::Win32::System::ErrorReporting::WerAddExcludedApplication(PCWSTR(name.as_ptr()), false).is_ok() }
}

/// Mete el proceso en un Job Object con límite de memoria.
///
/// # Errors
/// Si Windows no permite crear el job o asignar el proceso (por ejemplo, si ya está en un job que
/// no admite anidamiento).
pub fn limit_memory(megabytes: usize) -> Result<ProcessLimits, PlatformError> {
    // SAFETY: el job se cierra al soltar `ProcessLimits`; la estructura se rellena por completo.
    unsafe {
        let job = CreateJobObjectW(None, PCWSTR::null()).map_err(|_| last_error("CreateJobObjectW"))?;
        let job = OwnedHandle(job);

        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_PROCESS_MEMORY;
        info.ProcessMemoryLimit = megabytes * 1024 * 1024;
        SetInformationJobObject(
            job.0,
            JobObjectExtendedLimitInformation,
            (&raw const info).cast::<c_void>(),
            u32::try_from(size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>()).unwrap_or(0),
        )
        .map_err(|_| last_error("SetInformationJobObject"))?;

        AssignProcessToJobObject(job.0, GetCurrentProcess()).map_err(|_| {
            PlatformError::new(PlatformErrorKind::Other, "the process could not be added to its own job object")
        })?;
        Ok(ProcessLimits { _job: job })
    }
}

/// Puertos TCP en escucha de este proceso. El agente no debe abrir ninguno (solo sale hacia el Cloud).
///
/// # Errors
/// Si la tabla de conexiones no se puede consultar.
pub fn listening_ports() -> Result<Vec<u16>, PlatformError> {
    use windows::Win32::NetworkManagement::IpHelper::{
        GetExtendedTcpTable, MIB_TCPTABLE_OWNER_PID, TCP_TABLE_OWNER_PID_LISTENER,
    };
    const AF_INET: u32 = 2;
    const MIB_TCP_STATE_LISTEN: u32 = 2;

    let pid = std::process::id();
    let mut size = 0u32;
    // SAFETY: primera llamada para conocer el tamaño; segunda con el buffer del tamaño indicado.
    unsafe {
        GetExtendedTcpTable(None, &raw mut size, false, AF_INET, TCP_TABLE_OWNER_PID_LISTENER, 0);
        let mut buffer = vec![0u8; size as usize];
        let status = GetExtendedTcpTable(
            Some(buffer.as_mut_ptr().cast()),
            &raw mut size,
            false,
            AF_INET,
            TCP_TABLE_OWNER_PID_LISTENER,
            0,
        );
        if status != 0 {
            return Err(PlatformError::new(PlatformErrorKind::Other, format!("GetExtendedTcpTable: {status}")));
        }
        let table = buffer.as_ptr().cast::<MIB_TCPTABLE_OWNER_PID>();
        let count = (*table).dwNumEntries as usize;
        let rows = std::ptr::addr_of!((*table).table)
            .cast::<windows::Win32::NetworkManagement::IpHelper::MIB_TCPROW_OWNER_PID>();
        let mut ports = Vec::new();
        for index in 0..count {
            let row = &*rows.add(index);
            if row.dwOwningPid == pid && row.dwState == MIB_TCP_STATE_LISTEN {
                // El puerto viene en orden de red.
                ports.push(u16::from_be_bytes([(row.dwLocalPort & 0xff) as u8, ((row.dwLocalPort >> 8) & 0xff) as u8]));
            }
        }
        Ok(ports)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_the_mitigations_it_can() {
        let report = apply();
        assert!(report.dll_search_restricted, "la búsqueda de DLL debe quedar restringida a System32");
        // Las otras dependen de la política del equipo: se informa, no se exige.
        println!("{report:?}");
    }

    #[test]
    fn limits_memory_with_a_job_object() {
        match limit_memory(512) {
            Ok(limits) => drop(limits),
            // En CI el proceso puede estar ya dentro de un job que no admite anidamiento.
            Err(error) => println!("SKIPPED: {error}"),
        }
    }

    #[test]
    fn the_agent_opens_no_listening_ports() {
        let ports = listening_ports().expect("tcp table");
        assert!(ports.is_empty(), "el agente no debe escuchar en ningún puerto: {ports:?}");
    }
}

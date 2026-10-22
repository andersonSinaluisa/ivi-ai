//! Sandbox de rutas (HU-WIN-054; WT-06 y WT-10 del threat model).
//!
//! La ruta se abre **sin seguir reparse points** y todo lo demás se hace sobre ese handle: la ruta
//! final se obtiene del propio handle y las lecturas no vuelven a resolver la ruta, así que cambiar
//! un directorio por un junction después de validar no sirve de nada (TOCTOU).

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use jarvis_tooling::platform::{DirEntryInfo, OpenEntry, Opened, PlatformError, PlatformErrorKind};
use windows::Win32::Foundation::{FILETIME, HANDLE, MAX_PATH};
use windows::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_HIDDEN,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_GENERIC_READ,
    FILE_ID_BOTH_DIR_INFO, FILE_NAME_NORMALIZED, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    FileIdBothDirectoryInfo, FileIdBothDirectoryRestartInfo, GetDriveTypeW, GetFileInformationByHandle,
    GetFileInformationByHandleEx, GetFinalPathNameByHandleW, OPEN_EXISTING, ReadFile,
};
use windows::Win32::System::IO::OVERLAPPED;
use windows::core::{HSTRING, PCWSTR};

use crate::sys::{OwnedHandle, last_error, wide_to_string};

/// Valor que devuelve `GetDriveTypeW` para un disco local fijo.
const DRIVE_FIXED: u32 = 3;

/// Archivo o directorio abierto que solo permite leer.
#[derive(Debug)]
pub struct Win32Entry {
    handle: OwnedHandle,
}

impl OpenEntry for Win32Entry {
    fn read(&self, offset: u64, max_bytes: usize) -> Result<Vec<u8>, PlatformError> {
        let mut buffer = vec![0u8; max_bytes];
        let mut read = 0u32;
        let mut overlapped = OVERLAPPED::default();
        overlapped.Anonymous.Anonymous.Offset = (offset & 0xFFFF_FFFF) as u32;
        overlapped.Anonymous.Anonymous.OffsetHigh = (offset >> 32) as u32;
        // SAFETY: buffer propio del tamaño indicado; el handle es de solo lectura y sigue vivo.
        let result =
            unsafe { ReadFile(self.handle.0, Some(&mut buffer), Some(&raw mut read), Some(&raw mut overlapped)) };
        match result {
            Ok(()) => {
                buffer.truncate(read as usize);
                Ok(buffer)
            }
            // Leer más allá del final devuelve ERROR_HANDLE_EOF: no es un error para nosotros.
            Err(error) if error.code().0 == -2_147_024_858 => Ok(Vec::new()),
            Err(_) => Err(last_error("ReadFile")),
        }
    }

    fn list(&self, limit: usize) -> Result<Vec<DirEntryInfo>, PlatformError> {
        let mut entries = Vec::new();
        let mut buffer = vec![0u8; 64 * 1024];
        let mut first = true;
        loop {
            let class = if first { FileIdBothDirectoryRestartInfo } else { FileIdBothDirectoryInfo };
            first = false;
            // SAFETY: buffer propio con su tamaño; la API rellena una lista enlazada por offsets.
            let ok = unsafe {
                GetFileInformationByHandleEx(
                    self.handle.0,
                    class,
                    buffer.as_mut_ptr().cast::<c_void>(),
                    u32::try_from(buffer.len()).unwrap_or(0),
                )
            };
            if ok.is_err() {
                break; // ERROR_NO_MORE_FILES termina el recorrido.
            }
            let mut offset = 0usize;
            loop {
                // SAFETY: `offset` siempre apunta a una entrada válida dentro del buffer.
                let info = unsafe { &*buffer.as_ptr().add(offset).cast::<FILE_ID_BOTH_DIR_INFO>() };
                let name_length = info.FileNameLength as usize / 2;
                // SAFETY: la API garantiza `FileNameLength` bytes de nombre tras la cabecera.
                let name_slice = unsafe { std::slice::from_raw_parts(info.FileName.as_ptr(), name_length) };
                let name = String::from_utf16_lossy(name_slice);
                if name != "." && name != ".." {
                    let attributes = info.FileAttributes;
                    entries.push(DirEntryInfo {
                        name,
                        is_dir: attributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0,
                        size: u64::try_from(info.EndOfFile).unwrap_or(0),
                        modified: filetime(info.LastWriteTime),
                        hidden: attributes & FILE_ATTRIBUTE_HIDDEN.0 != 0,
                    });
                    if entries.len() >= limit.saturating_add(1) {
                        return Ok(entries);
                    }
                }
                if info.NextEntryOffset == 0 {
                    break;
                }
                offset += info.NextEntryOffset as usize;
            }
        }
        Ok(entries)
    }
}

fn filetime(ticks: i64) -> Option<SystemTime> {
    let ticks = u64::try_from(ticks).ok()?;
    let unix = ticks.checked_sub(116_444_736_000_000_000)?;
    Some(UNIX_EPOCH + Duration::from_nanos(unix.saturating_mul(100)))
}

fn filetime_struct(value: FILETIME) -> Option<SystemTime> {
    let ticks = (u64::from(value.dwHighDateTime) << 32) | u64::from(value.dwLowDateTime);
    let unix = ticks.checked_sub(116_444_736_000_000_000)?;
    Some(UNIX_EPOCH + Duration::from_nanos(unix.saturating_mul(100)))
}

/// Abre una ruta aplicando las comprobaciones del sandbox.
///
/// # Errors
/// `NotFound` si no existe; `AccessDenied` si es un reparse point, está en una unidad no fija o el
/// sistema deniega el acceso.
pub fn open_sandboxed(path: &Path) -> Result<Opened, PlatformError> {
    let wide = HSTRING::from(path.as_os_str());
    // SAFETY: cadena válida; el handle se cierra al soltar `OwnedHandle`.
    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            FILE_GENERIC_READ.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            // BACKUP_SEMANTICS permite abrir directorios; OPEN_REPARSE_POINT **no** sigue enlaces.
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            None,
        )
    }
    .map_err(|_| last_error("CreateFileW"))?;
    let handle = OwnedHandle(handle);

    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: handle válido recién abierto; la estructura es local.
    unsafe { GetFileInformationByHandle(handle.0, &raw mut info) }
        .map_err(|_| last_error("GetFileInformationByHandle"))?;

    if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
        return Err(PlatformError::new(
            PlatformErrorKind::AccessDenied,
            "symlinks, junctions and mount points are not allowed inside the sandbox",
        ));
    }

    let final_path = final_path_of(handle.0)?;
    if !is_fixed_drive(&final_path) {
        return Err(PlatformError::new(
            PlatformErrorKind::AccessDenied,
            "only fixed drives are allowed (no network or removable volumes)",
        ));
    }

    let is_dir = info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0;
    let size = (u64::from(info.nFileSizeHigh) << 32) | u64::from(info.nFileSizeLow);
    Ok(Opened {
        final_path,
        is_dir,
        size,
        modified: filetime_struct(info.ftLastWriteTime),
        hidden: info.dwFileAttributes & FILE_ATTRIBUTE_HIDDEN.0 != 0,
        entry: Box::new(Win32Entry { handle }),
    })
}

/// Ruta real del handle (resuelve 8.3, mayúsculas y enlaces intermedios).
fn final_path_of(handle: HANDLE) -> Result<PathBuf, PlatformError> {
    let mut buffer = vec![0u16; MAX_PATH as usize * 4];
    // SAFETY: buffer propio con su longitud en caracteres.
    let length = unsafe { GetFinalPathNameByHandleW(handle, &mut buffer, FILE_NAME_NORMALIZED) };
    if length == 0 {
        return Err(last_error("GetFinalPathNameByHandleW"));
    }
    let text = wide_to_string(&buffer);
    // Windows devuelve la forma `\\?\C:\...`: se recorta para comparar con las raíces configuradas.
    Ok(PathBuf::from(text.strip_prefix(r"\\?\").unwrap_or(&text)))
}

fn is_fixed_drive(path: &Path) -> bool {
    let Some(root) = path.components().next() else { return false };
    let root = format!("{}\\", root.as_os_str().to_string_lossy());
    let wide = HSTRING::from(root);
    // SAFETY: cadena válida terminada en NUL.
    unsafe { GetDriveTypeW(PCWSTR(wide.as_ptr())) == DRIVE_FIXED }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_a_file_and_reads_it_through_the_handle() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("notes.txt");
        std::fs::write(&file, b"hola jarvis").expect("write");

        let opened = open_sandboxed(&file).expect("open");
        assert!(!opened.is_dir);
        assert_eq!(opened.size, 11);
        assert_eq!(opened.entry.read(0, 64).expect("read"), b"hola jarvis");
        assert_eq!(opened.entry.read(5, 64).expect("read"), b"jarvis", "lee desde el offset pedido");
        assert!(opened.entry.read(99, 64).expect("read").is_empty(), "más allá del final no es un error");
        // La ruta final es la real: sirve para comprobar las raíces permitidas.
        assert!(opened.final_path.to_string_lossy().to_lowercase().ends_with("notes.txt"));
    }

    #[test]
    fn lists_a_directory_without_dot_entries() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("a.txt"), b"a").expect("write");
        std::fs::create_dir(dir.path().join("sub")).expect("mkdir");

        let opened = open_sandboxed(dir.path()).expect("open");
        assert!(opened.is_dir);
        let entries = opened.entry.list(100).expect("list");
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"a.txt") && names.contains(&"sub"), "{names:?}");
        assert!(!names.contains(&".") && !names.contains(&".."));
        assert!(entries.iter().any(|e| e.is_dir && e.name == "sub"));
    }

    #[test]
    fn rejects_junctions_instead_of_following_them() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("secret");
        std::fs::create_dir(&target).expect("mkdir");
        let link = dir.path().join("link");

        // `mklink /J` no necesita privilegios; si el sistema lo impide, el test se salta.
        let created = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J", &link.to_string_lossy(), &target.to_string_lossy()])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !created {
            eprintln!("SKIPPED: no se pudo crear el junction");
            return;
        }

        let error = open_sandboxed(&link).expect_err("un junction debe rechazarse");
        assert_eq!(error.kind, PlatformErrorKind::AccessDenied, "{error}");
    }

    #[test]
    fn reports_missing_paths() {
        let error = open_sandboxed(Path::new(r"C:\does-not-exist-jarvis\x.txt")).expect_err("no existe");
        assert_eq!(error.kind, PlatformErrorKind::NotFound);
    }
}

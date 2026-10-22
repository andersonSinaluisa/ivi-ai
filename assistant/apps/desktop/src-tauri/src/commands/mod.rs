//! Comandos Tauri: la única superficie que la UI puede invocar.

pub mod chat;
pub mod cloud;
pub mod memory;
pub mod settings;
pub mod system;
pub mod tools;
pub mod voice;

pub type CmdResult<T> = Result<T, String>;

pub(crate) fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

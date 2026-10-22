//! Integración con Windows (docs/05-windows). Único crate con `unsafe`: expone interfaces seguras
//! (`WindowsApi`, `CredentialStore`) y encapsula Win32/COM.
//!
//! Nivel de integración: APIs nativas y Win32 (ADR-002). Sin input sintético ni UI Automation.

#![cfg(windows)]

pub mod authenticode;
pub mod commands;
pub mod credentials;
pub mod events;
pub mod fs;
pub mod hardening;
pub mod http;
pub mod launch;
pub mod notify;
pub mod pipe;
pub mod process;
pub mod session;
pub mod sys;
pub mod task_scheduler;
pub mod uia;
pub mod window;

mod api;

pub use api::Win32Api;
pub use credentials::{CredentialStore, Secret, WindowsCredentialStore};
pub use sys::{SingleInstance, is_elevated, local_app_data, user_sid_string};

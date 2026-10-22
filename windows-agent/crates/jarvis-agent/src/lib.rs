//! Jarvis Windows Agent: composición del pipeline, tools, política y enlace con el Cloud.
//!
//! El binario `jarvis-agent` es una capa fina de CLI sobre esta librería; tenerla separada permite
//! probar el pipeline completo con dobles de plataforma y de transporte.

pub mod cloud;
pub mod config;
pub mod diagnostics;
pub mod health;
pub mod install;
pub mod ipc;
pub mod logging;
pub mod paths;
pub mod pipeline;
pub mod policy_watch;
pub mod provision;
pub mod runtime;
pub mod safe_mode;
pub mod tool_api;
pub mod tools;
pub mod update;

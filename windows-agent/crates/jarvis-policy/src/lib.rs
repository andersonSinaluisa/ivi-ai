//! Autoridad local de permisos (ADR-004, docs/04-security/permission-engine.md).
//!
//! `policy.toml` vive en la PC, lo edita solo el usuario y ningún mensaje del Cloud puede
//! modificarlo. La evaluación es una función pura y determinista.

pub mod apps;
pub mod decision;
pub mod files;
pub mod model;
pub mod principal;

pub use apps::{AppCatalog, AppEntry, LaunchSpec, ResolveError};
pub use decision::{Authenticity, Decision, DecisionKind, DenyReason, PolicyState, evaluate};
pub use files::{
    AllowedRoot, CommandEntry, CommandParam, FilesystemPolicy, PathRejection, check_path_shape, is_within,
};
pub use model::{KeyEntry, Policy, PolicyError, SignaturePolicy};
pub use principal::Principal;

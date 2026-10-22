//! Principals: quién solicita una invocación.

use std::fmt;

use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Principal {
    /// `requestedBy { type: "User" }` del Cloud (identidad afirmada, C10).
    CloudUser(Uuid),
    /// `requestedBy { type: "Device" }` del Cloud (identidad afirmada, C10).
    CloudDevice(Uuid),
    /// Owner en la propia PC (CLI por named pipe con el SID del usuario).
    LocalOwner,
    /// Futuro Agent Core (Local Tool API).
    LocalAgentCore,
}

impl Principal {
    /// Construye el principal a partir de `requestedBy` del Cloud. `None` si el tipo es desconocido.
    #[must_use]
    pub fn from_cloud(kind: &str, id: Uuid) -> Option<Self> {
        match kind {
            "User" => Some(Self::CloudUser(id)),
            "Device" => Some(Self::CloudDevice(id)),
            _ => None,
        }
    }

    #[must_use]
    pub const fn is_remote(&self) -> bool {
        matches!(self, Self::CloudUser(_) | Self::CloudDevice(_))
    }

    /// Notación de la política: `cloud:user:<id>`, `cloud:device:<id>`, `local:owner`, `local:agent-core`.
    #[must_use]
    pub fn policy_id(&self) -> String {
        match self {
            Self::CloudUser(id) => format!("cloud:user:{}", id.hyphenated()),
            Self::CloudDevice(id) => format!("cloud:device:{}", id.hyphenated()),
            Self::LocalOwner => "local:owner".to_owned(),
            Self::LocalAgentCore => "local:agent-core".to_owned(),
        }
    }

    /// Parsea la notación de la política (sin comodines).
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "local:owner" => Some(Self::LocalOwner),
            "local:agent-core" => Some(Self::LocalAgentCore),
            _ => {
                if let Some(id) = text.strip_prefix("cloud:user:") {
                    Uuid::try_parse(id).ok().map(Self::CloudUser)
                } else if let Some(id) = text.strip_prefix("cloud:device:") {
                    Uuid::try_parse(id).ok().map(Self::CloudDevice)
                } else {
                    None
                }
            }
        }
    }
}

impl fmt::Display for Principal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.policy_id())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_policy_notation() {
        let id = Uuid::now_v7();
        for p in
            [Principal::CloudUser(id), Principal::CloudDevice(id), Principal::LocalOwner, Principal::LocalAgentCore]
        {
            assert_eq!(Principal::parse(&p.policy_id()), Some(p));
        }
        assert_eq!(Principal::parse("cloud:*"), None);
        assert_eq!(Principal::from_cloud("Service", id), None);
    }
}

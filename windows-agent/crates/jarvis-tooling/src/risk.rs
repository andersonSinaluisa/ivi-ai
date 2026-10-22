//! Niveles de riesgo R0…R4 (docs/04-security/risk-model.md).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum RiskLevel {
    /// Observe.
    R0,
    /// Sensitive read.
    R1,
    /// Reversible action.
    R2,
    /// Impactful action.
    R3,
    /// Critical.
    R4,
}

impl RiskLevel {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::R0 => "R0",
            Self::R1 => "R1",
            Self::R2 => "R2",
            Self::R3 => "R3",
            Self::R4 => "R4",
        }
    }
}

impl fmt::Display for RiskLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for RiskLevel {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "R0" => Ok(Self::R0),
            "R1" => Ok(Self::R1),
            "R2" => Ok(Self::R2),
            "R3" => Ok(Self::R3),
            "R4" => Ok(Self::R4),
            other => Err(format!("unknown risk level '{other}' (expected R0..R4)")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::RiskLevel;

    #[test]
    fn levels_are_ordered() {
        assert!(RiskLevel::R0 < RiskLevel::R1);
        assert!(RiskLevel::R3 < RiskLevel::R4);
        assert_eq!(RiskLevel::R2.max(RiskLevel::R3), RiskLevel::R3);
        assert_eq!("R2".parse::<RiskLevel>(), Ok(RiskLevel::R2));
        assert!("R9".parse::<RiskLevel>().is_err());
    }
}

//! Detección y redacción de secretos. Jarvis no guarda contraseñas, tokens ni claves.

use std::sync::OnceLock;

use regex::Regex;

fn patterns() -> &'static [Regex] {
    static P: OnceLock<Vec<Regex>> = OnceLock::new();
    P.get_or_init(|| {
        [
            // "password: x", "contraseña = x", "token x", "api key: x", "pin: 1234"
            r"(?i)\b(password|passwd|pwd|contraseña|clave|token|secret|secreto|api[_ -]?key|pin)\b\s*(?:es|is|[:=])?\s*\S+",
            // Claves con prefijos conocidos
            r"\bsk-[A-Za-z0-9_\-]{16,}",
            r"\bgh[pousr]_[A-Za-z0-9]{20,}",
            r"\bAKIA[0-9A-Z]{16}\b",
            r"\bxox[abprs]-[A-Za-z0-9\-]{10,}",
            // JWT
            r"\beyJ[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,}",
            // Bloques de clave privada
            r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----",
            // Números de tarjeta (13–19 dígitos con separadores opcionales)
            r"\b(?:\d[ -]?){13,19}\b",
        ]
        .iter()
        .filter_map(|p| Regex::new(p).ok())
        .collect()
    })
}

#[must_use]
pub fn contains_secret(text: &str) -> bool {
    patterns().iter().any(|p| p.is_match(text))
}

#[must_use]
pub fn redact_secrets(text: &str) -> String {
    let mut out = text.to_owned();
    for p in patterns() {
        out = p.replace_all(&out, "[REDACTADO]").into_owned();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_common_secrets() {
        for s in [
            "mi password: hunter2",
            "la contraseña es Perro123",
            "usa este token ghp_abcdefghijklmnopqrstuvwxyz123456",
            "OPENAI sk-proj-abcdefghijklmnop1234",
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U",
            "tarjeta 4111 1111 1111 1111",
        ] {
            assert!(contains_secret(s), "no detectó: {s}");
            assert!(redact_secrets(s).contains("[REDACTADO]"), "no redactó: {s}");
        }
    }

    #[test]
    fn leaves_normal_text_alone() {
        for s in ["abre el bloc de notas", "¿qué hora es en Guayaquil?", "mi proyecto se llama LOOP"] {
            assert!(!contains_secret(s), "falso positivo: {s}");
            assert_eq!(redact_secrets(s), s);
        }
    }
}

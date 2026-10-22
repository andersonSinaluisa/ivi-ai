//! Privacidad de los títulos de ventana (docs/05-windows/active-window.md).

use jarvis_policy::model::{Privacy, TitleMode};

pub const MAX_TITLE_CHARS: usize = 256;

/// Aplica la política de títulos. Devuelve `(título, redactado)`.
#[must_use]
pub fn apply(privacy: &Privacy, process_name: Option<&str>, title: Option<&str>) -> (Option<String>, bool) {
    let Some(title) = title else { return (None, false) };
    if process_name.is_some_and(|name| privacy.never_title.iter().any(|p| p.eq_ignore_ascii_case(name))) {
        return (None, true);
    }
    if is_private_browsing(title) {
        return (None, true);
    }
    match privacy.title_mode {
        TitleMode::AppOnly => (None, true),
        TitleMode::Full => (Some(truncate(title)), false),
        TitleMode::RedactPatterns => {
            let redacted = redact(title);
            let changed = redacted != title;
            (Some(truncate(&redacted)), changed)
        }
    }
}

fn is_private_browsing(title: &str) -> bool {
    let lower = title.to_lowercase();
    ["incognito", "inprivate", "private browsing", "navegación privada"].iter().any(|m| lower.contains(m))
}

fn truncate(title: &str) -> String {
    if title.chars().count() <= MAX_TITLE_CHARS {
        title.to_owned()
    } else {
        title.chars().take(MAX_TITLE_CHARS - 1).collect::<String>() + "…"
    }
}

/// Sustituye correos, números largos (tarjetas, cuentas, teléfonos) y URLs con query.
#[must_use]
pub fn redact(title: &str) -> String {
    let mut out = String::with_capacity(title.len());
    for token in title.split_inclusive(char::is_whitespace) {
        let trimmed = token.trim_end();
        let suffix = &token[trimmed.len()..];
        if is_email(trimmed) || is_long_number(trimmed) || is_url_with_query(trimmed) {
            out.push_str("[REDACTED]");
        } else {
            out.push_str(trimmed);
        }
        out.push_str(suffix);
    }
    out
}

fn is_email(token: &str) -> bool {
    let mut parts = token.split('@');
    let (Some(user), Some(domain), None) = (parts.next(), parts.next(), parts.next()) else { return false };
    !user.is_empty() && domain.contains('.') && domain.len() > 3
}

fn is_long_number(token: &str) -> bool {
    let digits = token.chars().filter(char::is_ascii_digit).count();
    digits >= 8 && token.chars().all(|c| c.is_ascii_digit() || matches!(c, '-' | ' ' | '+' | '(' | ')'))
}

fn is_url_with_query(token: &str) -> bool {
    (token.starts_with("http://") || token.starts_with("https://")) && token.contains('?')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn privacy(mode: TitleMode) -> Privacy {
        Privacy { title_mode: mode, never_title: vec!["KeePassXC.exe".into()] }
    }

    #[test]
    fn password_managers_never_expose_their_title() {
        let (title, redacted) = apply(&privacy(TitleMode::Full), Some("keepassxc.exe"), Some("Personal.kdbx"));
        assert_eq!(title, None);
        assert!(redacted);
    }

    #[test]
    fn private_browsing_titles_are_hidden() {
        let (title, redacted) = apply(&privacy(TitleMode::Full), Some("chrome.exe"), Some("Bank - Incognito"));
        assert_eq!(title, None);
        assert!(redacted);
    }

    #[test]
    fn sensitive_patterns_are_replaced_by_default() {
        let (title, redacted) = apply(
            &privacy(TitleMode::RedactPatterns),
            Some("outlook.exe"),
            Some("Inbox (3) - persona@dominio.com - Outlook"),
        );
        assert_eq!(title.as_deref(), Some("Inbox (3) - [REDACTED] - Outlook"));
        assert!(redacted);

        let (title, _) = apply(&privacy(TitleMode::RedactPatterns), Some("chrome.exe"), Some("Pago 4111111111111111"));
        assert_eq!(title.as_deref(), Some("Pago [REDACTED]"));

        let (title, redacted) =
            apply(&privacy(TitleMode::RedactPatterns), Some("code.exe"), Some("Jarvis - Visual Studio Code"));
        assert_eq!(title.as_deref(), Some("Jarvis - Visual Studio Code"));
        assert!(!redacted);
    }

    #[test]
    fn app_only_mode_drops_every_title() {
        let (title, redacted) = apply(&privacy(TitleMode::AppOnly), Some("code.exe"), Some("anything"));
        assert_eq!(title, None);
        assert!(redacted);
    }

    #[test]
    fn titles_are_truncated() {
        let long = "x".repeat(400);
        let (title, _) = apply(&privacy(TitleMode::Full), Some("code.exe"), Some(&long));
        assert_eq!(title.expect("title").chars().count(), MAX_TITLE_CHARS);
    }
}

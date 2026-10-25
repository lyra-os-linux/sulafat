//! GNU gettext setup and session-locale selection.

use std::collections::HashMap;
use std::sync::OnceLock;

use gettextrs::{bind_textdomain_codeset, bindtextdomain, setlocale, textdomain, LocaleCategory};

const DOMAIN: &str = "sulafat";
const DEFAULT_LOCALE_DIR: &str = "/usr/share/locale";

static ACTIVE: OnceLock<&'static str> = OnceLock::new();

pub fn tr(message: &str) -> String {
    gettextrs::gettext(message)
}

pub fn format(message: &str, values: &[(&str, &str)]) -> String {
    let mut result = tr(message);
    for (name, value) in values {
        result = result.replace(&format!("{{{name}}}"), value);
    }
    result
}

/// Locale escolhido em `init` (en-US, pt-BR ou es-ES).
pub fn active_locale() -> &'static str {
    ACTIVE.get().copied().unwrap_or("en-US")
}

fn locale_dir() -> &'static str {
    option_env!("SULAFAT_LOCALEDIR").unwrap_or(DEFAULT_LOCALE_DIR)
}

/// Return the supported BCP-47 locale for a POSIX locale, safely ignoring encoding and modifier.
fn normalize_locale(raw: &str) -> &'static str {
    let base = raw
        .split(['.', '@'])
        .next()
        .unwrap_or_default()
        .replace('-', "_")
        .to_ascii_lowercase();
    match base.as_str() {
        "pt" | "pt_br" => "pt-BR",
        "es" | "es_es" => "es-ES",
        "en" | "en_us" | "c" | "posix" => "en-US",
        _ => "en-US",
    }
}

fn session_locale(get: impl Fn(&str) -> Option<String>) -> &'static str {
    let raw = get("LC_ALL")
        .filter(|value| !value.is_empty())
        .or_else(|| get("LC_MESSAGES").filter(|value| !value.is_empty()))
        .or_else(|| get("LANG").filter(|value| !value.is_empty()))
        .unwrap_or_else(|| "en_US.UTF-8".to_string());
    normalize_locale(&raw)
}

pub fn init() {
    let _ = setlocale(LocaleCategory::LcAll, "");
    let locale = session_locale(|name| std::env::var(name).ok());
    let _ = ACTIVE.set(locale);
    // Select only the messages category. Environment variables remain untouched for child
    // processes (notably OpenSSH), and the normalized locale cannot escape the supported set.
    let posix_locale = match locale {
        "pt-BR" => "pt_BR.UTF-8",
        "es-ES" => "es_ES.UTF-8",
        _ => "en_US.UTF-8",
    };
    let _ = setlocale(LocaleCategory::LcMessages, posix_locale);
    bindtextdomain(DOMAIN, locale_dir()).expect("failed to bind Sulafat translation directory");
    bind_textdomain_codeset(DOMAIN, "UTF-8").expect("failed to select UTF-8 translations");
    textdomain(DOMAIN).expect("failed to select Sulafat translation domain");
}

/// O catálogo inteiro do locale ativo, para a interface web traduzir sem uma
/// chamada por texto. Lê o `.mo` do checkout (build de desenvolvimento) ou o
/// instalado, o mesmo que o gettext usa no processo.
#[tauri::command]
pub fn i18n_catalog() -> HashMap<String, String> {
    let posix = active_locale().replace('-', "_");
    let relative = format!("{posix}/LC_MESSAGES/{DOMAIN}.mo");
    [
        std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../po/locale")).join(&relative),
        std::path::Path::new(locale_dir()).join(&relative),
    ]
    .iter()
    .find_map(|path| std::fs::read(path).ok())
    .map(|bytes| parse_mo(&bytes))
    .unwrap_or_default()
}

/// Lê um catálogo GNU `.mo` (só a forma singular; a interface não usa plurais).
fn parse_mo(bytes: &[u8]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let big = match bytes.get(0..4) {
        Some([0xde, 0x12, 0x04, 0x95]) => false,
        Some([0x95, 0x04, 0x12, 0xde]) => true,
        _ => return map,
    };
    let word = |offset: usize| -> Option<u32> {
        let raw: [u8; 4] = bytes.get(offset..offset + 4)?.try_into().ok()?;
        Some(if big {
            u32::from_be_bytes(raw)
        } else {
            u32::from_le_bytes(raw)
        })
    };
    let (Some(count), Some(originals), Some(translations)) = (word(8), word(12), word(16)) else {
        return map;
    };
    let string = |table: u32, index: u32| -> Option<String> {
        let entry = table as usize + index as usize * 8;
        let length = word(entry)? as usize;
        let offset = word(entry + 4)? as usize;
        let raw = bytes.get(offset..offset + length)?;
        String::from_utf8(raw.split(|&b| b == 0).next()?.to_vec()).ok()
    };
    for index in 0..count {
        if let (Some(key), Some(value)) = (string(originals, index), string(translations, index)) {
            if !key.is_empty() && !value.is_empty() {
                map.insert(key, value);
            }
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn normalizes_supported_posix_locales_and_falls_back() {
        assert_eq!(normalize_locale("en_US.UTF-8"), "en-US");
        assert_eq!(normalize_locale("pt_BR.UTF-8@custom"), "pt-BR");
        assert_eq!(normalize_locale("es_ES.utf8"), "es-ES");
        assert_eq!(normalize_locale("fr_FR.UTF-8"), "en-US");
        assert_eq!(normalize_locale("../../pt_BR"), "en-US");
    }

    #[test]
    fn locale_environment_uses_posix_precedence() {
        let vars = HashMap::from([
            ("LANG", "en_US.UTF-8".to_string()),
            ("LC_MESSAGES", "es_ES.UTF-8".to_string()),
            ("LC_ALL", "pt_BR.UTF-8".to_string()),
        ]);
        assert_eq!(session_locale(|name| vars.get(name).cloned()), "pt-BR");
    }

    #[test]
    fn each_required_lang_and_unsupported_fallback_are_selected() {
        for (lang, expected) in [
            ("en_US.UTF-8", "en-US"),
            ("pt_BR.UTF-8", "pt-BR"),
            ("es_ES.UTF-8", "es-ES"),
            ("fr_FR.UTF-8", "en-US"),
        ] {
            assert_eq!(
                session_locale(|name| (name == "LANG").then(|| lang.to_string())),
                expected
            );
        }
    }

    #[test]
    fn parses_the_compiled_catalog() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../po/locale/pt_BR/LC_MESSAGES/sulafat.mo"
        );
        let catalog = parse_mo(&std::fs::read(path).expect("build.rs compiles the catalogs"));
        assert_eq!(catalog.get("Connect").map(String::as_str), Some("Conectar"));
        assert!(parse_mo(b"not a catalog").is_empty());
    }
}

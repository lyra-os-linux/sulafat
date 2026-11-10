//! Ganchos de teste, ativos só em builds de debug: `SULAFAT_DEV_SCRIPT`
//! (JSON `[[regex, texto], …]`, digitado quando a saída casa) e
//! `SULAFAT_DEV_DUMP` (arquivo com o estado das abas e o texto do terminal).

use serde_json::Value;

fn var(name: &str) -> Option<String> {
    cfg!(debug_assertions)
        .then(|| std::env::var(name).ok())
        .flatten()
}

/// `null` quando nenhum gancho está ligado (sempre, em release): a interface
/// só cria o intervalo que lê o terminal quando recebe um objeto (sulafat#10).
#[tauri::command]
pub fn dev_config() -> Value {
    let connect = var("SULAFAT_DEV_CONNECT");
    let script = var("SULAFAT_DEV_SCRIPT").and_then(|s| serde_json::from_str::<Value>(&s).ok());
    let dump = var("SULAFAT_DEV_DUMP").is_some();
    if connect.is_none() && script.is_none() && !dump {
        return Value::Null;
    }
    serde_json::json!({ "connect": connect, "script": script, "dump": dump })
}

#[cfg(test)]
mod tests {
    #[test]
    fn no_hook_means_null() {
        for name in [
            "SULAFAT_DEV_CONNECT",
            "SULAFAT_DEV_SCRIPT",
            "SULAFAT_DEV_DUMP",
        ] {
            assert!(
                std::env::var_os(name).is_none(),
                "{name} definido no ambiente do teste"
            );
        }
        assert!(super::dev_config().is_null());
    }
}

#[tauri::command]
pub fn dev_dump(text: String) -> Result<(), String> {
    let path = var("SULAFAT_DEV_DUMP").ok_or("SULAFAT_DEV_DUMP not set")?;
    std::fs::write(path, text).map_err(|e| e.to_string())
}

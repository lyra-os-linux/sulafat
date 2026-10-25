//! Ganchos de teste, ativos só em builds de debug: `SULAFAT_DEV_SCRIPT`
//! (JSON `[[regex, texto], …]`, digitado quando a saída casa) e
//! `SULAFAT_DEV_DUMP` (arquivo com o estado das abas e o texto do terminal).

use serde_json::Value;

fn var(name: &str) -> Option<String> {
    cfg!(debug_assertions)
        .then(|| std::env::var(name).ok())
        .flatten()
}

#[tauri::command]
pub fn dev_config() -> Value {
    serde_json::json!({
        "connect": var("SULAFAT_DEV_CONNECT"),
        "script": var("SULAFAT_DEV_SCRIPT").and_then(|s| serde_json::from_str::<Value>(&s).ok()),
        "dump": var("SULAFAT_DEV_DUMP").is_some(),
    })
}

#[tauri::command]
pub fn dev_dump(text: String) -> Result<(), String> {
    let path = var("SULAFAT_DEV_DUMP").ok_or("SULAFAT_DEV_DUMP not set")?;
    std::fs::write(path, text).map_err(|e| e.to_string())
}

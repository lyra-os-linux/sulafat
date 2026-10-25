//! Terminal preferences (font, scrollback): a small settings file of our own — `sulafat-core`
//! only knows about `~/.ssh/config` and UI metadata, not terminal rendering choices — reusing
//! its XDG config-dir lookup so both files live side by side.

use std::fs;

use serde::{Deserialize, Serialize};

fn default_scrollback() -> u32 {
    10_000
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub font: Option<String>,
    #[serde(default = "default_scrollback")]
    pub scrollback_lines: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            font: None,
            scrollback_lines: default_scrollback(),
        }
    }
}

fn settings_path() -> Option<std::path::PathBuf> {
    sulafat_core::metadata::config_dir()
        .ok()
        .map(|dir| dir.join("settings.toml"))
}

impl Settings {
    pub fn load() -> Self {
        let Some(path) = settings_path() else {
            return Self::default();
        };
        let Ok(contents) = fs::read_to_string(path) else {
            return Self::default();
        };
        toml::from_str(&contents).unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(path) = settings_path() else { return };
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(contents) = toml::to_string_pretty(self) {
            let _ = fs::write(path, contents);
        }
    }
}

#[tauri::command]
pub fn settings_get() -> Settings {
    Settings::load()
}

/// Grava na hora, como o antigo diálogo do GTK; as abas abertas aplicam a
/// mudança na interface.
#[tauri::command]
pub fn settings_set(settings: Settings) -> Settings {
    settings.save();
    settings
}

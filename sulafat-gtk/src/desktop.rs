//! Integração com o GNOME (tema, destaque e fonte monoespaçada), clipboard,
//! seletor de arquivos, links e ações do sistema pelo logind.

use gio::prelude::*;
use glib::variant::ToVariant;
use gtk::prelude::{FileChooserExt, NativeDialogExt};
use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, State, WebviewWindow};

use crate::hosts::Hosts;
use crate::i18n::tr;

fn interface_settings() -> Option<gio::Settings> {
    let source = gio::SettingsSchemaSource::default()?;
    source.lookup("org.gnome.desktop.interface", true)?;
    Some(gio::Settings::new("org.gnome.desktop.interface"))
}

fn interface_state(settings: &gio::Settings) -> Value {
    let has_accent = settings
        .settings_schema()
        .is_some_and(|schema| schema.has_key("accent-color"));
    json!({
        "color_scheme": settings.string("color-scheme").to_string(),
        "accent": if has_accent { settings.string("accent-color").to_string() } else { "blue".to_owned() },
        "monospace_font": settings.string("monospace-font-name").to_string(),
    })
}

#[tauri::command]
pub fn interface_style() -> Value {
    interface_settings()
        .map(|settings| interface_state(&settings))
        .unwrap_or_else(
            || json!({ "color_scheme": "default", "accent": "blue", "monospace_font": "" }),
        )
}

/// Avisa a janela quando o tema, o destaque ou a fonte monoespaçada do GNOME mudam.
pub fn watch_interface(app: AppHandle) {
    let Some(settings) = interface_settings() else {
        return;
    };
    settings.connect_changed(None, move |settings, key| {
        if matches!(key, "color-scheme" | "accent-color" | "monospace-font-name") {
            let _ = app.emit("interface-changed", interface_state(settings));
        }
    });
    // GSettings só emite "changed" para chaves já lidas.
    let _ = interface_state(&settings);
    std::mem::forget(settings);
}

#[derive(Serialize)]
pub struct AppInfo {
    version: &'static str,
    locale: &'static str,
}

#[tauri::command]
pub fn app_info() -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        locale: crate::i18n::active_locale(),
    }
}

/// Erros que impediriam a interface de iniciar vão para o stderr.
#[tauri::command]
pub fn app_log(message: String) {
    eprintln!("sulafat (ui): {message}");
}

/// Famílias monoespaçadas instaladas, para o seletor de fonte do terminal.
#[tauri::command]
pub fn monospace_fonts() -> Vec<String> {
    let Ok(output) = std::process::Command::new("fc-list")
        .args([":spacing=mono", "family"])
        .output()
    else {
        return Vec::new();
    };
    let mut families: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.split(',').next())
        .map(|family| family.trim().to_owned())
        .filter(|family| !family.is_empty() && !family.starts_with('.'))
        .collect();
    families.sort_by_key(|family| family.to_lowercase());
    families.dedup();
    families
}

fn launch(uri: &str) -> Result<(), String> {
    let context = gio::AppLaunchContext::new();
    crate::webkit_env::clean_launch_context(&context);
    gio::AppInfo::launch_default_for_uri(uri, Some(&context)).map_err(|error| error.to_string())
}

/// Links clicados no terminal: só os esquemas que o VTE também abria.
#[tauri::command]
pub fn open_uri(uri: String) -> Result<(), String> {
    if !["http://", "https://", "ftp://"]
        .iter()
        .any(|scheme| uri.starts_with(scheme))
    {
        return Err(tr("Unsupported link."));
    }
    launch(&uri)
}

/// "Abrir arquivos": o `sftp://` do host no gerenciador de arquivos.
#[tauri::command]
pub fn open_files(hosts: State<'_, Hosts>, alias: String) -> Result<(), String> {
    let uri = hosts
        .sftp_uri(&alias)
        .ok_or_else(|| tr("Host not found."))?;
    launch(&uri)
}

/// Seletor nativo (portal) da chave privada, começando em `~/.ssh`.
#[tauri::command]
pub fn pick_identity_file(window: WebviewWindow) -> Option<String> {
    let parent = window.gtk_window().ok();
    let title = tr("Select private key");
    let chooser = gtk::FileChooserNative::new(
        Some(title.as_str()),
        parent.as_ref(),
        gtk::FileChooserAction::Open,
        None,
        None,
    );
    chooser.set_modal(true);
    chooser.set_show_hidden(true);
    if let Some(home) = std::env::var_os("HOME") {
        let ssh = std::path::Path::new(&home).join(".ssh");
        if ssh.is_dir() {
            let _ = chooser.set_current_folder(ssh);
        }
    }
    let accepted = chooser.run() == gtk::ResponseType::Accept;
    let path = accepted
        .then(|| chooser.filename())
        .flatten()
        .map(|path| path.to_string_lossy().into_owned());
    chooser.destroy();
    path
}

fn clipboard(primary: bool) -> gtk::Clipboard {
    gtk::Clipboard::get(&if primary {
        gtk::gdk::SELECTION_PRIMARY
    } else {
        gtk::gdk::SELECTION_CLIPBOARD
    })
}

/// Ctrl+Shift+V e clique do meio (seleção primária), como no VTE.
#[tauri::command]
pub fn clipboard_get(primary: bool) -> Option<String> {
    clipboard(primary)
        .wait_for_text()
        .map(|text| text.to_string())
}

/// Ctrl+Shift+C e a seleção primária ao selecionar com o mouse.
#[tauri::command]
pub fn clipboard_set(text: String, primary: bool) {
    clipboard(primary).set_text(&text);
}

fn login1_manager() -> Result<gio::DBusProxy, glib::Error> {
    gio::DBusProxy::for_bus_sync(
        gio::BusType::System,
        gio::DBusProxyFlags::NONE,
        None,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
        gio::Cancellable::NONE,
    )
}

/// Desligar, reiniciar ou encerrar a sessão gráfica (útil quando o Sulafat
/// é a única janela de um terminal de administração).
#[tauri::command]
pub async fn system_action(action: String) -> Result<(), String> {
    let method = match action.as_str() {
        "power-off" => "PowerOff",
        "restart" => "Reboot",
        "log-out" => "Terminate",
        _ => return Err(tr("System action failed")),
    };
    let result = (|| {
        let manager = login1_manager()?;
        if method == "Terminate" {
            let reply = manager.call_sync(
                "GetSessionByPID",
                Some(&(std::process::id(),).to_variant()),
                gio::DBusCallFlags::NONE,
                -1,
                gio::Cancellable::NONE,
            )?;
            let (session_path,) = reply.get::<(glib::variant::ObjectPath,)>().ok_or_else(|| {
                glib::Error::new(gio::IOErrorEnum::InvalidData, "invalid session path")
            })?;
            let session = gio::DBusProxy::for_bus_sync(
                gio::BusType::System,
                gio::DBusProxyFlags::NONE,
                None,
                "org.freedesktop.login1",
                &session_path,
                "org.freedesktop.login1.Session",
                gio::Cancellable::NONE,
            )?;
            session.call_sync(
                "Terminate",
                None,
                gio::DBusCallFlags::NONE,
                -1,
                gio::Cancellable::NONE,
            )?;
        } else {
            manager.call_sync(
                method,
                Some(&(true,).to_variant()),
                gio::DBusCallFlags::NONE,
                -1,
                gio::Cancellable::NONE,
            )?;
        }
        Ok::<(), glib::Error>(())
    })();
    result.map_err(|error| format!("{}: {error}", tr("System action failed")))
}

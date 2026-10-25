#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod desktop;
mod dev;
mod hosts;
mod i18n;
mod launcher_badge;
mod prefs;
mod terminal;

use tauri::Manager;

const APP_ID: &str = "org.lyraos.Sulafat";

fn main() {
    i18n::init();
    let filter = tracing_subscriber::EnvFilter::builder()
        .with_env_var("SULAFAT_LOG")
        .with_default_directive(tracing::level_filters::LevelFilter::WARN.into())
        .from_env_lossy();
    tracing_subscriber::fmt().with_env_filter(filter).init();

    // O prgname mantém o app_id/WM_CLASS que liga a janela ao org.lyraos.Sulafat.desktop.
    glib::set_prgname(Some(APP_ID));
    glib::set_application_name("Sulafat");

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .manage(hosts::Hosts::load())
        .manage(terminal::Sessions::default())
        .setup(|app| {
            desktop::watch_interface(app.handle().clone());
            hosts::watch_config(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            desktop::app_info,
            desktop::app_log,
            desktop::interface_style,
            desktop::monospace_fonts,
            desktop::open_uri,
            desktop::open_files,
            desktop::pick_identity_file,
            desktop::clipboard_get,
            desktop::clipboard_set,
            desktop::system_action,
            dev::dev_config,
            dev::dev_dump,
            i18n::i18n_catalog,
            hosts::hosts_list,
            hosts::host_save,
            hosts::host_duplicate,
            hosts::host_delete,
            hosts::backup_restore,
            prefs::settings_get,
            prefs::settings_set,
            terminal::quick_target,
            terminal::session_open,
            terminal::session_spawn,
            terminal::session_write,
            terminal::session_resize,
            terminal::session_ack,
            terminal::session_close,
        ])
        .run(tauri::generate_context!())
        .expect("failed to start Sulafat");
}

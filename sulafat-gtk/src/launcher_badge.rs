//! Badges the app's dock/taskbar icon with the number of active SSH sessions, via the
//! `com.canonical.Unity.LauncherEntry` D-Bus signal honored by dash-to-dock and similar docks.
//! Docks that don't understand the signal simply never see it — this is a bare broadcast on the
//! session bus, no reply expected.

use std::sync::OnceLock;

use gio::prelude::*;

const APP_URI: &str = "application://org.lyraos.Sulafat.desktop";
const OBJECT_PATH: &str = "/org/lyraos/Sulafat";

static CONNECTION: OnceLock<Option<gio::DBusConnection>> = OnceLock::new();

/// Reports how many saved hosts have an active session (quick connections, which have no row
/// in the sidebar, don't count — same rule as the former GTK window).
pub fn report(count: u32) {
    let connection = CONNECTION
        .get_or_init(|| gio::bus_get_sync(gio::BusType::Session, None::<&gio::Cancellable>).ok());
    let Some(connection) = connection else {
        return;
    };
    let properties = glib::VariantDict::new(None);
    properties.insert("count", count as i64);
    properties.insert("count-visible", count > 0);
    let params = (APP_URI, properties.end()).to_variant();
    let _ = connection.emit_signal(
        None,
        OBJECT_PATH,
        "com.canonical.Unity.LauncherEntry",
        "Update",
        Some(&params),
    );
}

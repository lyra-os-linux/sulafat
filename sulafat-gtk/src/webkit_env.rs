//! Contorno do crash do WebKitGTK 2.52 ao encerrar (sulafat#6, vega#166).
//!
//! O renderer DMA-BUF do WebKit guarda o dispositivo GBM num singleton
//! estático (`WebCore::GBMDevice`). No `exit()` do WebKitWebProcess, o
//! destrutor dele corre contra o desmonte do Mesa e do glvnd e corrompe o
//! heap ("corrupted double-linked list"), deixando um coredump. Com o
//! renderer desligado, o compositing acelerado continua ligado. Retirar
//! quando o WebKit corrigir.

use gio::prelude::*;
use portable_pty::CommandBuilder;
use std::ffi::OsStr;
use std::sync::atomic::{AtomicBool, Ordering};

pub const DISABLE_DMABUF: &str = "WEBKIT_DISABLE_DMABUF_RENDERER";

/// Se a variável foi definida pelo Sulafat (e não herdada do usuário).
static SET_BY_SULAFAT: AtomicBool = AtomicBool::new(false);

/// Desliga o renderer DMA-BUF do WebKit. Chamar no início do `main`, antes
/// de qualquer thread. Um valor já definido pelo usuário é respeitado.
pub fn apply() {
    if should_set(std::env::var_os(DISABLE_DMABUF).as_deref()) {
        // SAFETY: chamado no início do main, com uma única thread.
        unsafe { std::env::set_var(DISABLE_DMABUF, "1") };
        SET_BY_SULAFAT.store(true, Ordering::Relaxed);
    }
}

fn should_set(existing: Option<&OsStr>) -> bool {
    existing.is_none()
}

/// O `ssh` das sessões não herda o contorno.
pub fn clean_pty_command(command: &mut CommandBuilder) {
    strip_pty_command(command, SET_BY_SULAFAT.load(Ordering::Relaxed));
}

fn strip_pty_command(command: &mut CommandBuilder, set_by_sulafat: bool) {
    if set_by_sulafat {
        command.env_remove(DISABLE_DMABUF);
    }
}

/// Apps abertos pelos links do terminal também não.
pub fn clean_launch_context(context: &gio::AppLaunchContext) {
    if SET_BY_SULAFAT.load(Ordering::Relaxed) {
        context.unsetenv(DISABLE_DMABUF);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sets_only_when_the_user_did_not() {
        assert!(should_set(None));
        assert!(!should_set(Some(OsStr::new("0"))));
        assert!(!should_set(Some(OsStr::new("1"))));
    }

    #[test]
    fn ssh_loses_the_workaround_set_by_sulafat() {
        let mut command = CommandBuilder::new("ssh");
        command.env(DISABLE_DMABUF, "1");
        strip_pty_command(&mut command, true);
        assert!(command.get_env(DISABLE_DMABUF).is_none());
    }

    #[test]
    fn ssh_keeps_a_value_the_user_set() {
        let mut command = CommandBuilder::new("ssh");
        command.env(DISABLE_DMABUF, "0");
        strip_pty_command(&mut command, false);
        assert_eq!(command.get_env(DISABLE_DMABUF), Some(OsStr::new("0")));
    }
}

//! Sessões: o `ssh` do OpenSSH num pseudoterminal, com a saída enviada à aba
//! (xterm.js) por um canal binário. Como no VTE, o `ssh` cuida sozinho de
//! senha, passphrase e confirmação de fingerprint, dentro do terminal; este
//! módulo nunca intercepta nem registra o conteúdo da sessão.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::time::Duration;

use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use sulafat_core::command::{build_ssh_command, parse_quick_target, ConnectTarget};
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, Emitter, Manager};

use crate::hosts::is_plain_alias;
use crate::i18n::tr;
use crate::launcher_badge;

/// Saída ainda não desenhada pelo xterm.js: acima do limite, a leitura do PTY
/// para e o `ssh` espera, em vez de a janela acumular megabytes.
const HIGH_WATER: usize = 2 * 1024 * 1024;
const LOW_WATER: usize = 512 * 1024;

#[derive(Default)]
struct Flow {
    pending: Mutex<(usize, bool)>,
    drained: Condvar,
}

impl Flow {
    fn add(&self, bytes: usize) {
        let mut guard = self.pending.lock().unwrap();
        guard.0 += bytes;
        while guard.0 > HIGH_WATER && !guard.1 {
            guard = self.drained.wait(guard).unwrap();
        }
    }

    fn ack(&self, bytes: usize) {
        let mut guard = self.pending.lock().unwrap();
        guard.0 = guard.0.saturating_sub(bytes);
        if guard.0 < LOW_WATER {
            self.drained.notify_all();
        }
    }

    fn reset(&self) {
        *self.pending.lock().unwrap() = (0, false);
    }

    fn close(&self) {
        self.pending.lock().unwrap().1 = true;
        self.drained.notify_all();
    }
}

struct Running {
    master: Box<dyn MasterPty + Send>,
    input: mpsc::Sender<Vec<u8>>,
    killer: Box<dyn ChildKiller + Send + Sync>,
}

struct Session {
    argv: Vec<String>,
    alias: Option<String>,
    output: Channel<InvokeResponseBody>,
    flow: Arc<Flow>,
    generation: u32,
    running: Option<Running>,
}

#[derive(Default)]
pub struct Sessions {
    next: AtomicU32,
    map: Mutex<HashMap<u32, Session>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    Alias(String),
    Quick(String),
}

#[derive(Serialize)]
pub struct Opened {
    id: u32,
    title: String,
}

#[derive(Clone, Serialize)]
struct SessionState {
    id: u32,
    running: bool,
    code: Option<u32>,
    signal: Option<String>,
}

fn quick_title(target: &ConnectTarget) -> String {
    match target {
        ConnectTarget::Alias(alias) => alias.clone(),
        ConnectTarget::Quick { user, host, .. } => match user {
            Some(u) => format!("{u}@{host}"),
            None => host.clone(),
        },
    }
}

/// Valida o texto da conexão rápida (`[usuário@]host[:porta]`) e devolve o
/// título da aba, ou nada se o destino não é aceito.
#[tauri::command]
pub fn quick_target(text: String) -> Option<String> {
    parse_quick_target(&text).map(|target| quick_title(&target))
}

fn size(cols: u16, rows: u16) -> PtySize {
    PtySize {
        rows: rows.max(2),
        cols: cols.max(2),
        pixel_width: 0,
        pixel_height: 0,
    }
}

impl Sessions {
    fn report_badge(&self) {
        let map = self.map.lock().unwrap();
        let mut aliases: Vec<&str> = map
            .values()
            .filter(|s| s.running.is_some())
            .filter_map(|s| s.alias.as_deref())
            .collect();
        aliases.sort_unstable();
        aliases.dedup();
        launcher_badge::report(aliases.len() as u32);
    }

    fn spawn(&self, app: &AppHandle, id: u32, cols: u16, rows: u16) -> Result<(), String> {
        let mut map = self.map.lock().unwrap();
        let session = map.get_mut(&id).ok_or_else(|| tr("Session not found."))?;
        if session.running.is_some() {
            return Ok(());
        }
        let failed = |error: &dyn std::fmt::Display| {
            tracing::error!(%error, "falha ao iniciar o ssh");
            format!("{}: {error}", tr("Could not start ssh"))
        };
        let pair = native_pty_system()
            .openpty(size(cols, rows))
            .map_err(|e| failed(&e))?;
        let mut command = CommandBuilder::new(&session.argv[0]);
        command.args(&session.argv[1..]);
        if let Some(home) = std::env::var_os("HOME") {
            command.cwd(home);
        }
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        let mut child = pair.slave.spawn_command(command).map_err(|e| failed(&e))?;
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().map_err(|e| failed(&e))?;
        let mut writer = pair.master.take_writer().map_err(|e| failed(&e))?;
        let killer = child.clone_killer();

        session.generation += 1;
        let generation = session.generation;
        session.flow.reset();
        let flow = session.flow.clone();
        let output = session.output.clone();

        let (input, inbox) = mpsc::channel::<Vec<u8>>();
        std::thread::Builder::new()
            .name(format!("sulafat-pty-in-{id}"))
            .spawn(move || {
                for data in inbox {
                    if writer
                        .write_all(&data)
                        .and_then(|()| writer.flush())
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .map_err(|e| failed(&e))?;

        let (drained_tx, drained_rx) = mpsc::channel::<()>();
        std::thread::Builder::new()
            .name(format!("sulafat-pty-out-{id}"))
            .spawn(move || {
                let mut buffer = vec![0u8; 64 * 1024];
                loop {
                    match reader.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if output
                                .send(InvokeResponseBody::Raw(buffer[..n].to_vec()))
                                .is_err()
                            {
                                break;
                            }
                            flow.add(n);
                        }
                    }
                }
                let _ = drained_tx.send(());
            })
            .map_err(|e| failed(&e))?;

        let app_for_wait = app.clone();
        std::thread::Builder::new()
            .name(format!("sulafat-pty-wait-{id}"))
            .spawn(move || {
                let status = child.wait();
                // A última saída (por exemplo "Connection closed") chega antes do aviso de fim.
                let _ = drained_rx.recv_timeout(Duration::from_millis(500));
                let sessions = app_for_wait.state::<Sessions>();
                {
                    let mut map = sessions.map.lock().unwrap();
                    match map.get_mut(&id) {
                        Some(s) if s.generation == generation => s.running = None,
                        _ => return,
                    }
                }
                let (code, signal) = match status {
                    Ok(status) => (Some(status.exit_code()), status.signal().map(str::to_owned)),
                    Err(_) => (None, None),
                };
                let _ = app_for_wait.emit(
                    "session-state",
                    SessionState {
                        id,
                        running: false,
                        code,
                        signal,
                    },
                );
                sessions.report_badge();
            })
            .map_err(|e| failed(&e))?;

        session.running = Some(Running {
            master: pair.master,
            input,
            killer,
        });
        drop(map);
        let _ = app.emit(
            "session-state",
            SessionState {
                id,
                running: true,
                code: None,
                signal: None,
            },
        );
        self.report_badge();
        Ok(())
    }

    fn with_running<T>(&self, id: u32, f: impl FnOnce(&mut Running) -> T) -> Option<T> {
        self.map
            .lock()
            .unwrap()
            .get_mut(&id)
            .and_then(|s| s.running.as_mut())
            .map(f)
    }
}

/// Abre uma aba nova e já inicia o `ssh` no tamanho atual do terminal.
#[tauri::command]
pub fn session_open(
    app: AppHandle,
    sessions: tauri::State<'_, Sessions>,
    target: Target,
    cols: u16,
    rows: u16,
    output: Channel<InvokeResponseBody>,
) -> Result<Opened, String> {
    let (connect, alias) = match target {
        Target::Alias(alias) if is_plain_alias(&alias) => {
            (ConnectTarget::Alias(alias.clone()), Some(alias))
        }
        Target::Alias(_) => return Err(tr("This host is a pattern and cannot be connected to.")),
        Target::Quick(text) => (
            parse_quick_target(&text).ok_or_else(|| tr("Use user@host or user@host:port."))?,
            None,
        ),
    };
    let id = sessions.next.fetch_add(1, Ordering::Relaxed) + 1;
    let title = quick_title(&connect);
    sessions.map.lock().unwrap().insert(
        id,
        Session {
            argv: build_ssh_command(&connect),
            alias,
            output,
            flow: Arc::default(),
            generation: 0,
            running: None,
        },
    );
    if let Err(error) = sessions.spawn(&app, id, cols, rows) {
        sessions.map.lock().unwrap().remove(&id);
        return Err(error);
    }
    Ok(Opened { id, title })
}

/// "Reconectar": o mesmo comando, na mesma aba.
#[tauri::command]
pub fn session_spawn(
    app: AppHandle,
    sessions: tauri::State<'_, Sessions>,
    id: u32,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    sessions.spawn(&app, id, cols, rows)
}

/// Teclas e textos colados, já codificados pelo xterm.js (`bytes` traz os
/// relatórios de mouse em modo binário, que não são UTF-8).
#[tauri::command]
pub fn session_write(
    sessions: tauri::State<'_, Sessions>,
    id: u32,
    data: Option<String>,
    bytes: Option<Vec<u8>>,
) {
    let input = data.map(String::into_bytes).or(bytes).unwrap_or_default();
    if input.is_empty() {
        return;
    }
    sessions.with_running(id, |r| {
        let _ = r.input.send(input);
    });
}

#[tauri::command]
pub fn session_resize(sessions: tauri::State<'_, Sessions>, id: u32, cols: u16, rows: u16) {
    sessions.with_running(id, |r| {
        let _ = r.master.resize(size(cols, rows));
    });
}

/// O xterm.js desenhou `bytes` da saída: libera a leitura do PTY.
#[tauri::command]
pub fn session_ack(sessions: tauri::State<'_, Sessions>, id: u32, bytes: usize) {
    if let Some(session) = sessions.map.lock().unwrap().get(&id) {
        session.flow.ack(bytes);
    }
}

/// Fecha a aba: encerra o `ssh` (se ainda roda) e esquece a sessão.
#[tauri::command]
pub fn session_close(sessions: tauri::State<'_, Sessions>, id: u32) {
    let removed = sessions.map.lock().unwrap().remove(&id);
    if let Some(mut session) = removed {
        session.flow.close();
        if let Some(mut running) = session.running.take() {
            let _ = running.killer.kill();
        }
    }
    sessions.report_badge();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quick_titles_match_the_former_tabs() {
        assert_eq!(
            quick_target("ana@10.0.0.5:2222".into()).as_deref(),
            Some("ana@10.0.0.5")
        );
        assert_eq!(quick_target("server".into()).as_deref(), Some("server"));
        assert_eq!(quick_target("  ".into()), None);
    }

    #[test]
    fn flow_control_blocks_until_acknowledged() {
        let flow = Arc::new(Flow::default());
        flow.add(HIGH_WATER);
        let producer = {
            let flow = flow.clone();
            std::thread::spawn(move || flow.add(1))
        };
        std::thread::sleep(Duration::from_millis(50));
        assert!(!producer.is_finished());
        flow.ack(HIGH_WATER);
        producer.join().unwrap();
        // Fechar a aba solta uma leitura parada no limite.
        flow.close();
        flow.add(HIGH_WATER * 2);
    }
}

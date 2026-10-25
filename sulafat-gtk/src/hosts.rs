//! Hosts do `~/.ssh/config` e metadados de interface (grupo, cor, notas), com
//! as mesmas regras da antiga janela GTK: o `sulafat-core` grava o arquivo de
//! forma atômica e com backup, e uma falha recarrega o que está no disco.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use sulafat_core::metadata::{HostMeta, Metadata};
use sulafat_core::ssh_config::{validate_host, SshConfig, SshHost};
use tauri::{AppHandle, Emitter, Manager};

use crate::i18n::{format as tr_format, tr};

struct State {
    cfg: SshConfig,
    metadata: Metadata,
    load_error: Option<String>,
}

pub struct Hosts(Mutex<State>);

/// `SshHost` como a interface o vê e devolve.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostData {
    pub alias: String,
    pub host_name: Option<String>,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub identity_file: Option<String>,
    pub proxy_jump: Option<String>,
    #[serde(default)]
    pub extra: String,
    #[serde(default)]
    pub read_only: bool,
}

impl From<SshHost> for HostData {
    fn from(host: SshHost) -> Self {
        Self {
            alias: host.alias,
            host_name: host.host_name,
            user: host.user,
            port: host.port,
            identity_file: host.identity_file,
            proxy_jump: host.proxy_jump,
            extra: host.extra,
            read_only: host.read_only,
        }
    }
}

#[derive(Clone, Serialize)]
pub struct HostEntry {
    host: HostData,
    meta: HostMeta,
    /// Um alias simples, que o `ssh` consegue usar como destino (blocos com
    /// curingas ou vários padrões só servem de modelo).
    connectable: bool,
}

#[derive(Clone, Serialize)]
pub struct HostList {
    hosts: Vec<HostEntry>,
    groups: Vec<String>,
    /// Falha ao ler o `~/.ssh/config`: a interface fica somente leitura.
    load_error: Option<String>,
}

pub fn is_plain_alias(alias: &str) -> bool {
    !alias.is_empty()
        && !alias.starts_with('-')
        && !alias
            .chars()
            .any(|c| matches!(c, '*' | '?' | '!' | ',') || c.is_whitespace() || c.is_control())
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.is_empty())
}

// Preserve an explicit default port on an unchanged form; removing it could expose
// a later Port/Include. An actual edit to 22 must also remain explicit.
fn edited_port(original: Option<u16>, selected: Option<u16>) -> Option<u16> {
    match selected {
        Some(port) if port == original.unwrap_or(22) => original,
        other => other,
    }
}

fn unique_alias(base: &str, existing: &[String]) -> String {
    let candidate = format!("{base}-copia");
    if !existing.contains(&candidate) {
        return candidate;
    }
    let mut n = 2;
    loop {
        let candidate = format!("{base}-copia-{n}");
        if !existing.contains(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

impl State {
    fn known_aliases(&self) -> Vec<String> {
        self.cfg
            .list_hosts()
            .into_iter()
            .filter(|h| !h.read_only)
            .map(|h| h.alias)
            .collect()
    }

    fn persist(&mut self) -> Result<(), String> {
        let saved = self.cfg.save().map_err(|e| e.to_string()).and_then(|()| {
            let known = self.known_aliases();
            self.metadata
                .save(&known)
                .map_err(|e| format!("SSH config salvo, mas os metadados falharam: {e}"))
        });
        if saved.is_err() {
            self.reload_persisted();
        }
        saved
    }

    fn reload_persisted(&mut self) {
        if let Ok(cfg) = SshConfig::load_from(self.cfg.path()) {
            self.cfg = cfg;
        }
        if let Ok(metadata) = Metadata::load() {
            self.metadata = metadata;
        }
    }

    fn writable(&self) -> Result<(), String> {
        match self.load_error {
            Some(_) => Err(tr(
                "Sulafat is read-only until the configuration can be loaded.",
            )),
            None => Ok(()),
        }
    }

    fn editable_host(&self, alias: &str) -> Result<SshHost, String> {
        self.cfg
            .list_hosts()
            .into_iter()
            .find(|h| h.alias == alias && !h.read_only)
            .ok_or_else(|| tr("Host not found."))
    }

    fn list(&self) -> HostList {
        let hosts = self
            .cfg
            .list_hosts()
            .into_iter()
            .map(|host| HostEntry {
                meta: self.metadata.get(&host.alias).cloned().unwrap_or_default(),
                connectable: is_plain_alias(&host.alias),
                host: host.into(),
            })
            .collect();
        HostList {
            hosts,
            groups: self.metadata.groups(),
            load_error: self.load_error.clone(),
        }
    }
}

impl Hosts {
    pub fn load() -> Self {
        let (cfg, load_error) = match SshConfig::load() {
            Ok(cfg) => (cfg, None),
            Err(e) => {
                tracing::error!("falha ao carregar ~/.ssh/config: {e}");
                (
                    SshConfig::empty_at_default_path().expect("default SSH path"),
                    Some(e.to_string()),
                )
            }
        };
        let metadata = Metadata::load().unwrap_or_default();
        Self(Mutex::new(State {
            cfg,
            metadata,
            load_error,
        }))
    }

    /// Endereço `sftp://` do host, para o gerenciador de arquivos.
    pub fn sftp_uri(&self, alias: &str) -> Option<String> {
        let state = self.0.lock().unwrap();
        let host = state
            .cfg
            .list_hosts()
            .into_iter()
            .find(|h| h.alias == alias && is_plain_alias(&h.alias))?;
        let host_part = host.host_name.clone().unwrap_or_else(|| host.alias.clone());
        let user_part = host
            .user
            .as_ref()
            .map(|u| format!("{u}@"))
            .unwrap_or_default();
        let port_part = host.port.map(|p| format!(":{p}")).unwrap_or_default();
        Some(format!("sftp://{user_part}{host_part}{port_part}/"))
    }
}

#[tauri::command]
pub fn hosts_list(state: tauri::State<'_, Hosts>) -> HostList {
    state.0.lock().unwrap().list()
}

/// Cria (`original` vazio) ou atualiza um host. Renomear o alias atualiza o
/// bloco existente no lugar.
#[tauri::command]
pub fn host_save(
    state: tauri::State<'_, Hosts>,
    original: Option<String>,
    host: HostData,
    meta: HostMeta,
) -> Result<HostList, String> {
    let mut s = state.0.lock().unwrap();
    s.writable()?;
    let base = match &original {
        Some(alias) => s.editable_host(alias)?,
        None => SshHost::default(),
    };
    let alias = host.alias.trim().to_owned();
    if alias.is_empty() {
        return Err(tr("The alias cannot be empty."));
    }
    let renamed = original.as_deref() != Some(alias.as_str());
    if renamed && s.cfg.list_hosts().iter().any(|h| h.alias == alias) {
        return Err(tr_format(
            "A host named “{alias}” already exists.",
            &[("alias", &alias)],
        ));
    }
    let edited = SshHost {
        alias: alias.clone(),
        host_name: non_empty(host.host_name),
        user: non_empty(host.user),
        port: edited_port(base.port, host.port),
        identity_file: non_empty(host.identity_file),
        proxy_jump: non_empty(host.proxy_jump),
        extra: host.extra,
        read_only: false,
    };
    validate_host(&edited).map_err(|e| e.to_string())?;
    match original.as_deref() {
        Some(previous) if previous != alias => s.cfg.upsert_host_renaming(previous, edited),
        _ => s.cfg.upsert_host(edited),
    }
    let meta = HostMeta {
        group: non_empty(meta.group.map(|g| g.trim().to_owned())),
        color: non_empty(meta.color),
        notes: non_empty(meta.notes.map(|n| n.trim().to_owned())),
    };
    s.metadata.set(alias, meta);
    s.persist()?;
    Ok(s.list())
}

#[tauri::command]
pub fn host_duplicate(state: tauri::State<'_, Hosts>, alias: String) -> Result<String, String> {
    let mut s = state.0.lock().unwrap();
    s.writable()?;
    let mut copy = s.editable_host(&alias)?;
    let meta = s.metadata.get(&alias).cloned().unwrap_or_default();
    let existing: Vec<String> = s.cfg.list_hosts().into_iter().map(|h| h.alias).collect();
    copy.alias = unique_alias(&alias, &existing);
    let new_alias = copy.alias.clone();
    s.cfg.upsert_host(copy);
    s.metadata.set(new_alias.clone(), meta);
    s.persist()?;
    Ok(new_alias)
}

#[tauri::command]
pub fn host_delete(state: tauri::State<'_, Hosts>, alias: String) -> Result<(), String> {
    let mut s = state.0.lock().unwrap();
    s.writable()?;
    s.editable_host(&alias)?;
    s.cfg.remove_host(&alias);
    s.metadata.set(alias, HostMeta::default());
    s.persist()
}

/// Restaura a geração mais recente do backup. Também tira a interface do modo
/// somente leitura quando o arquivo restaurado pode ser lido.
#[tauri::command]
pub fn backup_restore(state: tauri::State<'_, Hosts>) -> Result<HostList, String> {
    let mut s = state.0.lock().unwrap();
    s.cfg.restore_backup(1).map_err(|e| e.to_string())?;
    match SshConfig::load_from(s.cfg.path()) {
        Ok(cfg) => {
            s.cfg = cfg;
            s.load_error = None;
        }
        Err(e) => s.load_error = Some(e.to_string()),
    }
    Ok(s.list())
}

/// Recarrega quando o `~/.ssh/config` muda por fora (outro editor, `ssh-copy-id`…).
pub fn watch_config(app: AppHandle) {
    let path = app
        .state::<Hosts>()
        .0
        .lock()
        .unwrap()
        .cfg
        .path()
        .to_path_buf();
    let Ok(watcher) = sulafat_core::watch::ConfigWatcher::watch(&path) else {
        tracing::warn!("sem monitoramento de mudanças em {}", path.display());
        return;
    };
    std::thread::Builder::new()
        .name("sulafat-config-watch".to_owned())
        .spawn(move || {
            while watcher.receiver().recv().is_ok() {
                std::thread::sleep(sulafat_core::watch::DEBOUNCE);
                watcher.poll();
                let Ok(fresh) = SshConfig::load_from(&path) else {
                    continue;
                };
                let list = {
                    let state = app.state::<Hosts>();
                    let mut s = state.0.lock().unwrap();
                    s.cfg = fresh;
                    s.load_error = None;
                    s.list()
                };
                let _ = app.emit("hosts-changed", list);
            }
        })
        .expect("failed to spawn the config watcher");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_preserves_explicit_and_inherited_ports() {
        assert_eq!(edited_port(Some(22), Some(22)), Some(22));
        assert_eq!(edited_port(None, Some(22)), None);
        assert_eq!(edited_port(Some(2200), Some(22)), Some(22));
        assert_eq!(edited_port(None, Some(2200)), Some(2200));
        assert_eq!(edited_port(Some(2200), None), None);
    }

    #[test]
    fn duplicate_aliases_never_collide() {
        let existing = vec!["web".to_owned(), "web-copia".to_owned()];
        assert_eq!(unique_alias("db", &existing), "db-copia");
        assert_eq!(unique_alias("web", &existing), "web-copia-2");
    }

    #[test]
    fn only_plain_aliases_are_connection_targets() {
        assert!(is_plain_alias("web01"));
        assert!(is_plain_alias("db.example.com"));
        for pattern in ["*.example.com", "web? db", "!bad", "-oProxyCommand", ""] {
            assert!(!is_plain_alias(pattern), "{pattern}");
        }
    }
}

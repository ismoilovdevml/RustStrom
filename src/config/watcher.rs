//! Configuration hot reload.
//!
//! Watches the directory of the config file instead of the file itself: editors (vim, `sed -i`),
//! `mv` and Kubernetes ConfigMaps replace the file atomically, which silently ends a watch on the
//! original inode. An invalid new configuration is logged and the running one is kept.

use super::RuntimeConfig;
use crate::state::State;
use log::{info, trace, warn};
use notify::{watcher, DebouncedEvent, RecursiveMode, Watcher};
use pingora_core::server::ShutdownWatch;
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::{mpsc::channel, Arc},
    thread,
    time::Duration,
};
use tokio::sync::watch;

pub async fn run(state: Arc<State>, mut shutdown: ShutdownWatch) {
    let mut changes = match start_watcher(&state.config_path) {
        Ok(receiver) => receiver,
        Err(e) => {
            warn!("Configuration hot reload disabled: {}", e);
            return;
        }
    };
    loop {
        tokio::select! {
            changed = changes.changed() => {
                if changed.is_err() {
                    warn!("Configuration hot reload stopped");
                    return;
                }
            }
            _ = shutdown.changed() => return,
        }
        reload(&state);
    }
}

/// Loads the configuration again and swaps it in when it is valid.
pub fn reload(state: &State) {
    let old_config = state.config.load_full();
    match super::load(&state.config_path, &state.metrics, Some(&old_config)) {
        Ok(new_config) => {
            warn_about_ineffectual_config_changes(&old_config, &new_config);
            let removed = removed_backends(&old_config, &new_config);
            state.config.store(Arc::new(new_config));
            for address in removed {
                state.metrics.remove_backend(&address);
            }
            state.on_config_change();
            info!("Reloaded configuration");
        }
        Err(e) => {
            warn!("Could not reload configuration due to: {}", e);
            warn!("Keeping old configuration")
        }
    }
}

/// Backend addresses of `old` that no configured pool of `new` uses anymore.
fn removed_backends(old: &RuntimeConfig, new: &RuntimeConfig) -> Vec<String> {
    let kept: std::collections::HashSet<&str> = new
        .pools
        .iter()
        .flat_map(|p| p.backends.iter().map(|b| b.address.as_str()))
        .collect();
    let mut removed: Vec<String> = old
        .pools
        .iter()
        .flat_map(|p| p.backends.iter().map(|b| b.address.as_str()))
        .filter(|address| !kept.contains(address))
        .map(str::to_string)
        .collect();
    removed.sort();
    removed.dedup();
    removed
}

fn warn_about_ineffectual_config_changes(old: &RuntimeConfig, new: &RuntimeConfig) {
    let restart_only = [
        ("http_address", old.http_address != new.http_address),
        ("https_address", old.https_address != new.https_address),
        (
            "prometheus_metrics_address",
            old.metrics_address != new.metrics_address,
        ),
        ("threads", old.process.threads != new.process.threads),
        (
            "grace_period_secs",
            old.process.grace_period_secs != new.process.grace_period_secs,
        ),
        (
            "upstream_keepalive_pool_size",
            old.process.upstream_keepalive_pool_size != new.process.upstream_keepalive_pool_size,
        ),
        (
            "upgrade_sock",
            old.process.upgrade_sock != new.process.upgrade_sock,
        ),
        ("pid_file", old.process.pid_file != new.process.pid_file),
        (
            "h2_idle_timeout_ms",
            old.client.h2_idle_timeout != new.client.h2_idle_timeout,
        ),
    ];
    for (name, changed) in restart_only {
        if changed {
            warn!("A restart is required for the new {} to take effect", name);
        }
    }
    if new.process.max_retries > old.process.max_retries {
        warn!(
            "retries above {} take effect after a restart",
            old.process.max_retries.saturating_sub(1)
        );
    }
}

fn start_watcher(path: &Path) -> Result<watch::Receiver<u64>, String> {
    let config_path = path
        .canonicalize()
        .map_err(|e| format!("could not resolve '{}': {}", path.display(), e))?;
    let directory = config_path
        .parent()
        .ok_or_else(|| "config path has no parent".to_string())?
        .to_path_buf();
    let file_name = config_path.file_name().map(|it| it.to_os_string());
    let (sender, receiver) = watch::channel(0u64);
    thread::Builder::new()
        .name("config-watcher".into())
        .spawn(move || {
            if let Err(e) = watch_blocking(&directory, file_name, sender) {
                warn!("Configuration hot reload stopped: {}", e);
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(receiver)
}

fn watch_blocking(
    directory: &Path,
    file_name: Option<OsString>,
    async_sender: watch::Sender<u64>,
) -> Result<(), String> {
    let (sender, receiver) = channel();
    let mut watcher =
        watcher(sender, Duration::from_secs(1)).map_err(|e| format!("watch error: {:?}", e))?;
    watcher
        .watch(directory, RecursiveMode::NonRecursive)
        .map_err(|e| format!("watch error: {:?}", e))?;
    let mut generation = 0u64;
    loop {
        let event = receiver.recv().map_err(|e| e.to_string())?;
        let touched: Vec<PathBuf> = match &event {
            DebouncedEvent::Create(p) | DebouncedEvent::Write(p) | DebouncedEvent::Chmod(p) => {
                vec![p.clone()]
            }
            DebouncedEvent::Rename(from, to) => vec![from.clone(), to.clone()],
            DebouncedEvent::Remove(p) => {
                warn!("'{}' was deleted", p.display());
                vec![]
            }
            DebouncedEvent::Rescan => vec![directory.to_path_buf()],
            other => {
                trace!("{:?}", other);
                vec![]
            }
        };
        let relevant = touched.iter().any(|p| {
            p.as_path() == directory
                || p.file_name() == file_name.as_deref()
                // Kubernetes swaps the `..data` symlink when a ConfigMap changes
                || p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(".."))
        });
        if relevant {
            generation = generation.wrapping_add(1);
            async_sender.send(generation).map_err(|e| e.to_string())?;
        }
    }
}

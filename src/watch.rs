//! Hot-reload de config por mtime poll (std puro, sin watcher deps).
//! Suficiente: la config cambia una vez por semana, no cada frame.

use std::path::Path;
use std::time::SystemTime;

pub fn mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

/// Estado incremental del watcher: compara mtime del config.
#[derive(Default)]
pub struct Watcher {
    last: Option<SystemTime>,
}

impl Watcher {
    pub fn new(path: &Path) -> Self {
        Self { last: mtime(path) }
    }

    /// true si el archivo cambió desde la última vista (o apareció).
    pub fn changed(&mut self, path: &Path) -> bool {
        let mt = mtime(path);
        if mt == self.last {
            return false;
        }
        self.last = mt;
        mt.is_some()
    }
}

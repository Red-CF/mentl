//! LspHub: un client por lenguaje; eventos hacia el main loop; requests con
//! req_id app-side (main es single-threaded → ids simples y sin oneshots).

pub mod client;
pub mod transport;
pub mod types;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};

use crate::config::{LspCfg, ServerCfg};

#[derive(Debug, Clone)]
pub struct Diag {
    pub line: usize,       // 0-based
    pub col: usize,        // graphemes (ya convertida desde UTF-16 al ingest)
    pub severity: u32,     // 1 error 2 warn 3 info 4 hint
    pub msg: String,
}

pub enum LspEvent {
    Diagnostics { uri: String, diags: Vec<types::Diag> },
    Response { req_id: u64, body: serde_json::Value },
    Started { lang: &'static str },
    StartedFailed { lang: &'static str, err: String },
    Died { lang: &'static str, err: String },
}

pub enum Cmd {
    /// Lenguaje arrancado correctamente.
    Started,
}

pub struct ClientHandle {
    pub tx: Sender<client::Cmd>,
    pub lang: &'static str,
    pub ready: bool,
}

pub struct LspHub {
    pub tx: Sender<LspEvent>,
    pub rx: Receiver<LspEvent>,
   clients: HashMap<&'static str, ClientHandle>,
    next_req: u64,
    pub enabled: bool,
}

impl Default for LspHub {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self { tx, rx, clients: HashMap::new(), next_req: 1, enabled: false }
    }
}

impl LspHub {
    /// Arranca (lazy) el server para `lang` si hay config y no está ya vivo.
    pub fn ensure_client(&mut self, lang: &'static str, cfg: &LspCfg, server: Option<&ServerCfg>) {
        if !cfg.enabled || self.clients.contains_key(lang) {
            return;
        }
        let Some(s) = server else { return };
        let Some(h) = client::spawn(lang, &s.command, self.tx.clone()) else { return };
        let h = ClientHandle { tx: h.tx, lang: h.lang, ready: false };
        self.clients.insert(lang, h);
    }

    pub fn ready(&self, lang: &str) -> Option<&ClientHandle> {
        self.clients.get(lang)
    }

    pub fn is_ready(&self, lang: &str) -> bool {
        self.clients.get(lang).map(|c| c.ready).unwrap_or(false)
    }

    /// Marca ready y devuelve el handle para mandar didOpen pendientes.
    pub fn mark_ready(&mut self, lang: &'static str) -> bool {
        let changed = self
            .clients
            .get_mut(lang)
            .map(|c| {
                let was = c.ready;
                c.ready = true;
                !was
            })
            .unwrap_or(false);
        changed
    }

    pub fn remove(&mut self, lang: &'static str) {
        self.clients.remove(lang);
    }

    pub fn clients(&self) -> impl Iterator<Item = (&'static str, &ClientHandle)> {
        self.clients.iter().map(|(k, v)| (*k, v))
    }

    pub fn request(&mut self, lang: &'static str, method: &str, params: serde_json::Value) -> Option<u64> {
        let Some(c) = self.clients.get(lang) else { return None };
        let req_id = self.next_req;
        self.next_req += 1;
        let _ = c.tx.send(client::Cmd::Request { req_id, method: method.into(), params });
        Some(req_id)
    }

    pub fn notify(&self, lang: &'static str, method: &str, params: serde_json::Value) {
        if let Some(c) = self.clients.get(lang) {
            let _ = c.tx.send(client::Cmd::Notify { method: method.into(), params });
        }
    }

    /// Drena los eventos pendientes (copia, sin pedir &mut del caller).
    pub fn drain(&self) -> Vec<LspEvent> {
        let mut out = Vec::new();
        loop {
            match self.rx.try_recv() {
                Ok(ev) => out.push(ev),
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => return out,
            }
        }
    }

    /// Shutdown best-effort de todos los clientes.
    pub fn shutdown(&self) {
        for (_, c) in self.clients.iter() {
            let _ = c.tx.send(client::Cmd::Shutdown);
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
    }

    pub fn language_of(&self, path: &PathBuf) -> Option<&'static str> {
        let ext = path.extension()?.to_str()?; // rs -> rust, java -> java
        match ext {
            "rs" => Some("rust"),
            "java" => Some("java"),
            _ => None,
        }
    }
}
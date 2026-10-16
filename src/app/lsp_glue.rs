//! Glue LSP: ciclo documental, requests y debounce/didChange.

use std::path::PathBuf;
use std::time::Instant;

use crate::app::{same_path, App, CompletionPopup, Diag, HoverOrDef};
use crate::lsp::{self, LspEvent};

impl App {
    pub(super) fn ldbg(&self, m: &str) {
        if std::env::var("MENTL_LSP_DEBUG").is_ok() {
            eprintln!("[app] {m}");
        }
    }

    pub(super) fn lsp_sync_tick(&mut self) {
        if !self.config.lsp.enabled {
            return;
        }
        let debounce = std::time::Duration::from_millis(self.config.lsp.debounce_ms);
        let mut to_send: Vec<(&'static str, PathBuf, String)> = Vec::new();
        for b in self.buffers.iter().flatten() {
            let Some(path) = b.path.clone() else { continue };
            let Some(lang) = self.lsp.language_of(&path) else { continue };
            if !self.lsp.is_ready(lang) {
                continue;
            }
            let state = self.sync_state.entry(path.clone())
                .or_insert((u64::MAX, None));
            let (sent_rev, pending_since) = *state;
            if b.rev != sent_rev {
                let since = pending_since.unwrap_or(Instant::now());
                state.1 = Some(since);
                if since.elapsed() >= debounce {
                    let text = b
                        .rope
                        .lines()
                        .map(|l| l.to_string())
                        .collect::<Vec<_>>()
                        .join("");
                    to_send.push((lang, path.clone(), text));
                }
            }
        }
        for (lang, path, text) in to_send {
            let uri = lsp::types::path_to_uri(&path);
            let params = serde_json::json!({
                "textDocument": {
                    "uri": uri,
                    "version": 0,
                    "languageId": lang,
                    "text": text,
                }
            });
            // marca rev como mandado ANTES de que lleguen diagnósticos nuevos
            let rev = self
                .buffers
                .iter()
                .flatten()
                .find(|b| b.path.as_ref() == Some(&path))
                .map(|b| b.rev)
                .unwrap_or(0);
            self.sync_state.insert(path, (rev, None));
            self.lsp.notify(lang, "textDocument/didChange", params);
        }
    }

    pub fn lsp_did_open(&mut self, path: &PathBuf) {
        if !self.config.lsp.enabled {
            return;
        }
        let Some(lang) = self.lsp.language_of(path) else { return };
        if !self.lsp.is_ready(lang) {
            return;
        }
        let Some(text_b) = self
            .buffers
            .iter()
            .flatten()
            .find(|b| b.path.as_ref() == Some(path))
        else {
            return;
        };
        let text = text_b.rope.lines().map(|l| l.to_string()).collect::<Vec<_>>().join("");
        let rev = text_b.rev;
        let uri = lsp::types::path_to_uri(path);
        let params = serde_json::json!({
            "textDocument": {"uri": uri, "version": 0, "languageId": lang, "text": text}
        });
        self.sync_state.insert(path.clone(), (rev, None));
        self.lsp.notify(lang, "textDocument/didOpen", params);
    }

    pub fn lsp_did_save(&mut self) {
        if !self.config.lsp.enabled {
            return;
        }
        let Some(path) = self.buffer().path.clone() else { return };
        let Some(lang) = self.lsp.language_of(&path) else { return };
        if !self.lsp.is_ready(lang) {
            return;
        }
        let text = self
            .buffer()
            .rope
            .lines()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("");
        self.lsp.notify(
            lang,
            "textDocument/didSave",
            serde_json::json!({
                "textDocument": {"uri": lsp::types::path_to_uri(&path)},
                "text": text,
            }),
        );
    }

    pub fn lsp_shutdown(&mut self) {
        self.lsp.shutdown();
    }

    pub fn on_lsp(&mut self) -> bool {
        let events = self.lsp.drain();
        let mut need = false;
        for ev in events {
            match ev {
                LspEvent::Started { lang } => {
                    if self.lsp.mark_ready(lang) {
                        // didOpen de buffers ya abiertos de este lenguaje
                        let paths: Vec<PathBuf> = self
                            .buffers
                            .iter()
                            .flatten()
                            .filter(|b| {
                                b.path.as_ref()
                                    .is_some_and(|p| self.lsp.language_of(p) == Some(lang))
                            })
                            .map(|b| b.path.clone().unwrap())
                            .collect();
                        for p in paths {
                            self.lsp_did_open(&p);
                        }
                    }
                }
                LspEvent::StartedFailed { lang, err } => {
                    self.status_msg(format!("lsp:{lang} no arranca — {err}"));
                    self.lsp.remove(lang);
                }
                LspEvent::Died { lang, err } => {
                    self.status_msg(format!("lsp:{lang} caído — {err}"));
                    self.lsp.remove(lang);
                    need = true;
                }
                LspEvent::Diagnostics { uri, diags } => {
                    if let Some(path) = lsp::types::uri_to_path(&uri) {
                        let converted: Vec<Diag> = diags
                            .into_iter()
                            .map(|d| Diag {
                                line: d.line as usize,
                                col_u16: d.character,
                                severity: d.severity,
                                msg: d.msg,
                            })
                            .collect();
                        self.diagnostics.insert(path, converted);
                        need = true;
                    }
                }
                LspEvent::Response { req_id, body } => {
                    // completion en vuelo?
                    let comp_hit = self
                        .completion_wait
                        .as_ref()
                        .is_some_and(|(r, _, _, _)| *r != 0 && *r == req_id);
                    if comp_hit {
                        self.ldbg("completion response!");
                        let (_, _, rev, _) = self.completion_wait.take().unwrap();
                        self.completion_wait = None;
                        if self.buffer().rev == rev && !self.buffer().ai {
                            let items = lsp::types::parse_completion(&body);
                            if !items.is_empty() {
                                self.completion = Some(CompletionPopup { items, filtered: vec![], cursor: 0 });
                                self.completion_refilter();
                                need = true;
                            }
                        }
                    }
                    else {
                    // hover pendiente con este id?
                    let hover_hit = self.hover_wait.as_ref().is_some_and(|(r, _, _)| *r == req_id);
                    if hover_hit {
                        self.hover_wait = None;
                        self.hover = lsp::types::parse_hover(&body);
                        need = self.hover.is_some();
                    } else {
                        let def_hit = self.def_wait.as_ref().is_some_and(|(r, _, _)| *r == req_id);
                        if def_hit {
                            let (_, origin, _) = self.def_wait.take().unwrap();
                            let locs = lsp::types::parse_definition(&body);
                            self.goto_locations(locs, origin);
                            need = true;
                        }
                    }
                    }
                }
            }
        }
        need
    }

    pub fn goto_locations(&mut self, locs: Vec<lsp::types::DefLocation>, _from: (usize, usize)) {
        let Some(first) = locs.first() else {
            self.status_msg("sin definición");
            return;
        };
        if locs.len() > 1 {
            self.status_msg(format!("{} definiciones — voy a la primera", locs.len()));
        }
        let Some(path) = lsp::types::uri_to_path(&first.uri) else {
            self.status_msg("def: uri no file");
            return;
        };
        let same = same_path(self.buffer().path.as_ref(), Some(&path));
        if !same {
            if let Err(e) = self.open_in_pane(&path) {
                self.status_msg(format!("no puedo abrir: {e}"));
                return;
            }
        }
        let (l, g) = self.buffer().pos_of_lsp(first.line, first.character);
        self.buffer().to(l, g);
        self.follow_cursor();
    }

    pub fn lsp_request_at_cursor(&mut self, kind: HoverOrDef) {
        let path = match self.buffer().path.clone() {
            Some(p) => p,
            None => {
                self.status_msg("buffer sin archivo");
                return;
            }
        };
        let Some(lang) = self.lsp.language_of(&path) else {
            self.status_msg("sin lsp para este tipo de archivo");
            return;
        };
        if !self.lsp.is_ready(lang) {
            self.status_msg(format!("lsp:{lang} aún arrancando…"));
            return;
        }
        let (l, g) = self.buffer().cursor;
        let (ll, col) = self.buffer().lsp_pos_of(l, g);
        let params = serde_json::json!({
            "textDocument": {"uri": lsp::types::path_to_uri(&path)},
            "position": {"line": ll, "character": col},
        });
        let method = match kind {
            HoverOrDef::Hover => "textDocument/hover",
            HoverOrDef::Def => "textDocument/definition",
        };
        let Some(req_id) = self.lsp.request(lang, method, params) else { return };
        match kind {
            HoverOrDef::Hover => {
                self.hover = None;
                self.hover_wait = Some((req_id, (l, g), Instant::now()));
            }
            HoverOrDef::Def => {
                self.def_wait = Some((req_id, (l, g), Instant::now()));
            }
        }
    }

}

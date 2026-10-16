//! Estado del app + dispatch. La capa bazel/LSP futura conecta aquí: nueva
//! Action y nuevo match arm — nada del modelo de texto cambia.

use std::collections::HashMap;
use std::path::PathBuf;
use unicode_segmentation::UnicodeSegmentation;
use std::time::Instant;

use crate::action::Action;
use crate::buffer::Buffer;
use crate::config::{self, Config};
use crate::finder::Finder;
use crate::input::InputKey;
use crate::keymap::{Mode, KeyMap, Match};
use crate::lsp::{self, Diag, LspEvent};
use crate::panes::{Dir, Focus, Panes};
use crate::theme::{CursorStyle, Theme};
use ratatui::layout::Rect;
use crate::view::Geometry;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Welcome,
    Picker,
    Editor,
}

pub struct App {
    pub screen: Screen,
    pub mode: Mode,
    pub buffers: Vec<Buffer>,
    pub panes: Panes,
    pub finder: Option<Finder>,
    /// rect del área completa de editor (lo pone el draw)
    pub last_area: ratatui::layout::Rect,
    /// rect del área EDITOR (sidebar/tabs/panel fuera) para panes/focus/popups
    pub last_editor_rect: ratatui::layout::Rect,
    pub cursor_style: CursorStyle,
    pub cursor_style_dirty: bool,
    pub config: Config,
    pub theme: Theme,
    pub keymap: KeyMap,
    pub pending: Vec<InputKey>,
    pub last_key: Option<Instant>,
    /// búsqueda activa
    pub search: Option<String>,
    pub search_prompt: bool,
    pub search_input: String,
    pub msg: Option<(String, Instant)>,
    pub quit_pending: bool,
    pub quit: bool,
    pub picker: Vec<std::path::PathBuf>,
    pub picker_cursor: usize,
    pub picker_scroll: usize,
    pub config_error: Option<String>,
    /// altura del viewport de editor conocida en el último draw (24 default)
    pub last_view_h: usize,
    pub lsp: lsp::LspHub,
    pub diagnostics: HashMap<std::path::PathBuf, Vec<Diag>>,
    /// hover en espera: req_id, cursor al pedir, instante
    pub hover_wait: Option<(u64, (usize, usize), Instant)>,
    pub hover: Option<String>,
    /// pending goto-def: req_id, cursor de origen, instante
    pub def_wait: Option<(u64, (usize, usize), Instant)>,
    /// didChange debounce: path -> (último rev mandado, primera ed sin mandar)
    pub sync_state: HashMap<std::path::PathBuf, (u64, Option<Instant>)>,
    // ------------------------------------------------------------ fase 3 ---
    pub sidebar: Option<crate::explorer::Sidebar>,
    pub sidebar_focus: bool,
    pub project_root: PathBuf,
    pub problems_open: bool,
    pub problems_focus: bool,
    pub problems_cursor: usize,
    pub problems_scroll: usize,
    /// popup de completion: items, filtrados(idx), cursor, prefijo len
    pub completion: Option<CompletionPopup>,
    pub completion_wait: Option<(u64, PathBuf, u64, Instant)>,
    pub ai_ctx: Option<String>,
    pub ai_jobs: Vec<AiJob>,
}

pub struct CompletionPopup {
    pub items: Vec<lsp_types::CompletionItem>,
    pub filtered: Vec<usize>,
    pub cursor: usize,
}

/// Fila del problems panel.
pub struct ProblemRow {
    pub path: std::path::PathBuf,
    pub line_u16: u32,
    pub col_u16: u32,
    pub line_display: usize,
    pub severity: u32,
    pub msg: String,
}

mod lsp_types {
    pub use crate::lsp::types::CompletionItem;
}

pub struct AiJob {
    pub rx: std::sync::mpsc::Receiver<std::io::Result<String>>,
    pub started: Instant,
    pub marker: (usize, usize), // línea, grapheme del "… thinking …"
}

#[derive(Clone, Copy)]
pub enum WaitKind {
    Hover,
    Def,
    Completion,
}

impl App {
    pub fn new(arg: Option<std::path::PathBuf>) -> Self {
        let arg_ref = arg.clone();
        let arg_dir = arg_ref.as_ref().filter(|p| p.is_dir()).cloned();
        let screen;
        let mut buffers = Vec::new();
        let mut picker = Vec::new();
        let mut msg = None;
        match arg {
            None => screen = Screen::Welcome,
            Some(p) if p.is_dir() => {
                let mut entries: Vec<_> = std::fs::read_dir(&p)
                    .map(|rd| {
                        rd.filter_map(|e| e.ok())
                            .map(|e| e.path())
                            .collect()
                    })
                    .unwrap_or_default();
                entries.sort();
                picker = entries;
                screen = Screen::Picker;
            }
            Some(p) => match Buffer::open(&p) {
                Ok(b) => {
                    buffers.push(b);
                    screen = Screen::Editor;
                }
                Err(e) => {
                    msg = Some((e, Instant::now()));
                    screen = Screen::Welcome;
                }
            },
        }
        let (config, config_error) = match config::load() {
            Ok(c) => (c, None),
            Err(e) => (Config::default(), Some(e)),
        };
        let theme = Theme::build(&config.theme);
        let (keymap, warnings) = KeyMap::build(&config.keys);
        let msg = match (&msg, warnings.is_empty() && config_error.is_none()) {
            (Some((m, t)), _) => Some((m.clone(), *t)),
            (None, false) => Some((config_error.clone().unwrap_or_else(|| {
                warnings.join(" · ")
            }), Instant::now())),
            (None, true) => None,
        };
        if buffers.is_empty() {
            buffers.push(Buffer::empty(None));
        }
        let mut app = Self {
            screen,
            mode: Mode::Normal,
            buffers,
            panes: Panes::first(0),
            finder: None,
            project_root: arg_dir
                .clone()
                .or_else(|| {
                    arg_ref.as_ref().filter(|p| p.is_file()).and_then(|p| p.parent().map(|d| d.to_path_buf()))
                })
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default()),
            last_area: Rect::new(0, 0, 80, 24),
            last_editor_rect: Rect::new(0, 0, 80, 24),
            cursor_style: CursorStyle::parse(&config.editor.cursor_normal),
            cursor_style_dirty: true,
            config,
            theme,
            keymap,
            pending: Vec::new(),
            last_key: None,
            search: None,
            search_prompt: false,
            search_input: String::new(),
            msg,
            quit_pending: false,
            quit: false,
            picker,
            picker_cursor: 0,
            picker_scroll: 0,
            config_error,
            last_view_h: 24,
            lsp: lsp::LspHub::default(),
            diagnostics: HashMap::new(),
            hover_wait: None,
            hover: None,
            def_wait: None,
            sync_state: HashMap::new(),
            sidebar: None,
            sidebar_focus: false,
            problems_open: false,
            problems_focus: false,
            problems_cursor: 0,
            problems_scroll: 0,
            completion: None,
            completion_wait: None,
            ai_ctx: None,
            ai_jobs: Vec::new(),
        };
        // arranca client si el buffer inicial es de un lenguaje LSP
        if app.screen == Screen::Editor {
            let p = app.buffer().path.clone();
            if let Some(p) = p {
                if let Some(lang) = app.lsp.language_of(&p) {
                    app.lsp.ensure_client(lang, &app.config.lsp, app.config.lsp_server(lang));
                    app.lsp_did_open(&p);
                }
            }
        }
        app
    }

    /// Índice del buffer del pane activo.
    pub fn active_buffer(&self) -> usize {
        self.panes.buffer_of(self.panes.active)
    }

    pub fn buffer(&mut self) -> &mut Buffer {
        let idx = self.active_buffer();
        &mut self.buffers[idx]
    }

    /// Push o retarget: si el path ya está abierto, el pane activo apunta a
    /// ese buffer (tab switch, sin didClose). Si no, push y retarget.
    pub fn open_in_pane(&mut self, path: &std::path::Path) -> Result<(), String> {
        let b = Buffer::open(path)?;
        let new_path = b.path.clone();
        // ya abierto en otro buffer? (comparación por slot distinto al activo)
        if let Some(existing) = self.buffers.iter().position(|bb| {
            !std::ptr::eq(bb, &self.buffers[self.active_buffer()])
                && same_path(bb.path.as_ref(), Some(&path.to_path_buf()))
        }) {
            self.panes.panes[self.panes.active].buffer = existing;
            self.follow_cursor();
            return Ok(());
        }
        // el buffer del pane activo se queda vivo (es una tab)
        let idx = self.buffers.len();
        self.buffers.push(b);
        self.panes.panes[self.panes.active].buffer = idx;
        if let Some(p) = new_path {
            let lang = self.lsp.language_of(&p);
            if let Some(lang) = lang {
                self.lsp.ensure_client(lang, &self.config.lsp, self.config.lsp_server(lang));
                self.lsp_did_open(&p);
            }
        }
        Ok(())
    }

    /// Cierra el buffer del pane activo como tab: retarget de panes, renumber,
    /// didClose solo si nadie lo muestra, limpieza de diagnósticos/sync.
    pub fn close_buffer(&mut self) {
        let idx = self.active_buffer();
        let path = self.buffers[idx].path.clone();
        // vecino para el foco: prev, sino next
        let next = if idx > 0 { idx - 1 } else { idx + 1 };
        // retarget de TODOS los panes que apunten a idx
        for pane in self.panes.panes.iter_mut() {
            if pane.buffer == idx {
                if self.buffers.len() > 1 {
                    pane.buffer = next.min(self.buffers.len() - 1);
                    if pane.buffer >= idx {
                        pane.buffer -= 1;
                    }
                }
                // único buffer: se sustituye por uno vacío (sin renumber)
            }
        }
        if self.buffers.len() == 1 {
            self.buffers[0] = Buffer::empty(None);
        } else {
            self.buffers.remove(idx);
            // renumber: todo buffer index > idx baja uno
            for pane in self.panes.panes.iter_mut() {
                if pane.buffer > idx {
                    pane.buffer -= 1;
                }
            }
        }
        // didClose solo si el path no vive en otro buffer
        if let Some(p) = path {
            let still_open = self.buffers.iter().any(|bb| same_path(bb.path.as_ref(), Some(&p)));
            if !still_open {
                if let Some(lang) = self.lsp.language_of(&p) {
                    let uri = lsp::types::path_to_uri(&p);
                    self.lsp.notify(lang, "textDocument/didClose", serde_json::json!({
                        "textDocument": {"uri": uri}
                    }));
                    self.diagnostics.remove(&p);
                    self.sync_state.remove(&p);
                }
            }
        }
        self.follow_cursor();
    }

    /// Tab siguiente/anterior del buffer en el pane activo (wrap-around).
    pub fn cycle_tab(&mut self, forward: bool) {
        if self.buffers.len() <= 1 {
            return;
        }
        let cur = self.active_buffer();
        let len = self.buffers.len();
        let target = match forward {
            true => (cur + 1) % len,
            false => (cur + len - 1) % len,
        };
        self.panes.panes[self.panes.active].buffer = target;
        self.follow_cursor();
    }

    pub fn status_msg(&mut self, s: impl Into<String>) {
        self.msg = Some((s.into(), Instant::now()));
    }

    /// Tick del loop principal: timeout de chord + decay de mensaje + debounce
    /// de didChange LSP. Devuelve true si algo cambió visualmente.
    pub fn tick(&mut self) -> bool {
        let mut changed = false;
        if !self.pending.is_empty() {
            let to = std::time::Duration::from_millis(self.config.editor.chord_timeout_ms);
            if self.last_key.is_some_and(|t| t.elapsed() > to) {
                self.pending.clear();
                changed = true;
            }
        }
        if let Some((_, t)) = &self.msg {
            if t.elapsed() >= std::time::Duration::from_secs(5) {
                self.msg = None;
                changed = true;
            }
        }
        self.lsp_sync_tick();
        self.completion_tick();
        changed |= self.ai_tick();
        changed
    }

    /// didChange LSP con sync completo, debounce desde la PRIMERA edición sin
    /// mandar. Mandar NO se dibuja — solo la llegada de diagnósticos.
    fn lsp_sync_tick(&mut self) {
        if !self.config.lsp.enabled {
            return;
        }
        let debounce = std::time::Duration::from_millis(self.config.lsp.debounce_ms);
        let mut to_send: Vec<(&'static str, PathBuf, String)> = Vec::new();
        for b in &self.buffers {
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
            let rev = self.buffers.iter().find(|b| b.path.as_ref() == Some(&path))
                .map(|b| b.rev)
                .unwrap_or(0);
            self.sync_state.insert(path, (rev, None));
            self.lsp.notify(lang, "textDocument/didChange", params);
        }
    }

    /// didOpen — solo si el server ya está ready (si no, lo manda on Started).
    pub fn lsp_did_open(&mut self, path: &PathBuf) {
        if !self.config.lsp.enabled {
            return;
        }
        let Some(lang) = self.lsp.language_of(path) else { return };
        if !self.lsp.is_ready(lang) {
            return;
        }
        let Some(text_b) = self.buffers.iter().find(|b| b.path.as_ref() == Some(path)) else { return };
        let text = text_b.rope.lines().map(|l| l.to_string()).collect::<Vec<_>>().join("");
        let rev = text_b.rev;
        let uri = lsp::types::path_to_uri(path);
        let params = serde_json::json!({
            "textDocument": {"uri": uri, "version": 0, "languageId": lang, "text": text}
        });
        self.sync_state.insert(path.clone(), (rev, None));
        self.lsp.notify(lang, "textDocument/didOpen", params);
    }

    /// Eventos LSP desde el hub. true = redibujar.
    fn ldbg(&self, m: &str) {
        if std::env::var("MENTL_LSP_DEBUG").is_ok() {
            eprintln!("[app] {m}");
        }
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
                        // UNA búsqueda del buffer por publish (no una por diag)
                        let buf_slot: Option<usize> = self
                            .buffers
                            .iter()
                            .position(|b| same_path(b.path.as_ref(), Some(&path)));
                        let converted: Vec<Diag> = diags
                            .into_iter()
                            .map(|d| {
                                let (line, col) = match buf_slot {
                                    Some(i) => self.buffers[i].pos_of_lsp(d.line, d.character),
                                    None => (d.line as usize, d.character as usize),
                                };
                                Diag { line, col, col_u16: d.character, severity: d.severity, msg: d.msg }
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

    /// Recarga config TOML; roto = mantiene todo como estaba + msg.
    pub fn reload_config(&mut self) {
        match config::load() {
            Ok(c) => {
                self.config = c;
                self.theme = Theme::build(&self.config.theme);
                let (km, warnings) = KeyMap::build(&self.config.keys);
                self.keymap = km;
                self.config_error = if warnings.is_empty() {
                    None
                } else {
                    Some(warnings.join(" · "))
                };
                self.status_msg("config recargada ✓");
            }
            Err(e) => {
                self.config_error = Some(e.clone());
                self.status_msg(format!("config rota, mantengo la anterior: {e}"));
            }
        }
    }

    /// Mensaje vigente si no ha decayido (5s), sin mutar.
    pub fn live_msg(&self) -> Option<String> {
        match &self.msg {
            Some((m, t)) if t.elapsed() < std::time::Duration::from_secs(5) => Some(m.clone()),
            _ => None,
        }
    }

    fn geo(&self) -> Geometry {
        Geometry { height: self.last_view_h, scrolloff: self.config.editor.scrolloff }
    }

    pub fn follow_cursor(&mut self) {
        let (l, _) = self.buffer().cursor;
        let geo = self.geo();
        self.panes.panes[self.panes.active].view.ensure_visible(l, &geo);
    }

    /// Rects por pane (área de editor: sidebar/tabs/panel fuera).
    pub fn pane_rects(&self) -> Vec<(usize, Rect)> {
        let mut out = Vec::new();
        let mut seps = Vec::new();
        self.panes.layout(self.last_editor_rect, &mut out, &mut seps);
        out
    }

    // --------------------------------------------------------------- keys ---

    pub fn on_key(&mut self, k: InputKey) {
        // hover se cierra con cualquier tecla (y esa tecla sigue su curso)
        if self.hover.is_some() {
            self.hover = None;
        }
        // hover/def timeouts (stale)
        for slot in [&mut self.hover_wait, &mut self.def_wait] {
            if let Some((_, _, until)) = slot {
                if until.elapsed() > std::time::Duration::from_secs(3) {
                    *slot = None;
                }
            }
        }
        if self.screen == Screen::Welcome {
            match k.code {
                crossterm::event::KeyCode::Char('q') | crossterm::event::KeyCode::Esc => {
                    self.quit = true;
                }
                _ => {}
            }
            return;
        }
        if self.screen == Screen::Picker {
            self.picker_key(k);
            return;
        }
        if self.sidebar_focus {
            self.sidebar_key(k);
            return;
        }
        if self.problems_focus {
            self.problems_key(k);
            return;
        }
        // popup de completion: intercepta ANTES del keymap (no crea chords);
        // Esc cierra SOLO el popup, no sale de insert (difiere del bind Esc).
        if self.mode == Mode::Insert && self.completion.is_some() {
            use crossterm::event::KeyCode as K;
            match k.code {
                K::Up => return self.completion_move(-1),
                K::Down => return self.completion_move(1),
                K::Enter => return self.completion_accept(None),
                K::Tab => return self.completion_accept(Some(0)),
                K::Esc => return self.completion_close(),
                _ => {}
            }
        }
        if self.search_prompt {
            self.search_key(k);
            return;
        }
        if self.finder.is_some() {
            self.finder_key(k);
            return;
        }
        // chord timeout
        if let Some(t) = self.last_key {
            let to = std::time::Duration::from_millis(self.config.editor.chord_timeout_ms);
            if !self.pending.is_empty() && t.elapsed() > to {
                self.pending.clear();
            }
        }
        self.last_key = Some(Instant::now());
        self.pending.push(k);
        let seq = std::mem::take(&mut self.pending);
        match self.keymap.match_seq(self.mode, &seq) {
            Match::Action(a) => self.do_action(a),
            Match::Pending => {
                self.pending = seq; // espera el tail del chord
            }
            Match::None => {
                if self.mode == Mode::Insert && seq.len() == 1 {
                    self.insert_key(seq[0]);
                }
                // normal: tecla no mapeada se ignora (silencio vim-like)
            }
        }
        self.follow_cursor();
    }

    fn picker_key(&mut self, k: InputKey) {
        use crossterm::event::KeyCode as K;
        match k.code {
            K::Char('j') | K::Down => {
                self.picker_cursor = (self.picker_cursor + 1).min(self.picker.len().saturating_sub(1));
            }
            K::Char('k') | K::Up => {
                self.picker_cursor = self.picker_cursor.saturating_sub(1);
            }
            K::Enter => {
                if let Some(p) = self.picker.get(self.picker_cursor) {
                    let p = p.clone();
                    match self.open_in_pane(&p) {
                        Ok(()) => self.screen = Screen::Editor,
                        Err(e) => self.status_msg(e),
                    }
                }
            }
            K::Char('q') | K::Esc => self.quit = true,
            _ => {}
        }
    }

    fn finder_key(&mut self, k: InputKey) {
        let pick = self.finder.as_mut().map(|f| f.key(k)).unwrap_or(None);
        if let Some(p) = pick {
            if let Err(e) = self.open_in_pane(&p) {
                self.status_msg(e);
            }
            self.finder = None;
        } else if k.code == crossterm::event::KeyCode::Esc {
            self.finder = None;
        }
    }

    // -------------------------------------------------------------- fase 3 --

    fn sidebar_key(&mut self, k: InputKey) {
        use crossterm::event::KeyCode as K;
        let Some(sb) = &mut self.sidebar else { return self.close_sidebar() };
        match k.code {
            K::Char('j') | K::Down => sb.move_cursor(1),
            K::Char('k') | K::Up => sb.move_cursor(-1),
            K::Char('l') | K::Right => {
                if sb.rows.get(sb.cursor).is_some_and(|r| r.is_dir) {
                    sb.toggle_row();
                } else {
                    self.sidebar_open_file();
                    return;
                }
            }
            K::Char('h') | K::Left => sb.collapse_or_parent(),
            K::Enter => {
                if sb.rows.get(sb.cursor).is_some_and(|r| r.is_dir) {
                    sb.toggle_row();
                } else {
                    self.sidebar_open_file();
                    return;
                }
            }
            _ => {}
        }
    }

    fn sidebar_open_file(&mut self) {
        let path = self
            .sidebar
            .as_ref()
            .and_then(|s| s.rows.get(s.cursor))
            .filter(|r| !r.is_dir)
            .map(|r| r.path.clone());
        if let Some(p) = path {
            if let Err(e) = self.open_in_pane(&p) {
                self.status_msg(e);
            }
        }
        self.close_sidebar();
    }

    fn close_sidebar(&mut self) {
        self.sidebar_focus = false;
    }

    fn problems_key(&mut self, k: InputKey) {
        use crossterm::event::KeyCode as K;
        let count = self.problem_rows().len();
        if count == 0 {
            return;
        }
        self.problems_cursor = self.problems_cursor.min(count - 1);
        match k.code {
            K::Char('j') | K::Down => self.problems_cursor += 1,
            K::Char('k') | K::Up => self.problems_cursor = self.problems_cursor.saturating_sub(1),
            K::Enter => {
                let rows = self.problem_rows();
                if let Some(p) = rows.get(self.problems_cursor) {
                    let (path, line_u16, col_u16) = (p.path.clone(), p.line_u16, p.col_u16);
                    if let Err(e) = self.open_in_pane(&path) {
                        self.status_msg(e);
                        return;
                    }
                    let (l, g) = self.buffer().pos_of_lsp(line_u16, col_u16);
                    self.buffer().to(l, g);
                }
                self.problems_focus = false;
            }
            K::Esc => self.problems_focus = false,
            _ => {}
        }
        let count = self.problem_rows().len();
        self.problems_cursor = self.problems_cursor.min(count.saturating_sub(1));
    }

    /// Filas del problems panel: diagnósticos por (path, línea).
    pub fn problem_rows(&self) -> Vec<ProblemRow> {
        let mut out: Vec<ProblemRow> = Vec::new();
        for (path, diags) in &self.diagnostics {
            let path = path.clone();
            for d in diags {
                out.push(ProblemRow {
                    path: path.clone(),
                    line_u16: d.line as u32,
                    col_u16: d.col_u16,
                    line_display: d.line,
                    severity: d.severity,
                    msg: d.msg.clone(),
                });
            }
        }
        out.sort_by(|a, b| {
            a.path.cmp(&b.path)
                .then(a.line_display.cmp(&b.line_display))
                .then(a.col_u16.cmp(&b.col_u16))
        });
        out
    }

    // ------------------------------------------------------------ completión --

    fn completion_move(&mut self, d: isize) {
        let Some(p) = self.completion.as_mut() else { return };
        let n = p.filtered.len();
        if n == 0 {
            return;
        }
        let c = p.cursor as isize + d;
        p.cursor = c.clamp(0, n as isize - 1) as usize;
    }

    fn completion_accept(&mut self, which: Option<usize>) {
        let Some(p) = self.completion.as_ref() else { return };
        let idx = which
            .unwrap_or(p.cursor)
            .min(p.filtered.len().saturating_sub(1));
        let Some(&item_idx) = p.filtered.get(idx) else { return };
        let insert = p.items[item_idx].insert.clone();
        self.completion = None;
        self.buffer().insert_text(&insert, "insert");
        self.follow_cursor();
        let _ = which;
    }

    fn completion_close(&mut self) {
        self.completion = None;
        self.completion_wait = None;
    }

    /// Refiltra el popup: prefijo = run de word-chars hacia atrás del cursor.
    fn completion_refilter(&mut self) {
        let (prefix, cur_g) = {
            let b = self.buffer();
            let text = b.line_text(b.cursor.0);
            let (_, g) = b.cursor;
            let start = (0..g)
                .rev()
                .find(|&i| {
                    text.graphemes(true)
                        .nth(i)
                        .and_then(|x| x.chars().next())
                        .map(crate::buffer::is_word)
                        .unwrap_or(false)
                })
                .map_or(0usize, |i| i + 1);
            let word_b = Buffer::grapheme_byte_of(&text, start);
            let cut = Buffer::grapheme_byte_of(&text, g);
            (text.get(word_b..cut).unwrap_or("").to_string(), g)
        };
        let Some(p) = self.completion.as_mut() else { return };
        let _ = cur_g;
        p.filtered = p
            .items
            .iter()
            .enumerate()
            .filter(|(_, it)| it.label.to_lowercase().contains(&prefix.to_lowercase()))
            .map(|(i, _)| i)
            .collect();
        p.cursor = 0;
        if p.filtered.is_empty() {
            self.completion_close();
        }
    }

    fn trigger_completion(&mut self) {
        self.ldbg("trigger");
        if self.completion_wait.is_some() {
            return;
        }
        let Some(path) = self.buffer().path.clone() else { return };
        let Some(lang) = self.lsp.language_of(&path) else { return };
        if !self.lsp.is_ready(lang) {
            return;
        }
        let rev = self.buffer().rev;
        self.completion_wait = Some((0, path, rev, Instant::now())); // 0 = sin mandar
    }

    /// 0 = pendiente de enviar: cuando pasa el debounce y el estado sigue
    /// vivo, manda la request. Con req real: nada (la respuesta llega en
    /// on_lsp y ahí se consuma).
    fn completion_tick(&mut self) {
        if std::env::var("MENTL_LSP_DEBUG").is_ok() && self.completion_wait.is_some() { eprintln!("[app] comp tick"); }
        let Some((req, path, rev, since)) = self.completion_wait.clone() else { return };
        if self.mode != Mode::Insert {
            self.completion_close();
            return;
        }
        if req != 0 {
            return;
        }
        let debounce = std::time::Duration::from_millis(self.config.ui.completion_debounce_ms);
        if since.elapsed() < debounce {
            return;
        }
        if !same_path(Some(&path), self.buffer().path.as_ref()) || self.buffer().rev != rev {
            self.completion_close();
            return;
        }
        let Some(lang) = self.lsp.language_of(&path) else {
            self.completion_close();
            return;
        };
        let (l, g) = self.buffer().cursor;
        let (ll, col) = self.buffer().lsp_pos_of(l, g);
        let params = serde_json::json!({
            "textDocument": {"uri": lsp::types::path_to_uri(&path)},
            "position": {"line": ll, "character": col},
        });
        match self.lsp.request(lang, "textDocument/completion", params) {
            Some(wire) => self.completion_wait = Some((wire, path, rev, since)),
            None => self.completion_wait = None,
        }
    }

    // ------------------------------------------------------------------ AI --

    fn ai_toggle(&mut self) {
        // el pane activo ya muestra AI? -> ciérralo
        if self.buffer().ai {
            let _ = self.panes.close();
            self.follow_cursor();
            return;
        }
        self.panes.split(Dir::Horiz);
        let idx = self.buffers.len();
        self.buffers.push(Buffer::ai());
        self.panes.panes[self.panes.active].buffer = idx;
        self.mode = Mode::Insert;
        self.cursor_style_dirty = true;
    }

    fn ai_context(&mut self) {
        let path = self.buffer().path.clone();
        let Some(p) = path else {
            self.status_msg("buffer sin archivo para contexto");
            return;
        };
        let max = self.config.ai.max_context_lines;
        let content = self
            .buffer()
            .rope
            .lines()
            .take(max)
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("");
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.ai_ctx = Some(format!("Archivo: {}\n```\n{content}\n```", p.display()));
        self.status_msg(format!("ctx: {name} (máx {max} líneas)"));
    }

    /// Enter en insert-mode sobre un pane AI = manda el prompt (línea actual).
    fn ai_submit(&mut self) {
        let (line, _) = self.buffer().cursor;
        let prompt = self.buffer().line_text(line).trim().to_string();
        if prompt.is_empty() || prompt.starts_with("▸") || prompt.starts_with("…") {
            return;
        }
        if let Some(job) = self.ai_jobs.last() {
            let _ = job; // hay un job en vuelo — espera respuesta
            return;
        }
        let ctx = self.ai_ctx.take();
        let final_prompt = match ctx {
            Some(c) => format!("{c}\n\n{prompt}"),
            None => prompt.clone(),
        };
        let cmd = self.config.ai.command.clone();
        let timeout = std::time::Duration::from_secs(self.config.ai.timeout_secs);
        let (tx, rx) = std::sync::mpsc::channel();
        let _ = std::thread::Builder::new()
            .name("ai-job".into())
            .spawn(move || ai_run(cmd, final_prompt, timeout, tx));
        // transcript: bloque de prompt + marcador pendiente en el EOF
        self.buffer().file_end();
        self.buffer().insert_text(&format!("\n▸ {prompt}\n"), "insert");
        self.buffer().file_end();
        self.buffer().insert_text("\n… thinking …", "insert");
        self.buffer().file_end();
        let marker = self.buffer().cursor;
        self.ai_jobs.push(AiJob { rx, started: Instant::now(), marker });
    }

    fn replace_marker_line(&mut self, line: usize, text: &str) {
        let b = self.buffer();
        let len_lines = b.len_lines();
        let start = b.rope.line_to_char(line.min(len_lines.saturating_sub(1)));
        let end = b
            .rope
            .line_to_char((line + 1).min(len_lines))
            .min(b.rope.len_chars());
        b.rope.remove(start..end);
        b.insert_text(&format!("\n{text}\n"), "insert");
        b.file_end();
        b.modified = true;
    }

    /// Drena respuestas de jobs AI (tick). true = redibujar.
    fn ai_tick(&mut self) -> bool {
        let mut changed = false;
        let timeout = std::time::Duration::from_secs(self.config.ai.timeout_secs);
        for job in std::mem::take(&mut self.ai_jobs) {
            match job.rx.try_recv() {
                Ok(Ok(body)) => {
                    self.replace_marker_line(job.marker.0, &body.trim_end());
                    self.status_msg("ai: respuesta lista");
                    changed = true;
                }
                Ok(Err(e)) => {
                    self.replace_marker_line(job.marker.0, &format!("✖ {e}"));
                    self.status_msg(format!("ai: {e}"));
                    changed = true;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    if job.started.elapsed() > timeout {
                        self.replace_marker_line(job.marker.0, "✖ timeout");
                        changed = true;
                    } else {
                        self.ai_jobs.push(job); // sigue esperando
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    if job.started.elapsed() > timeout {
                        self.replace_marker_line(job.marker.0, "✖ sin respuesta");
                        changed = true;
                    }
                }
            }
        }
        changed
    }

    fn search_key(&mut self, k: InputKey) {
        use crossterm::event::KeyCode as K;
        match k.code {
            K::Enter => {
                self.search = if self.search_input.is_empty() { None } else {
                    Some(self.search_input.clone())
                };
                self.search_prompt = false;
                self.do_search(true);
            }
            K::Esc => {
                self.search_prompt = false;
                self.search_input.clear();
            }
            K::Backspace => {
                self.search_input.pop();
            }
            K::Char(c) => self.search_input.push(c),
            _ => {}
        }
    }

    fn insert_key(&mut self, k: InputKey) {
        use crossterm::event::KeyCode as K;
        match (k.code, k.ctrl) {
            (K::Enter, _) => {
                if self.buffer().ai {
                    self.ai_submit();
                } else {
                    self.buffer().insert_text("\n", "insert");
                    self.trigger_completion();
                }
            }
            (K::Char(c), false) => {
                self.buffer().insert_text(&c.to_string(), "insert");
                if self.buffer().ai {
                    // transcript: no completions
                } else if self.completion.is_some() {
                    self.completion_refilter();
                } else {
                    self.trigger_completion();
                }
            }
            (K::Tab, _) => {
                let w = self.config.editor.tab_width;
                self.buffer().insert_text(&" ".repeat(w), "insert");
            }
            (K::Backspace, _) => {
                self.buffer().delete_back();
            }
            (K::Delete, _) => {
                self.buffer().delete_forward();
            }
            (K::Left, _) => self.buffer().mv(0, -1),
            (K::Right, _) => self.buffer().mv(0, 1),
            (K::Up, _) => self.buffer().mv(-1, 0),
            (K::Down, _) => self.buffer().mv(1, 0),
            _ => {}
        }
    }

    fn do_action(&mut self, a: Action) {
        use Action::*;
        // cualquier acción que no sea quit cancela la confirmación de salida
        if !matches!(a, Quit) {
            self.quit_pending = false;
        }
        match a {
            CursorLeft => self.buffer().mv(0, -1),
            CursorRight => self.buffer().mv(0, 1),
            CursorUp => self.buffer().mv(-1, 0),
            CursorDown => self.buffer().mv(1, 0),
            WordNext => self.buffer().word_next(),
            WordPrev => self.buffer().word_prev(),
            LineStart => self.buffer().line_start(),
            LineEnd => self.buffer().line_end(),
            FileStart => self.buffer().file_start(),
            FileEnd => self.buffer().file_end(),
            InsertMode => {
                self.mode = Mode::Insert;
                self.cursor_style_dirty = true;
            }
            InsertLineStart => {
                self.mode = Mode::Insert;
                self.buffer().line_start();
            }
            InsertLineEnd => {
                self.mode = Mode::Insert;
                self.buffer().line_end();
            }
            OpenBelow => {
                self.buffer().open_below();
                self.mode = Mode::Insert;
            }
            OpenAbove => {
                self.buffer().open_above();
                self.mode = Mode::Insert;
            }
            DeleteChar => {
                if !self.buffer().delete_forward() {
                    self.status_msg("fin del buffer");
                }
            }
            DeleteCharBack => {
                self.buffer().delete_back();
            }
            DeleteLine => self.buffer().delete_line(),
            Undo => self.buffer().undo(),
            Redo => self.buffer().redo(),
            SearchPrompt => {
                self.search_prompt = true;
                self.search_input.clear();
            }
            SearchNext => self.do_search(true),
            SearchPrev => self.do_search(false),
            Save => match self.buffer().save() {
                Ok(()) => {
                    self.status_msg("guardado ✓");
                    self.lsp_did_save();
                }
                Err(e) => self.status_msg(e),
            },
            Quit => {
                let persistent_dirty = self.buffers.iter().any(|b| b.is_persistent() && b.modified);
                if self.quit_pending || !persistent_dirty {
                    self.quit = true;
                } else {
                    self.quit_pending = true;
                    self.status_msg("cambios sin guardar — pulsa quit otra vez");
                }
            }
            NormalMode => {
                self.mode = Mode::Normal;
                self.cursor_style_dirty = true;
            }
            SplitVert => self.panes.split(Dir::Vert),
            SplitHoriz => self.panes.split(Dir::Horiz),
            ClosePane => {
                if self.panes.close().is_none() {
                    self.status_msg("último pane — usa salir");
                }
            }
            FocusLeft => self.focus(Focus::Left),
            FocusRight => self.focus(Focus::Right),
            FocusUp => self.focus(Focus::Up),
            FocusDown => self.focus(Focus::Down),
            FindFile => {
                let cwd = std::env::current_dir().unwrap_or_default();
                let mut f = Finder::new(&cwd);
                f.filter();
                self.finder = Some(f);
            }
            CursorStyleNext => {
                self.cursor_style = self.cursor_style.next();
                self.cursor_style_dirty = true;
            }
            Hover => self.lsp_request_at_cursor(HoverOrDef::Hover),
            GotoDef => self.lsp_request_at_cursor(HoverOrDef::Def),
            ToggleExplorer => {
                if self.sidebar.is_some() {
                    self.sidebar = None;
                    self.sidebar_focus = false;
                } else {
                    self.sidebar = Some(crate::explorer::Sidebar::new(self.project_root.clone()));
                    self.sidebar_focus = true;
                }
            }
            TabNext => self.cycle_tab(true),
            TabPrev => self.cycle_tab(false),
            CloseBuffer => self.close_buffer(),
            ToggleProblems => {
                self.problems_open = !self.problems_open;
                self.problems_focus = self.problems_open;
            }
            ToggleAi => self.ai_toggle(),
            AiContext => self.ai_context(),
        }
        self.follow_cursor();
    }

    /// Foco al pane adyacente según la geometría actual.
    fn focus(&mut self, d: Focus) {
        let rects = self.pane_rects();
        let Some((_, act)) = rects
            .iter()
            .find(|(p, _)| *p == self.panes.active)
            .map(|(p, r)| (*p, *r))
        else {
            return;
        };
        let cx = act.x + act.width / 2;
        let cy = act.y + act.height / 2;
        let target = match d {
            Focus::Left => (act.x.saturating_sub(1), cy),
            Focus::Right => (act.x + act.width, cy),
            Focus::Up => (cx, act.y.saturating_sub(1)),
            Focus::Down => (cx, act.y + act.height),
        };
        for (p, r) in &rects {
            if *p == self.panes.active {
                continue;
            }
            let inside =
                target.0 >= r.x && target.0 < r.x + r.width && target.1 >= r.y && target.1 < r.y + r.height;
            if inside {
                self.panes.active = *p;
                return;
            }
        }
        // no hay pane adyacente: silencio
    }

    /// Búsqueda smart-case, wrap en todo el buffer.
    fn do_search(&mut self, forward: bool) {
        let Some(pat) = self.search.clone() else {
            return;
        };
        if pat.is_empty() {
            return;
        }
        let smart = pat.chars().any(|c| c.is_uppercase());
        let len = self.buffer().len_lines();
        let start_line = self.buffer().cursor.0;
        let start_g = self.buffer().cursor.1;
        for step in 1..=len {
            let ln = if forward {
                (start_line + step) % len
            } else {
                (start_line + len - step % len) % len
            };
            let text = self.buffer().line_text(ln);
            // en la línea de partida el match debe quedar de-donde-estoy
            let candidates: Vec<(usize, usize)> = if ln == start_line {
                let cut = Buffer::grapheme_byte_of(&text, start_g);
                if forward {
                    find_all(&text[cut..], &pat, smart)
                        .into_iter()
                        .map(|(s, e)| (s + cut, e + cut))
                        .collect()
                } else {
                    find_all(&text[..cut], &pat, smart)
                }
            } else {
                find_all(&text, &pat, smart)
            };
            if candidates.is_empty() {
                continue;
            }
            let hit = match forward {
                true => candidates[0],
                false => candidates[candidates.len() - 1],
            };
            let g = Buffer::byte_to_grapheme(&text, hit.0);
            self.buffer().to(ln, g);
            self.follow_cursor();
            self.status_msg(format!("búsqueda: línea {}", ln + 1));
            return;
        }
        self.status_msg(format!("sin coincidencia: {pat}"));
    }
    /// didSave con texto (algunos servers lo exigen).
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

    /// Cierra clientes LSP limpio (best-effort).
    pub fn lsp_shutdown(&mut self) {
        self.lsp.shutdown();
    }
}

#[derive(Clone, Copy)]
pub enum HoverOrDef {
    Hover,
    Def,
}

/// Comparación de paths tolerante (canonicalize con fallback a string).
pub fn same_path(a: Option<&PathBuf>, b: Option<&PathBuf>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => {
            if a == b {
                return true;
            }
            std::fs::canonicalize(a).ok().as_ref()
                == std::fs::canonicalize(b).ok().as_ref()
        }
        _ => false,
    }
}

/// Corre el comando AI en threads: lector de stdout + watchdog de timeout.
/// Nunca bloquea el main loop; el kill del watchdog cierra stdout y el
/// lector termina.
fn ai_run(
    cmd: Vec<String>,
    prompt: String,
    timeout: std::time::Duration,
    tx: std::sync::mpsc::Sender<std::io::Result<String>>,
) {
    use std::process::Stdio;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    if cmd.is_empty() {
        let _ = tx.send(Err(std::io::Error::other("[ai] sin command")));
        return;
    }
    let child = match std::process::Command::new(&cmd[0])
        .args(&cmd[1..])
        .arg(&prompt)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .stdin(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(Err(e));
            return;
        }
    };
    let killed = Arc::new(AtomicBool::new(false));
    let child_shared = Arc::new(Mutex::new(child));

    // watchdog: mata pasado el timeout
    let w_child = Arc::clone(&child_shared);
    let w_killed = Arc::clone(&killed);
    std::thread::spawn(move || {
        std::thread::sleep(timeout);
        w_killed.store(true, Ordering::SeqCst);
        if let Ok(mut c) = w_child.lock() {
            let _ = c.kill();
        }
    });

    // lector: stdout hasta EOF
    let mut stdout = child_shared.lock().unwrap().stdout.take().expect("stdout");
    let mut buf = String::new();
    let mut io_err: Option<std::io::Error> = None;
    {
        use std::io::Read;
        let mut chunk = [0u8; 8192];
        loop {
            match stdout.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => buf.push_str(&String::from_utf8_lossy(&chunk[..n])),
                Err(e) => {
                    io_err = Some(e);
                    break;
                }
            }
        }
    }
    let _ = child_shared.lock().unwrap().wait();
    let _ = tx.send(match (io_err, killed.load(Ordering::SeqCst)) {
        (Some(e), _) => Err(e),
        (None, true) => Err(std::io::Error::other("timeout")),
        (None, false) => Ok(buf),
    });
}

/// Todos los matches de `pat` en `text` como rangos de bytes. smart-case:
/// si pat tiene mayúsculas, case-sensitive; si no, case-insensitive pero
/// comparando char a char (seguro ante cambios de longitud de byte).
pub fn find_all(text: &str, pat: &str, smart: bool) -> Vec<(usize, usize)> {
    if smart {
        let mut out = Vec::new();
        let mut from = 0;
        while let Some(pos) = text[from..].find(pat) {
            let s = from + pos;
            out.push((s, s + pat.len()));
            from = s + 1;
            if from >= text.len() {
                break;
            }
        }
        return out;
    }
    let pc: Vec<char> = pat.chars().collect();
    if pc.is_empty() {
        return vec![];
    }
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut out = Vec::new();
    if chars.len() < pc.len() {
        return out;
    }
    for i in 0..=chars.len() - pc.len() {
        let hit = chars[i..i + pc.len()]
            .iter()
            .zip(&pc)
            .all(|((_, a), b)| a.to_lowercase().eq(b.to_lowercase()));
        if hit {
            let s = chars[i].0;
            let e = chars.get(i + pc.len()).map(|c| c.0).unwrap_or(text.len());
            out.push((s, e));
        }
    }
    out
}
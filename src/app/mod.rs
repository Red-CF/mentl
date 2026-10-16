//! Estado del app + dispatch. La capa bazel/LSP futura conecta aquí: nueva
//! Action y nuevo match arm — nada del modelo de texto cambia.

use std::collections::HashMap;
use std::path::PathBuf;
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
    Editor,
}

mod ai;
mod ui_panels;

pub struct App {
    pub screen: Screen,
    pub mode: Mode,
    pub buffers: Vec<Option<Buffer>>,
    pub panes: Panes,
    pub finder: Option<Finder>,
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
    // welcome hub
    pub welcome_cursor: usize,
    pub welcome_input: Option<String>,
    pub anim_frame: u8,
    pub anim_last: Instant,
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


impl App {
    pub fn new(arg: Option<std::path::PathBuf>) -> Self {
        let arg_ref = arg.clone();
        let arg_dir = arg_ref.as_ref().filter(|p| p.is_dir()).cloned();
        let screen;
        let mut buffers = Vec::new();
        let no_arg = arg.is_none();
        let arg_is_dir = arg.as_ref().is_some_and(|p| p.is_dir());
        let mut msg = None;
        match arg {
            None => screen = Screen::Welcome,
            Some(_) if arg_is_dir => {
                screen = Screen::Editor;
            }
            Some(p) => match Buffer::open(&p) {
                Ok(b) => {
                    buffers.push(Some(b));
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
            buffers.push(Some(Buffer::empty(None)));
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
            welcome_cursor: 0,
            welcome_input: None,
            anim_frame: 0,
            anim_last: Instant::now(),
        };
        // proyecto real (dir arg o sin args) => árbol abierto
        if no_arg || arg_is_dir {
            let root = app.project_root.clone();
            app.sidebar = Some(crate::explorer::Sidebar::new(root));
        }

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

    /// Nº de buffers vivos (tombstones fuera).
    pub fn open_count(&self) -> usize {
        self.buffers.iter().filter(|b| b.is_some()).count()
    }

    pub fn buffer(&mut self) -> &mut Buffer {
        let idx = self.active_buffer();
        self.buffers[idx].as_mut().expect("pane apunta a buffer vivo")
    }

    /// Vecino vivo del buffer idx (prev, si no next) — para cerrar tabs.
    fn neighbor_buffer(&self, idx: usize) -> usize {
        let alive: Vec<usize> = self
            .buffers
            .iter()
            .enumerate()
            .filter_map(|(i, b)| b.is_some().then_some(i))
            .collect();
        if alive.len() <= 1 {
            return idx;
        }
        let prev = alive.iter().rev().find(|&&i| i < idx).copied();
        let next = alive.iter().find(|&&i| i > idx).copied();
        prev.or(next).unwrap_or(idx)
    }

    /// Push o retarget: si el path ya está abierto, el pane activo apunta a
    /// ese buffer (tab switch, sin didClose). Si no, push y retarget.
    pub fn open_in_pane(&mut self, path: &std::path::Path) -> Result<(), String> {
        let b = Buffer::open(path)?;
        let new_path = b.path.clone();
        // ya abierto en otro buffer? (comparación por slot distinto al activo)
        let active = self.active_buffer();
        if let Some(existing) = self
            .buffers
            .iter()
            .enumerate()
            .filter_map(|(i, bb)| bb.as_ref().map(|b| (i, b)))
            .find(|(i, b)| {
                *i != active
                    && same_path(b.path.as_ref(), Some(&path.to_path_buf()))
            })
            .map(|(i, _)| i)
        {
            self.panes.panes[self.panes.active].buffer = existing;
            self.follow_cursor();
            return Ok(());
        }
        // push del buffer nuevo (tab) y retarget
        let idx = self.buffers.len();
        self.buffers.push(Some(b));
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
        let path = self.buffers[idx].as_ref().and_then(|b| b.path.clone());
        // retarget de TODOS los panes que apunten a idx (sin renumber: ids estables)
        let neighbor = self.neighbor_buffer(idx);
        for pane in self.panes.panes.iter_mut() {
            if pane.buffer == idx {
                pane.buffer = neighbor;
            }
        }
        if self.open_count() == 1 {
            // único buffer: se sustituye por uno vacío en su slot
            self.buffers[idx] = Some(Buffer::empty(None));
        } else {
            self.buffers[idx] = None; // tombstone, id estable
        }
        // didClose solo si el path no vive en otro buffer
        if let Some(p) = path {
            let still_open = self
                .buffers
                .iter()
                .flatten()
                .any(|bb| same_path(bb.path.as_ref(), Some(&p)));
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
        // animación del welcome (frame cycling, dibujo barato)
        if self.screen == Screen::Welcome && self.config.welcome.anim {
            let ms = std::time::Duration::from_millis(self.config.welcome.anim_ms);
            if self.anim_last.elapsed() >= ms {
                self.anim_frame = self.anim_frame.wrapping_add(1);
                self.anim_last = Instant::now();
                changed = true;
            }
        }
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

    /// didOpen — solo si el server ya está ready (si no, lo manda on Started).
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
            self.welcome_key(k);
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

    // ------------------------------------------------------------- welcome --

    const MENU: [&str; 4] = ["abrir proyecto (cwd)", "buscar archivo", "abrir ruta…", "solo AI"];

    fn welcome_key(&mut self, k: InputKey) {
        use crossterm::event::KeyCode as K;
        // input de ruta activo
        if let Some(inp) = self.welcome_input.as_mut() {
            match k.code {
                K::Enter => {
                    let raw = inp.clone();
                    self.welcome_input = None;
                    self.welcome_open_path(&raw);
                }
                K::Esc => self.welcome_input = None,
                K::Backspace => {
                    inp.pop();
                }
                K::Char(c) => inp.push(c),
                _ => {}
            }
            return;
        }
        let n = Self::MENU.len();
        match k.code {
            K::Char('j') | K::Down => self.welcome_cursor = (self.welcome_cursor + 1) % n,
            K::Char('k') | K::Up => self.welcome_cursor = (self.welcome_cursor + n - 1) % n,
            K::Enter => self.welcome_execute(),
            K::Esc | K::Char('q') => self.quit = true,
            _ => {}
        }
    }

    fn welcome_execute(&mut self) {
        match self.welcome_cursor {
            0 => {
                // abrir proyecto cwd
                self.screen = Screen::Editor;
                let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
                self.project_root = root.clone();
                self.sidebar = Some(crate::explorer::Sidebar::new(root));
            }
            1 => {
                self.screen = Screen::Editor;
                let cwd = std::env::current_dir().unwrap_or_default();
                let mut f = Finder::new(&cwd);
                f.filter();
                self.finder = Some(f);
            }
            2 => {
                self.welcome_input = Some(String::new());
            }
            3 => {
                self.screen = Screen::Editor;
                self.pane_zero_empty();
                self.ai_toggle();
            }
            _ => unreachable!(),
        }
    }

    fn welcome_open_path(&mut self, raw: &str) {
        let raw_p = PathBuf::from(raw.trim());
        let p = std::fs::canonicalize(&raw_p)
            .unwrap_or_else(|_| std::env::current_dir().unwrap_or_default().join(&raw_p));
        if p.is_dir() {
            self.screen = Screen::Editor;
            self.project_root = p.clone();
            self.sidebar = Some(crate::explorer::Sidebar::new(p));
            self.pane_zero_empty();
            return;
        }
        match self.open_in_pane(&p) {
            Ok(()) => self.screen = Screen::Editor,
            Err(e) => {
                self.status_msg(format!("{e}"));
                self.screen = Screen::Editor;
            }
        }
    }

    /// Garantiza pane activo con buffer vacío (welcome → AI / dir proyectos).
    fn pane_zero_empty(&mut self) {
        self.buffers[0] = Some(Buffer::empty(None));
        self.panes.panes[self.panes.active].buffer = 0;
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

                    /// Filas del problems panel: diagnósticos por (path, línea).
        // ------------------------------------------------------------ completión --

                /// Refiltra el popup: prefijo = run de word-chars hacia atrás del cursor.
            /// 0 = pendiente de enviar: cuando pasa el debounce y el estado sigue
    /// vivo, manda la request. Con req real: nada (la respuesta llega en
    /// on_lsp y ahí se consuma).
        // ------------------------------------------------------------------ AI --

    
    
    /// Enter en insert-mode sobre un pane AI = manda el prompt (línea actual).
    
    
    /// Drena respuestas de jobs AI (tick). true = redibujar.
    
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
                let persistent_dirty = self
                    .buffers
                    .iter()
                    .flatten()
                    .any(|b| b.is_persistent() && b.modified);
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
                match self.sidebar.as_ref() {
                    // cerrada -> abrir enfocada
                    None => {
                        self.sidebar = Some(crate::explorer::Sidebar::new(self.project_root.clone()));
                        self.sidebar_focus = true;
                    }
                    // abierta sin foco -> foco (no la cierro: la cerraste sin querer una vez y duele)
                    Some(_) if !self.sidebar_focus => self.sidebar_focus = true,
                    // abierta y enfocada -> cerrar
                    Some(_) => {
                        self.sidebar = None;
                        self.sidebar_focus = false;
                    }
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
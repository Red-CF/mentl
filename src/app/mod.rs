//! Estado del app + constructor + ciclo de vida de buffers.
//! Entrada está en `keys.rs`, LSP en `lsp_glue.rs`, AI en `ai.rs`,
//! paneles de UI en `ui_panels.rs`, draw en `../draw.rs`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::buffer::Buffer;
use crate::finder::Finder;
use crate::input::InputKey;
use crate::keymap::{KeyMap, Mode};
use crate::lsp::{self, Diag};
use crate::panes::Panes;
use crate::theme::{CursorStyle, Theme};
use crate::view::Geometry;
use ratatui::layout::Rect;

mod ai;
mod keys;
mod lsp_glue;
mod lsp_types {
    pub use crate::lsp::types::CompletionItem;
}
mod ui_panels;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Welcome,
    Editor,
}

pub struct App {
    pub screen: Screen,
    pub mode: Mode,
    pub buffers: Vec<Option<Buffer>>,
    pub panes: Panes,
    pub finder: Option<Finder>,
    /// rect del área EDITOR (sidebar/tabs/panel fuera) para panes/focus/popups
    pub last_editor_rect: Rect,
    pub cursor_style: CursorStyle,
    pub cursor_style_dirty: bool,
    pub config: crate::config::Config,
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
    pub diagnostics: HashMap<PathBuf, Vec<Diag>>,
    /// hover en espera: req_id, cursor al pedir, instante
    pub hover_wait: Option<(u64, (usize, usize), Instant)>,
    pub hover: Option<String>,
    /// pending goto-def: req_id, cursor de origen, instante
    pub def_wait: Option<(u64, (usize, usize), Instant)>,
    /// didChange debounce: path -> (último rev mandado, primera ed sin mandar)
    pub sync_state: HashMap<PathBuf, (u64, Option<Instant>)>,
    // ------------------------------------------------------- fase 3 UI -----
    pub sidebar: Option<crate::explorer::Sidebar>,
    pub sidebar_focus: bool,
    pub project_root: PathBuf,
    pub problems_open: bool,
    pub problems_focus: bool,
    pub problems_cursor: usize,
    pub problems_scroll: usize,
    pub completion: Option<CompletionPopup>,
    pub completion_wait: Option<(u64, PathBuf, u64, Instant)>,
    pub ai_ctx: Option<String>,
    pub ai_jobs: Vec<AiJob>,
    // ------------------------------------------------------------ welcome --
    pub welcome_cursor: usize,
    pub welcome_input: Option<String>,
    pub anim_frame: u8,
    pub anim_last: Instant,
    // --------------------------------------------------- fase 4: edición ---
    pub selection: Option<Selection>,
    pub clipboard: (Vec<String>, bool /*kind line? */),
    pub goto_prompt: bool,
    pub goto_input: String,
    pub replace_prompt: bool,
    pub replace_stage: u8, // 0 = patrón, 1 = reemplazo
    pub replace_input: String,
    pub replace_pattern: String,
}

/// Selección: ancla fija + cabeza (cursor). Kind por cómo se entró (v/V).
#[derive(Clone, Copy, Debug)]
pub struct Selection {
    pub anchor: (usize, usize),
    pub kind: crate::buffer::SelKind,
}

pub struct CompletionPopup {
    pub items: Vec<lsp_types::CompletionItem>,
    pub filtered: Vec<usize>,
    pub cursor: usize,
}

/// Fila del problems panel.
pub struct ProblemRow {
    pub path: PathBuf,
    pub line_u16: u32,
    pub col_u16: u32,
    pub line_display: usize,
    pub severity: u32,
    pub msg: String,
}

pub struct AiJob {
    pub rx: std::sync::mpsc::Receiver<std::io::Result<String>>,
    pub started: Instant,
    pub marker: (usize, usize),
}

#[derive(Clone, Copy)]
pub enum HoverOrDef {
    Hover,
    Def,
}

impl App {
    /// Única fuente del menú welcome (draw.rs la reutiliza — sin duplicar).
    pub const MENU: [(&str, &str); 4] = [
        ("abrir proyecto", "«cwd» como árbol"),
        ("buscar archivo", "finder difuso"),
        ("abrir ruta…", ""),
        ("solo AI", "pane de claude"),
    ];

    pub fn new(arg: Option<std::path::PathBuf>) -> Self {
        let arg_ref = arg.clone();
        let arg_dir = arg.as_ref().filter(|p| p.is_dir()).cloned();
        let no_arg = arg.is_none();
        let arg_is_dir = arg_dir.is_some();
        let mut msg = None;
        let screen;
        let mut buffers: Vec<Option<Buffer>> = Vec::new();
        match arg {
            None => screen = Screen::Welcome,
            Some(_) if arg_is_dir => screen = Screen::Editor,
            Some(p) => match Buffer::open(&p) {
                Ok(b) => {
                    buffers.push(Some(b));
                    screen = Screen::Editor;
                }
                Err(e) => {
                    msg = Some((e, Instant::now()));
                    screen = Screen::Editor;
                }
            },
        }
        let (config, config_error) = match crate::config::load() {
            Ok(c) => (c, None),
            Err(e) => (crate::config::Config::default(), Some(e)),
        };
        let theme = Theme::build(&config.theme);
        let (keymap, warnings) = KeyMap::build(&config.keys);
        let msg = match (&msg, warnings.is_empty() && config_error.is_none()) {
            (Some((m, t)), _) => Some((m.clone(), *t)),
            (None, false) => Some((
                config_error.clone().unwrap_or_else(|| warnings.join(" · ")),
                Instant::now(),
            )),
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
                    arg_ref
                        .as_ref()
                        .filter(|p| p.is_file())
                        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
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
            selection: None,
            clipboard: (Vec::new(), false),
            goto_prompt: false,
            goto_input: String::new(),
            replace_prompt: false,
            replace_stage: 0,
            replace_input: String::new(),
            replace_pattern: String::new(),
        };
        // proyecto real (dir arg o sin args) => árbol abierto
        if no_arg || arg_is_dir {
            let root = app.project_root.clone();
            app.sidebar = Some(crate::explorer::Sidebar::new(root));
        }
        // arranca client si el buffer inicial es de un lenguaje LSP
        if app.screen == Screen::Editor {
            let p = app.buffer().path.clone();
            if let Some(p) = p
                && let Some(lang) = app.lsp.language_of(&p)
            {
                app.lsp
                    .ensure_client(lang, &app.config.lsp, app.config.lsp_server(lang));
                app.lsp_did_open(&p);
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
        self.buffers[idx]
            .as_mut()
            .expect("pane apunta a buffer vivo")
    }

    /// Vecino vivo del buffer idx (prev, si no next) — para cerrar tabs.
    pub(super) fn neighbor_buffer(&self, idx: usize) -> usize {
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
    pub fn open_in_pane(&mut self, path: &Path) -> Result<(), String> {
        let b = Buffer::open(path)?;
        let new_path = b.path.clone();
        let active = self.active_buffer();
        if let Some(existing) = self
            .buffers
            .iter()
            .enumerate()
            .filter_map(|(i, bb)| bb.as_ref().map(|b| (i, b)))
            .find(|(i, b)| *i != active && same_path(b.path.as_ref(), Some(&path.to_path_buf())))
            .map(|(i, _)| i)
        {
            self.panes.panes[self.panes.active].buffer = existing;
            self.follow_cursor();
            return Ok(());
        }
        let idx = self.buffers.len();
        self.buffers.push(Some(b));
        self.panes.panes[self.panes.active].buffer = idx;
        if let Some(p) = new_path
            && let Some(lang) = self.lsp.language_of(&p)
        {
            self.lsp
                .ensure_client(lang, &self.config.lsp, self.config.lsp_server(lang));
            self.lsp_did_open(&p);
        }
        Ok(())
    }

    /// Cierra el buffer del pane activo como tab: retarget de panes,
    /// didClose solo si nadie lo muestra, limpieza de diagnósticos/sync.
    pub fn close_buffer(&mut self) {
        let idx = self.active_buffer();
        let path = self.buffers[idx].as_ref().and_then(|b| b.path.clone());
        let neighbor = self.neighbor_buffer(idx);
        for pane in self.panes.panes.iter_mut() {
            if pane.buffer == idx {
                pane.buffer = neighbor;
            }
        }
        if self.open_count() == 1 {
            self.buffers[idx] = Some(Buffer::empty(None));
        } else {
            self.buffers[idx] = None; // tombstone — el id es estable
        }
        if let Some(p) = path {
            let still_open = self
                .buffers
                .iter()
                .flatten()
                .any(|bb| same_path(bb.path.as_ref(), Some(&p)));
            if !still_open && let Some(lang) = self.lsp.language_of(&p) {
                let uri = lsp::types::path_to_uri(&p);
                self.lsp.notify(
                    lang,
                    "textDocument/didClose",
                    serde_json::json!({
                        "textDocument": {"uri": uri}
                    }),
                );
                self.diagnostics.remove(&p);
                self.sync_state.remove(&p);
            }
        }
        self.follow_cursor();
    }

    /// Tab siguiente/anterior del buffer en el pane activo (wrap-around).
    pub fn cycle_tab(&mut self, forward: bool) {
        if self.open_count() <= 1 {
            return;
        }
        let cur = self.active_buffer();
        let len = self.buffers.len();
        let target = match forward {
            true => (cur + 1) % len,
            false => (cur + len - 1) % len,
        };
        if self.buffers[target].is_none() {
            return; // tombstone
        }
        self.panes.panes[self.panes.active].buffer = target;
        self.follow_cursor();
    }

    pub fn status_msg(&mut self, s: impl Into<String>) {
        self.msg = Some((s.into(), Instant::now()));
    }

    /// Tick del loop principal: chord timeout + decay de mensaje + debounce
    /// de didChange + draining AI + anim de welcome. true = redibujar.
    pub fn tick(&mut self) -> bool {
        let mut changed = false;
        if !self.pending.is_empty() {
            let to = std::time::Duration::from_millis(self.config.editor.chord_timeout_ms);
            if self.last_key.is_some_and(|t| t.elapsed() > to) {
                self.pending.clear();
                changed = true;
            }
        }
        if let Some((_, t)) = &self.msg
            && t.elapsed() >= std::time::Duration::from_secs(5)
        {
            self.msg = None;
            changed = true;
        }
        self.lsp_sync_tick();
        self.completion_tick();
        changed |= self.ai_tick();
        if self.screen == Screen::Welcome && self.config.welcome.anim {
            // floor = tick del loop — sin esto, anim_ms=0 hace redraw eterno
            let ms = std::time::Duration::from_millis(self.config.welcome.anim_ms.max(120));
            if self.anim_last.elapsed() >= ms {
                self.anim_frame = self.anim_frame.wrapping_add(1);
                self.anim_last = Instant::now();
                changed = true;
            }
        }
        changed
    }

    /// Recarga config TOML; roto = mantiene todo como estaba + msg.
    pub fn reload_config(&mut self) {
        match crate::config::load() {
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

    /// Mensaje vigente si no ha decayed (5s), sin mutar.
    pub fn live_msg(&self) -> Option<String> {
        match &self.msg {
            Some((m, t)) if t.elapsed() < std::time::Duration::from_secs(5) => Some(m.clone()),
            _ => None,
        }
    }

    fn geo(&self) -> Geometry {
        Geometry {
            height: self.last_view_h,
            scrolloff: self.config.editor.scrolloff,
        }
    }

    pub fn follow_cursor(&mut self) {
        let (l, _) = self.buffer().cursor;
        let geo = self.geo();
        self.panes.panes[self.panes.active]
            .view
            .ensure_visible(l, &geo);
    }

    /// Rects por pane (área de editor: sidebar/tabs/panel fuera).
    pub fn pane_rects(&self) -> Vec<(usize, Rect)> {
        let mut out = Vec::new();
        let mut seps = Vec::new();
        self.panes
            .layout(self.last_editor_rect, &mut out, &mut seps);
        out
    }

    /// Garantiza pane activo con buffer vacío (welcome → AI / dir proyectos).
    pub(super) fn pane_zero_empty(&mut self) {
        self.buffers[0] = Some(Buffer::empty(None));
        self.panes.panes[self.panes.active].buffer = 0;
    }
}

/// Comparación de paths tolerante (canonicalize con fallback a string).
pub fn same_path(a: Option<&PathBuf>, b: Option<&PathBuf>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => {
            if a == b {
                return true;
            }
            std::fs::canonicalize(a).ok().as_ref() == std::fs::canonicalize(b).ok().as_ref()
        }
        _ => false,
    }
}

/// Todos los matches de `pat` en `text` como rangos de bytes. smart-case:
/// si pat tiene mayúsculas, case-sensitive; si no, case-insensitive char a
/// char (seguro ante cambios de longitud de byte).
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
mod edit;

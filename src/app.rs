//! Estado del app + dispatch. La capa bazel/LSP futura conecta aquí: nueva
//! Action y nuevo match arm — nada del modelo de texto cambia.

use std::time::Instant;

use crate::action::Action;
use crate::buffer::Buffer;
use crate::config::{self, Config};
use crate::finder::Finder;
use crate::input::InputKey;
use crate::keymap::{Mode, KeyMap, Match};
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
}

impl App {
    pub fn new(arg: Option<std::path::PathBuf>) -> Self {
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
        Self {
            screen,
            mode: Mode::Normal,
            buffers,
            panes: Panes::first(0),
            finder: None,
            last_area: Rect::new(0, 0, 80, 24),
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
        }
    }

    /// Índice del buffer del pane activo.
    pub fn active_buffer(&self) -> usize {
        self.panes.buffer_of(self.panes.active)
    }

    pub fn buffer(&mut self) -> &mut Buffer {
        let idx = self.active_buffer();
        &mut self.buffers[idx]
    }

    pub fn open_in_pane(&mut self, path: &std::path::Path) -> Result<(), String> {
        let b = Buffer::open(path)?;
        let idx = self.active_buffer();
        self.buffers[idx] = b;
        Ok(())
    }

    pub fn status_msg(&mut self, s: impl Into<String>) {
        self.msg = Some((s.into(), Instant::now()));
    }

    /// Tick del loop principal: timeout de chord + decay de mensaje.
    /// Devuelve true si algo cambió y hace falta redibujar.
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
        changed
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

    /// Rects por pane (con last_area).
    pub fn pane_rects(&self) -> Vec<(usize, Rect)> {
        let mut out = Vec::new();
        let mut seps = Vec::new();
        self.panes.layout(self.last_area, &mut out, &mut seps);
        out
    }

    // --------------------------------------------------------------- keys ---

    pub fn on_key(&mut self, k: InputKey) {
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
            (K::Char(c), false) => {
                self.buffer().insert_text(&c.to_string(), "insert");
            }
            (K::Enter, _) => self.buffer().insert_text("\n", "insert"),
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
                Ok(()) => self.status_msg("guardado ✓"),
                Err(e) => self.status_msg(e),
            },
            Quit => {
                if self.quit_pending || self.buffers.iter().all(|b| !b.modified) {
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
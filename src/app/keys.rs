//! Rutas de entrada: una puerta — keymap, modes, welcome, búsqueda.

use std::time::Instant;

use std::path::PathBuf;

use crate::app::{find_all, App, HoverOrDef, Screen};
use crate::buffer::Buffer;
use crate::input::InputKey;
use crate::keymap::{Mode, Match};
use crate::action::Action;
use crate::app::Selection;
use crate::app::edit::VisualOp;
use crate::panes::{Dir, Focus};
use unicode_segmentation::UnicodeSegmentation;
use crate::finder::Finder;

impl App {
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
            BracketMatch => {
                self.buffer().bracket_match();
            }
            YankLine => self.yank_line(),
            PasteAfter => self.paste_after(),
            VisualMode => {
                self.selection = Some(Selection {
                    anchor: self.buffer().cursor,
                    kind: crate::buffer::SelKind::Char,
                });
                self.mode = Mode::Visual;
                self.cursor_style_dirty = true;
            }
            VisualLineMode => {
                self.selection = Some(Selection {
                    anchor: self.buffer().cursor,
                    kind: crate::buffer::SelKind::Line,
                });
                self.mode = Mode::Visual;
                self.cursor_style_dirty = true;
            }
            VisualChar => {
                if let Some(s) = self.selection.as_mut() {
                    s.kind = crate::buffer::SelKind::Char;
                }
            }
            VisualLine => {
                if let Some(s) = self.selection.as_mut() {
                    s.kind = crate::buffer::SelKind::Line;
                }
            }
            VisualDelete => self.visual_op(VisualOp::Delete),
            VisualYank => self.visual_op(VisualOp::Yank),
            VisualIndent => self.visual_op(VisualOp::Indent),
            VisualDedent => self.visual_op(VisualOp::Dedent),
            VisualComment => self.visual_op(VisualOp::Comment),
            ReplacePrompt => {
                self.replace_prompt = true;
                self.replace_stage = 0;
                self.replace_input.clear();
                self.replace_pattern.clear();
            }
            GotoLinePrompt => {
                self.goto_prompt = true;
                self.goto_input.clear();
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
                self.screen = Screen::Editor;
                let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
                // ya abierta al arrancar sin args — no re-caminar el árbol
                if self.sidebar.as_ref().map(|s| s.root == root) != Some(true) {
                    self.project_root = root.clone();
                    self.sidebar = Some(crate::explorer::Sidebar::new(root));
                }
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
                // se queda en welcome para reintentar; el error se ve en el hub
                self.status_msg(format!("{e}"));
            }
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
            (K::Enter, _) => {
                if self.buffer().ai {
                    self.ai_submit();
                    return;
                }
                self.enter_autoindent();
            }
            (K::Char(c), false) => {
                if self.buffer().ai {
                    self.buffer().insert_text(&c.to_string(), "insert");
                    return;
                }
                self.insert_char_smart(c);
            }
            (K::Tab, _) => {
                let unit = self.buffer().indent_unit();
                self.buffer().insert_text(&unit, "insert");
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
        if self.goto_prompt {
            self.goto_key(k);
            return;
        }
        if self.replace_prompt {
            self.replace_key(k);
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

    /// Char en insert: auto-close de pares + skip-over; el par = 1 edición.
    fn insert_char_smart(&mut self, c: char) {
        match c {
            '(' => self.insert_pair('(', ')'),
            '[' => self.insert_pair('[', ']'),
            '"' | '\'' | '`' => {
                let next = self.buffer().next_grapheme_char_at_cursor();
                if next == Some(c) && !self.prev_char_is_word() {
                    self.buffer().mv(0, 1); // salta sobre la existente
                    return;
                }
                if self.prev_char_is_word() {
                    self.buffer().insert_text(&c.to_string(), "insert");
                } else {
                    self.buffer().insert_text(&format!("{c}{c}"), "insert");
                    self.buffer().mv(0, -1);
                }
            }
            ')' | ']' | '}' => {
                let next = self.buffer().next_grapheme_char_at_cursor();
                if next == Some(c) {
                    self.buffer().mv(0, 1); // skip-over, sin rev
                    return;
                }
                self.buffer().insert_text(&c.to_string(), "insert");
            }
            ch => self.buffer().insert_text(&ch.to_string(), "insert"),
        }
        if self.completion.is_some() {
            self.completion_refilter();
        } else {
            self.trigger_completion();
        }
    }

    fn insert_pair(&mut self, open: char, close: char) {
        self.buffer().insert_text(&format!("{open}{close}"), "insert");
        self.buffer().mv(0, -1);
    }

    fn prev_char_is_word(&self) -> bool {
        let b = self.buffers[self.active_buffer()].as_ref().expect("vivo");
        let text = b.line_text(b.cursor.0);
        text.graphemes(true)
            .nth(b.cursor.1)
            .and_then(|g| g.chars().next())
            .map(crate::buffer::is_word)
            .unwrap_or(false)
    }

    /// Enter con autoindent: sangría heredada +1 unidad si abre bloque.
    fn enter_autoindent(&mut self) {
        let (l, _) = self.buffer().cursor;
        let text = self.buffer().line_text(l);
        let lead: String = text.chars().take_while(|&c| c == ' ' || c == '\t').collect();
        let unit = self.buffer().indent_unit();
        let opens = text.trim_end().ends_with('{')
            || text.trim_end().ends_with('(')
            || text.trim_end().ends_with('[');
        let extra = match opens { true => unit, false => String::new() };
        self.buffer().insert_text(&format!("\n{lead}{extra}"), "insert");
    }
}

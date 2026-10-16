//! Lote 1 de edición: visual ops, clipboard, prompts de goto/replace.


use crate::input::InputKey;
use crate::keymap::Mode;
use crate::buffer::SelKind;

pub enum VisualOp {
    Delete,
    Yank,
    Indent,
    Dedent,
    Comment,
}

impl super::App {

    /// Rango normalizado (min, max) de líneas/graphemes de la selección.
    pub(super) fn sel_range(&self) -> Option<((usize, usize), (usize, usize))> {
        let sel = self.selection?;
        let head = match self.mode {
            Mode::Visual => self.buffers[self.active_buffer()].as_ref()?.cursor,
            _ => return None,
        };
        let (a, b) = sel.anchor;
        let (c, d) = head;
        Some(if (a, b) <= (c, d) {
            ((a, b), (c, d))
        } else {
            ((c, d), (a, b))
        })
    }

    pub(super) fn visual_op(&mut self, op: VisualOp) {
        if self.mode != Mode::Visual {
            return;
        }
        let Some((min, max)) = self.sel_range() else { return };
        let line_kind = self
            .selection
            .map(|s| s.kind == SelKind::Line)
            .unwrap_or(false);
        match op {
            VisualOp::Yank => {
                let (c0, c1) = {
                    let b = self.buffer();
                    b.sel_chars(min, max, line_kind)
                };
                let text: String = {
                    let b = self.buffer();
                    b.rope.slice(c0..c1).chunks().collect()
                };
                self.clipboard = (text.lines().map(String::from).collect(), line_kind);
                self.pbcopy_async();
            }
            VisualOp::Delete | VisualOp::Indent | VisualOp::Dedent | VisualOp::Comment => {
                let b = self.buffer();
                match op {
                    VisualOp::Delete => {
                        let (c0, c1) = b.sel_chars(min, max, line_kind);
                        b.replace_range(c0, c1, "", "visual_delete");
                    }
                    VisualOp::Indent => {
                        let unit = b.indent_unit();
                        b.indent_lines(min.0, max.0, &unit, false);
                    }
                    VisualOp::Dedent => {
                        let unit = b.indent_unit();
                        b.indent_lines(min.0, max.0, &unit, true);
                    }
                    VisualOp::Comment => {
                        b.comment_toggle_lines(min.0, max.0);
                    }
                    _ => {}
                }
            }
        }
        {
            let b = self.buffer();
            b.to(min.0, 0);
        }
        self.mode = Mode::Normal;
        self.selection = None;
        self.cursor_style_dirty = true;
        self.follow_cursor();
    }

    pub(super) fn yank_line(&mut self) {
        let (l, _) = self.buffer().cursor;
        let text = self.buffer().line_text(l);
        self.clipboard = (vec![text.clone()], true);
        self.pbcopy_async();
        self.status_msg("yank línea ✓");
    }

    pub(super) fn paste_after(&mut self) {
        let (clip, line_kind) = self.clipboard.clone();
        if clip.is_empty() {
            self.status_msg("registro vacío");
            return;
        }
        if line_kind {
            // pegar línea entera debajo de la actual (cursor ACTUAL)
            let (l, _) = self.buffer().cursor;
            self.buffer().line_end();
            let text = clip.join("\n");
            self.buffer().insert_text(&format!("\n{text}"), "insert");
            self.buffer().to(l + 1, 0);
        } else {
            self.buffer()
                .insert_text(&clip.join("\n"), "insert");
        }
        self.follow_cursor();
    }

    /// pbcopy best-effort (macOS): spawn async, fracaso silencioso.
    fn pbcopy_async(&self) {
        let content = self.clipboard.0.join("\n");
        if self.clipboard.1 {
            // kind line: conserva newline cola para paste line
        }
        let _ = std::thread::Builder::new().spawn(move || {
            use std::io::Write;
            use std::process::{Command, Stdio};
            if let Ok(mut c) = Command::new("pbcopy")
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                if let Some(mut si) = c.stdin.take() {
                    let _ = si.write_all(content.as_bytes());
                }
                let _ = c.wait();
            }
        });
    }

    // ---------------------------------------------------------- goto/replace --

    /// Pide línea (o %) — comparte el estilo de barra del search prompt.
    pub(super) fn goto_key(&mut self, k: InputKey) {
        use crossterm::event::KeyCode as K;
        if !self.goto_prompt {
            return;
        }
        match k.code {
            K::Enter => {
                let raw = self.goto_input.clone();
                self.goto_prompt = false;
                self.goto_input.clear();
                self.do_goto(&raw);
            }
            K::Esc => {
                self.goto_prompt = false;
                self.goto_input.clear();
            }
            K::Backspace => {
                self.goto_input.pop();
            }
            K::Char(c) => self.goto_input.push(c),
            _ => {}
        }
    }

    pub(super) fn do_goto(&mut self, raw: &str) -> bool {
        if raw.is_empty() {
            return false;
        }
        let lines = self.buffer().len_lines();
        let cur = self.buffer().cursor.0 as isize;
        let target = match raw {
            // "+3" / "-2" relativo, "50%" porcentaje, "42" absoluto
            r if r.starts_with('+') || r.starts_with('-') => {
                let d: isize = r[1..].parse().unwrap_or(0);
                cur + d * if r.starts_with('-') { -1 } else { 1 }
            }
            r if r.ends_with('%') => {
                let p: f64 = r.trim_end_matches('%').parse().unwrap_or(0.0);
                ((p / 100.0) * (lines.saturating_sub(1) as f64)) as isize
            }
            _ => raw.parse::<isize>().unwrap_or(0).saturating_sub(1),
        };
        let mut hit = false;
        if target >= 0 && (target as usize) < lines {
            self.buffer().to(target as usize, 0);
            hit = true;
        }
        if hit {
            self.follow_cursor();
            true
        } else {
            self.status_msg(format!("línea fuera: {raw}"));
            false
        }
    }

    /// Replace en dos etapas: patrón (stage 0) → reemplazo (stage 1).
    pub(super) fn replace_key(&mut self, k: InputKey) {
        use crossterm::event::KeyCode as K;
        if !self.replace_prompt {
            return;
        }
        match (k.code, self.replace_stage) {
            (K::Enter, 0) => {
                if self.replace_input.is_empty() {
                    self.replace_prompt = false;
                    self.status_msg("pattern vacío");
                    return;
                }
                self.replace_pattern = self.replace_input.clone();
                self.replace_input.clear();
                self.replace_stage = 1;
            }
            (K::Enter, 1) => {
                let rep = self.replace_input.clone();
                self.replace_prompt = false;
                self.replace_input.clear();
                self.do_replace(&rep);
            }
            (K::Esc, _) => {
                self.replace_prompt = false;
                self.replace_input.clear();
                self.replace_pattern.clear();
            }
            (K::Backspace, _) => {
                self.replace_input.pop();
            }
            (K::Char(c), _) => self.replace_input.push(c),
            _ => {}
        }
    }

    pub(super) fn do_replace(&mut self, rep: &str) {
        if self.buffer().ai || self.buffer().path.is_none() {
            self.status_msg("replace: solo buffers con file");
            return;
        }
        let pat = self.replace_pattern.clone();
        let (line_count, match_count) = self.buffer().replace_all(&pat, rep);
        match match_count {
            0 => self.status_msg(format!("sin matches: {pat}")),
            n => self.status_msg(format!("{} match(es) en {} línea(s) ✓", n, line_count)),
        }
        self.follow_cursor();
    }
}
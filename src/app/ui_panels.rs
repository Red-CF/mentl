//! Paneles de UI: completion popup, explorer sidebar, problems panel.


use std::time::Instant;

use crate::app::{App, ProblemRow};
use super::same_path;
use crate::lsp;
use crate::buffer::Buffer;
use crate::input::InputKey;
use crate::keymap::Mode;
use unicode_segmentation::UnicodeSegmentation;

use crossterm::event::KeyCode as K;

impl App {
pub(super) fn completion_move(&mut self, d: isize) {
        let Some(p) = self.completion.as_mut() else { return };
        let n = p.filtered.len();
        if n == 0 {
            return;
        }
        let c = p.cursor as isize + d;
        p.cursor = c.clamp(0, n as isize - 1) as usize;
    }


pub(super) fn completion_accept(&mut self, which: Option<usize>) {
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


pub(super) fn completion_close(&mut self) {
        self.completion = None;
        self.completion_wait = None;
    }


pub(super) fn completion_refilter(&mut self) {
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


pub(super) fn trigger_completion(&mut self) {
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


pub(super) fn completion_tick(&mut self) {
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


pub(super) fn sidebar_key(&mut self, k: InputKey) {
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


pub(super) fn sidebar_open_file(&mut self) {
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


pub(super) fn close_sidebar(&mut self) {
        self.sidebar_focus = false;
    }


pub(super) fn problems_key(&mut self, k: InputKey) {
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

}

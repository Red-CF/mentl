//! Buffer de texto. ÚNICA fuente de cursor math: cursor = (línea 0-based,
//! grapheme 0-based excluyendo el newline). Internamente todo ropey usa
//! CHAR offsets (codepoints, 1 por grapheme mínimo). Las funciones *_byte_of
//! operan sobre &str del render (bytes), ajeno al rope. Prohibido count() de
//! chars directo fuera de este módulo.

use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Instant;
use unicode_segmentation::UnicodeSegmentation;

use ropey::Rope;

use crate::undo::{self, Undo};

pub struct Buffer {
    pub path: Option<PathBuf>,
    pub rope: Rope,
    /// (línea 0-based, grapheme 0-based en la línea, sin contar newline)
    pub cursor: (usize, usize),
    pub modified: bool,
    pub untitled: bool,
    pub undo: Undo,
    last_stroke: Option<(&'static str, Instant)>,
}

pub fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl Buffer {
    // ------------------------------------------------------------ apertura --

    pub fn open(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            return Ok(Self::empty(Some(path.to_path_buf())));
        }
        let mut f = File::open(path).map_err(|e| e.to_string())?;
        let mut head = [0u8; 8192];
        let n = f.read(&mut head).map_err(|e| e.to_string())?;
        if head[..n].contains(&0u8) {
            return Err(format!("file binario, no lo abro: {}", path.display()));
        }
        f.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        let rope = Rope::from_reader(BufReader::new(f)).map_err(|e| e.to_string())?;
        Ok(Self {
            path: Some(path.to_path_buf()),
            rope,
            cursor: (0, 0),
            modified: false,
            untitled: false,
            undo: Undo::default(),
            last_stroke: None,
        })
    }

    pub fn empty(path: Option<PathBuf>) -> Self {
        Self {
            path,
            rope: Rope::from_str(""),
            cursor: (0, 0),
            modified: false,
            untitled: true,
            undo: Undo::default(),
            last_stroke: None,
        }
    }

    /// Nombre para statusline.
    pub fn display_name(&self) -> String {
        match &self.path {
            Some(p) => p.display().to_string(),
            None => "[untitled]".into(),
        }
    }

    // ---------------------------------------------------- lectura de texto --

    /// Texto de la línea SIN el newline de cola.
    pub fn line_text(&self, line: usize) -> String {
        let (c0, c1) = self.line_char_range(line);
        self.rope.slice(c0..c1).chunks().collect()
    }

    /// Número de graphemes en la línea (sin newline).
    pub fn line_len(&self, line: usize) -> usize {
        let (c0, c1) = self.line_char_range(line);
        let text: String = self.rope.slice(c0..c1).chunks().collect();
        text.graphemes(true).count()
    }

    pub fn len_lines(&self) -> usize {
        self.rope.len_lines()
    }

    /// (start_char, end_char_excl_newlines) del contenido de la línea.
    fn line_char_range(&self, line: usize) -> (usize, usize) {
        let c0 = self.rope.line_to_char(line);
        let mut end = if line + 1 < self.len_lines() {
            self.rope.line_to_char(line + 1)
        } else {
            self.rope.len_chars()
        };
        // strip "\n" y "\r" de cola si existen
        while end > c0 {
            match self.char_at(end - 1) {
                Some('\n') | Some('\r') => end -= 1,
                _ => break,
            }
        }
        (c0, end)
    }

    /// Char del índice (solo llamado en boundaries ascii internos).
    fn char_at(&self, c: usize) -> Option<char> {
        if c >= self.rope.len_chars() {
            return None;
        }
        self.rope.slice(c..c + 1).chunks().next().and_then(|s| s.chars().next())
    }

    /// Char offset del rope donde empieza el grapheme `gidx` de la línea.
    /// gidx == line_len -> justo antes del newline.
    fn grapheme_char(&self, line: usize, gidx: usize) -> usize {
        let gidx = gidx.min(self.line_len(line));
        let text = self.line_text(line);
        let mut c = self.rope.line_to_char(line);
        for (i, g) in text.graphemes(true).enumerate() {
            if i >= gidx {
                break;
            }
            c += g.chars().count();
        }
        c
    }

    /// Char del cursor.
    fn cursor_char(&self) -> usize {
        self.grapheme_char(self.cursor.0, self.cursor.1)
    }

    /// Byte offset dentro de `text` (str de render) para un grapheme idx.
    pub fn grapheme_byte_of(text: &str, gidx: usize) -> usize {
        let mut b = 0;
        for (i, g) in text.graphemes(true).enumerate() {
            if i >= gidx {
                break;
            }
            b += g.len();
        }
        b
    }

    /// grapheme idx dentro de `text` para un byte offset en boundary.
    pub fn byte_to_grapheme(text: &str, byte: usize) -> usize {
        text.get(..byte)
            .unwrap_or(text)
            .graphemes(true)
            .count()
    }

    // ------------------------------------------------------------- cursor --

    pub fn clamp_cursor(&mut self) {
        let max = self.len_lines().saturating_sub(1);
        self.cursor.0 = self.cursor.0.min(max);
        self.cursor.1 = self.cursor.1.min(self.line_len(self.cursor.0));
    }

    /// Movimiento relativo con clamps.
    pub fn mv(&mut self, d_line: isize, d_graph: isize) {
        let l = self.cursor.0 as isize + d_line;
        self.cursor.0 = l.clamp(0, self.len_lines().saturating_sub(1) as isize) as usize;
        let g = self.cursor.1 as isize + d_graph;
        self.cursor.1 = g.clamp(0, self.line_len(self.cursor.0) as isize) as usize;
    }

    pub fn to(&mut self, line: usize, graph: usize) {
        self.cursor = (line, graph);
        self.clamp_cursor();
    }

    pub fn line_start(&mut self) {
        self.cursor.1 = 0;
    }

    pub fn line_end(&mut self) {
        self.cursor.1 = self.line_len(self.cursor.0);
    }

    pub fn file_start(&mut self) {
        self.cursor = (0, 0);
    }

    pub fn file_end(&mut self) {
        let l = self.len_lines().saturating_sub(1);
        self.cursor = (l, self.line_len(l));
    }

    // --------------------------------------------------------- palabras ----

    fn word_flags(&self, line: usize) -> Vec<bool> {
        self.line_text(line)
            .graphemes(true)
            .map(|gr| gr.chars().next().is_some_and(is_word))
            .collect()
    }

    fn word_start_from(&self, line: usize, from: usize) -> Option<usize> {
        let flags = self.word_flags(line);
        (from..flags.len()).find(|&j| flags[j] && (j == 0 || !flags[j - 1]))
    }

    fn word_back_from(&self, line: usize, from: usize) -> Option<usize> {
        let flags = self.word_flags(line);
        (0..from).rev().find(|&j| flags[j] && (j == 0 || !flags[j - 1]))
    }

    pub fn word_next(&mut self) {
        let (mut l, g) = self.cursor;
        if let Some(j) = self.word_start_from(l, g + 1) {
            self.to(l, j);
            return;
        }
        while l + 1 < self.len_lines() {
            l += 1;
            if let Some(j) = self.word_start_from(l, 0) {
                self.to(l, j);
                return;
            }
        }
        self.file_end();
    }

    pub fn word_prev(&mut self) {
        let (mut l, g) = self.cursor;
        if let Some(j) = self.word_back_from(l, g) {
            self.to(l, j);
            return;
        }
        while l > 0 {
            l -= 1;
            let len = self.line_len(l);
            if let Some(j) = self.word_back_from(l, len) {
                self.to(l, j);
                return;
            }
        }
        self.file_start();
    }

    // ------------------------------------------------------------ ediciones --

    fn snap(&mut self, stroke: &'static str) {
        if undo::mark(&mut self.last_stroke, stroke) {
            self.undo.undo.push((self.rope.clone(), self.cursor));
            if self.undo.undo.len() > undo::MAX_HISTORY {
                self.undo.undo.remove(0);
            }
            self.undo.redo.clear();
        }
    }

    /// Inserta texto en el cursor. Actualiza cursor al final del insert.
    pub fn insert_text(&mut self, text: &str, stroke: &'static str) {
        self.snap(stroke);
        let c = self.cursor_char();
        self.rope.insert(c, text);
        let last_seg = text.split('\n').next_back().unwrap_or("");
        let newlines = text.chars().filter(|ch| *ch == '\n').count();
        if newlines == 0 {
            self.cursor.1 += last_seg.graphemes(true).count();
        } else {
            self.cursor.0 += newlines;
            self.cursor.1 = last_seg.graphemes(true).count();
        }
        self.clamp_cursor();
        self.modified = true;
    }

    /// Backspace. False si no hay nada que borrar.
    pub fn delete_back(&mut self) -> bool {
        let (l, g) = self.cursor;
        if (l, g) == (0, 0) {
            return false;
        }
        self.snap("delete_back");
        if g > 0 {
            let end = self.cursor_char();
            let start = self.grapheme_char(l, g - 1);
            self.rope.remove(start..end);
            self.cursor.1 = g - 1;
        } else {
            // join con la línea anterior: borro el newline de cola de l-1
            let c = self.rope.line_to_char(l); // primer char de la línea l
            let mut start = c - 1;
            if start > 0 && self.char_at(start - 1) == Some('\r') {
                start -= 1;
            }
            self.rope.remove(start..c);
            self.cursor = (l - 1, self.line_len(l - 1));
        }
        self.clamp_cursor();
        self.modified = true;
        true
    }

    /// Delete hacia adelante. False si no hay nada.
    pub fn delete_forward(&mut self) -> bool {
        let (l, g) = self.cursor;
        if l + 1 == self.len_lines() && g >= self.line_len(l) {
            return false;
        }
        self.snap("delete_char");
        if g < self.line_len(l) {
            let start = self.cursor_char();
            let end = self.grapheme_char(l, g + 1);
            self.rope.remove(start..end);
        } else {
            // join con la línea siguiente: borro el newline de cola de l
            let c1 = self.rope.line_to_char(l + 1);
            let mut start = c1 - 1;
            if start > 0 && self.char_at(start - 1) == Some('\r') {
                start -= 1;
            }
            self.rope.remove(start..c1);
        }
        self.clamp_cursor();
        self.modified = true;
        true
    }

    /// Borra la línea entera (incluido su newline).
    pub fn delete_line(&mut self) {
        self.snap("delete_line");
        let start = self.rope.line_to_char(self.cursor.0);
        let end = if self.cursor.0 + 1 < self.len_lines() {
            self.rope.line_to_char(self.cursor.0 + 1)
        } else {
            self.rope.len_chars()
        };
        self.rope.remove(start..end);
        self.clamp_cursor();
        self.modified = true;
    }

    /// Nueva línea debajo, cursor en ella, modo insert.
    pub fn open_below(&mut self) {
        let l = self.cursor.0;
        let c = if l + 1 < self.len_lines() {
            self.rope.line_to_char(l + 1)
        } else {
            self.rope.len_chars()
        };
        self.snap("insert");
        self.rope.insert(c, "\n");
        self.cursor = (l + 1, 0);
        self.modified = true;
    }

    /// Nueva línea encima, cursor en ella, modo insert.
    pub fn open_above(&mut self) {
        self.snap("insert");
        let c = self.rope.line_to_char(self.cursor.0);
        self.rope.insert(c, "\n");
        self.cursor = (self.cursor.0, 0);
        self.modified = true;
    }

    pub fn undo(&mut self) {
        if let Some((rope, cursor)) = self.undo.undo.pop() {
            self.undo.redo.push((self.rope.clone(), self.cursor));
            self.rope = rope;
            self.cursor = cursor;
            self.clamp_cursor();
            self.modified = true;
            self.last_stroke = None;
        }
    }

    pub fn redo(&mut self) {
        if let Some((rope, cursor)) = self.undo.redo.pop() {
            self.undo.undo.push((self.rope.clone(), self.cursor));
            self.rope = rope;
            self.cursor = cursor;
            self.clamp_cursor();
            self.modified = true;
            self.last_stroke = None;
        }
    }

    // --------------------------------------------------------------- save ---

    pub fn save(&mut self) -> Result<(), String> {
        let path = self
            .path
            .clone()
            .ok_or("buffer sin ruta — no hay :w con nombre aún")?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let f = File::create(&path).map_err(|e| e.to_string())?;
        self.rope.write_to(BufWriter::new(f))
            .map_err(|e| e.to_string())?;
        self.modified = false;
        Ok(())
    }
}
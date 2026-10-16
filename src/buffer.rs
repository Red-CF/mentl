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

/// Tipo de selección visual: por char o por línea.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelKind {
    Char,
    Line,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndentKind {
    Tabs,
    Spaces(usize),
}

pub struct Buffer {
    pub path: Option<PathBuf>,
    pub rope: Rope,
    /// (línea 0-based, grapheme 0-based en la línea, sin contar newline)
    pub cursor: (usize, usize),
    pub modified: bool,
    pub indent: IndentKind,
    pub untitled: bool,
    /// buffer de transcript AI: fuera de LSP/sync/todo lo persistente
    pub ai: bool,
    /// número de revisiones de edición (para debounce de LSP didChange)
    pub rev: u64,
    pub undo: Undo,
    last_stroke: Option<(&'static str, Instant)>,
}

pub fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl Buffer {
    // ------------------------------------------------------------ apertura --

    pub fn open(path: &Path) -> Result<Self, String> {
        // SIEMPRE absoluta: las claves LSP (uri) y el mapa de diagnósticos
        // coinciden así; relativo rompe todo el LSP silenciosamente.
        let path = &std::fs::canonicalize(path).unwrap_or_else(|_| {
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                std::env::current_dir().unwrap_or_default().join(path)
            }
        });
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
        let indent = detect_indent(&rope, 4);
        Ok(Self {
            path: Some(path.to_path_buf()),
            rope,
            cursor: (0, 0),
            modified: false,
            rev: 0,
            indent,
            untitled: false,
            ai: false,
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
            rev: 0,
            indent: IndentKind::Spaces(4),
            untitled: true,
            ai: false,
            undo: Undo::default(),
            last_stroke: None,
        }
    }

    /// Transcript de AI: nunca persistent, nunca LSP.
    pub fn ai() -> Self {
        Self {
            rope: Rope::from_str("◆ claude — escribe el prompt en una línea y Enter.\n\n"),
            cursor: (2, 0),
            path: None,
            modified: false,
            rev: 0,
            indent: IndentKind::Spaces(4),
            untitled: true,
            ai: true,
            undo: Undo::default(),
            last_stroke: None,
        }
    }

    /// ¿se persiste/guarda este buffer? (AI transcript = no)
    pub fn is_persistent(&self) -> bool {
        !self.ai
    }

    /// Unidad de indentación de este buffer.
    pub fn indent_unit(&self) -> String {
        match self.indent {
            IndentKind::Tabs => "\t".to_string(),
            IndentKind::Spaces(n) => " ".repeat(n),
        }
    }

    /// Nombre para statusline: solo el archivo (las paths ya son absolutas).
    pub fn display_name(&self) -> String {
        if self.ai {
            return "◆ claude".into();
        }
        match &self.path {
            Some(p) => p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.display().to_string()),
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

    /// Grapheme tras la posición del cursor (sin mover).
    pub fn next_grapheme_char_at_cursor(&self) -> Option<char> {
        self.line_text(self.cursor.0)
            .graphemes(true)
            .skip(self.cursor.1 + 1)
            .next()
            .and_then(|g| g.chars().next())
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
            let cap = undo::history_cap(self.rope.len_bytes());
            while self.undo.undo.len() > cap {
                self.undo.undo.remove(0);
            }
            self.undo.redo.clear();
        }
    }

    /// Toda mutación pasa por aquí: modificado + revisión LSP.
    fn touch(&mut self) {
        self.modified = true;
        self.rev += 1;
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
        self.touch();
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
        self.touch();
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
        self.touch();
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
        self.touch();
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
        self.touch();
    }

    /// Nueva línea encima, cursor en ella, modo insert.
    pub fn open_above(&mut self) {
        self.snap("insert");
        let c = self.rope.line_to_char(self.cursor.0);
        self.rope.insert(c, "\n");
        self.cursor = (self.cursor.0, 0);
        self.touch();
    }

    // ---------------------------------------------------- LSP positions ----
    // LSP habla (línea 0-based, columna en UTF-16 code units). UTF-16 NO es
    // display width (tab=1 unidad, 中=1, emoji SMP=2). Conversión ÚNICA aquí.

    /// unidades UTF-16 de la línea (sin newline).
    pub fn line_utf16_len(&self, line: usize) -> usize {
        self.line_text(line)
            .graphemes(true)
            .map(|g| g.chars().map(|c| c.len_utf16()).sum::<usize>())
            .sum()
    }

    /// (línea, grapheme) -> posición LSP (línea, col UTF-16).
    pub fn lsp_pos_of(&self, line: usize, g: usize) -> (u32, u32) {
        let col: usize = self
            .line_text(line)
            .graphemes(true)
            .take(g)
            .map(|gr| gr.chars().map(|c| c.len_utf16()).sum::<usize>())
            .sum();
        (line as u32, col as u32)
    }

    /// posición LSP -> (línea, grapheme), clamp a lo existente. Una col que
    /// cae dentro de un grafo (mid-surrogate, mid-ZWJ) resuelve al inicio
    /// del grafo que la contiene.
    pub fn pos_of_lsp(&mut self, line: u32, utf16_col: u32) -> (usize, usize) {
        let line = (line as usize).min(self.len_lines().saturating_sub(1));
        let text = self.line_text(line);
        let mut acc = 0usize;
        for (gi, gr) in text.graphemes(true).enumerate() {
            let w: usize = gr.chars().map(|c| c.len_utf16()).sum();
            if acc + w > utf16_col as usize {
                return (line, gi);
            }
            acc += w;
        }
        (line, text.graphemes(true).count())
    }

    pub fn undo(&mut self) {
        if let Some((rope, cursor)) = self.undo.undo.pop() {
            self.undo.redo.push((self.rope.clone(), self.cursor));
            self.rope = rope;
            self.cursor = cursor;
            self.clamp_cursor();
            self.touch();
            self.last_stroke = None;
        }
    }

    pub fn redo(&mut self) {
        if let Some((rope, cursor)) = self.undo.redo.pop() {
            self.undo.undo.push((self.rope.clone(), self.cursor));
            self.rope = rope;
            self.cursor = cursor;
            self.clamp_cursor();
            self.touch();
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

    // =============================================== primitivas de edición ==

    /// Edición multilínea de UNA pieza: 1 snap por stroke (N líneas en un
    /// lote = 1 undo si comparten stroke). Cursor lo fija el caller.
    pub fn replace_range(&mut self, ca: usize, cb: usize, text: &str, stroke: &'static str) {
        self.snap(stroke);
        self.rope.remove(ca..cb);
        if !text.is_empty() {
            self.rope.insert(ca, text);
        }
        self.touch();
        self.clamp_cursor();
    }

    /// Prefijo de comentario por extensión.
    pub fn comment_prefix(&self) -> &'static str {
        let ext = self
            .path
            .as_ref()
            .and_then(|p| p.extension().and_then(|e| e.to_str()))
            .unwrap_or("");
        match ext {
            "py" | "sh" | "toml" | "yaml" | "yml" | "conf" => "#",
            "lua" | "sql" | "ps1" => "--",
            // rs/java/js/ts/c/cpp/hpp/go/swift/kotlin/json... default //
            _ => "//",
        }
    }

    /// Toggle de comentario sobre el rango de líneas [first, last] (incl).
    /// Decide por la primera línea no vacía. 1 undo si es en <1s.
    pub fn comment_toggle_lines(&mut self, first: usize, last: usize) {
        let last = last.min(self.len_lines().saturating_sub(1));
        let pf = self.comment_prefix();
        // decisión por la primera línea con contenido
        let mut add = true;
        for l in first..=last {
            let t = self.line_text(l);
            let nonblank = t.trim_start();
            if nonblank.is_empty() {
                continue;
            }
            add = !nonblank.starts_with(pf);
            break;
        }
        // (línea, nuevo_texto) — se aplican DESCENDENTE: los offsets aguantan
        let mut edits: Vec<(usize, String)> = Vec::new();
        for l in first..=last {
            let t = self.line_text(l);
            if t.trim().is_empty() {
                continue;
            }
            let new = if add {
                format!("{pf} {t}")
            } else if t.trim_start().starts_with(pf) {
                let stripped = t.replacen(pf, "", 1);
                // un espacio tras el prefijo también se quita
                stripped.replacen(' ', "", 1)
            } else {
                t.clone() // sin prefijo: no toca
            };
            edits.push((l, new));
        }
        for (l, new) in edits.iter().rev() {
            let (c0, c1) = self.line_char_range(*l);
            self.replace_range(c0, c1, new, "comment");
        }
    }

    /// Indent/dedent del rango [first, last]. 1 undo por stroke "indent".
    pub fn indent_lines(&mut self, first: usize, last: usize, unit: &str, dedent: bool) {
        let last = last.min(self.len_lines().saturating_sub(1));
        let mut edits: Vec<(usize, String)> = Vec::new();
        for l in first..=last {
            let t = self.line_text(l);
            if t.trim().is_empty() {
                continue;
            }
            let new = match dedent {
                true if t.starts_with(unit) => t[unit.len()..].to_string(),
                true => t.clone(), // sin unidad que quitar: no-op
                false => format!("{unit}{t}"),
            };
            edits.push((l, new));
        }
        for (l, new) in edits.iter().rev() {
            let (c0, c1) = self.line_char_range(*l);
            self.replace_range(c0, c1, new, "indent");
        }
    }

    // ------------------------------------------------------- bracket match --

    /// Al pariente correspondiente del bracket bajo el cursor. False si no.
    pub fn bracket_match(&mut self) -> bool {
        const OPEN: &str = "([{";
        const CLOSE: &str = ")]}";
        let (l, g) = self.cursor;
        let line = self.line_text(l);
        let Some(c) = line.graphemes(true).nth(g).and_then(|x| x.chars().next()) else {
            return false;
        };
        // forward: buscar el close del opener bajo el cursor
        if let Some(i) = OPEN.find(c) {
            let close = CLOSE.as_bytes()[i] as char;
            let mut depth = 0usize;
            let mut cur = l;
            let mut from = g + 1;
            loop {
                let text = self.line_text(cur);
                let gs: Vec<(usize, char)> = text
                    .graphemes(true)
                    .enumerate()
                    .skip(from)
                    .map(|(i2, x)| (i2, x.chars().next().unwrap()))
                    .collect();
                for (ix, ch) in gs {
                    if ch == c {
                        depth += 1;
                    } else if ch == close {
                        if depth == 0 {
                            self.to(cur, ix);
                            return true;
                        }
                        depth -= 1;
                    }
                }
                cur += 1;
                from = 0;
                if cur >= self.len_lines() {
                    break;
                }
            }
            return false;
        }
        // backward: buscar el opener del close bajo el cursor
        if let Some(i) = CLOSE.find(c) {
            let open = OPEN.as_bytes()[i] as char;
            let mut depth = 0usize;
            let mut cur = l;
            loop {
                let text = self.line_text(cur);
                let mut gs: Vec<(usize, char)> = text
                    .graphemes(true)
                    .enumerate()
                    .map(|(i2, x)| (i2, x.chars().next().unwrap()))
                    .collect();
                if cur == l {
                    gs.truncate(g); // el cursor mismo no entra
                }
                for (ix, ch) in gs.iter().rev() {
                    let ch = *ch;
                    if ch == c {
                        depth += 1;
                    } else if ch == open {
                        if depth == 0 {
                            self.to(cur, *ix);
                            return true;
                        }
                        depth -= 1;
                    }
                }
                if cur == 0 {
                    break;
                }
                cur -= 1;
            }
        }
        false
    }

    // ---------------------------------------------------------- replace all --

    /// Replace-all smart-case por líneas (1 undo por stroke compartido).
    /// Devuelve (líneas cambiadas, matches).
    pub fn replace_all(&mut self, pat: &str, rep: &str) -> (usize, usize) {
        if pat.is_empty() {
            return (0, 0);
        }
        let smart = pat.chars().any(|c| c.is_uppercase());
        let mut edits = Vec::new();
        let mut matches = 0;
        for l in 0..self.len_lines() {
            let text = self.line_text(l);
            if text.is_empty() {
                continue;
            }
            let hits = crate::app::find_all(&text, pat, smart);
            if hits.is_empty() {
                continue;
            }
            matches += hits.len();
            let mut new = String::new();
            let mut last = 0;
            for (hs, he) in hits {
                new.push_str(&text[last..hs]);
                new.push_str(rep);
                last = he;
            }
            new.push_str(&text[last..]);
            edits.push((l, new));
        }
        for (l, new) in edits.iter().rev() {
            let (c0, c1) = self.line_char_range(*l);
            self.replace_range(c0, c1, new, "replace");
        }
        (edits.len(), matches)
    }

    // -------------------------------------------------------------- select ---

    /// Rango en chars del rope para (min, max) de líneas/graphemes.
    /// Line-kind incluye el newline cola; última línea sin newline hasta el
    /// final del documento.
    pub fn sel_chars(&self, min: (usize, usize), max: (usize, usize), line_kind: bool) -> (usize, usize) {
        if line_kind {
            let c0 = self.rope.line_to_char(min.0);
            let c1 = if max.0 + 1 < self.len_lines() {
                self.rope.line_to_char(max.0 + 1)
            } else {
                self.rope.len_chars()
            };
            (c0, c1)
        } else {
            let c0 = self.grapheme_char(min.0, min.1);
            let c1 = self.grapheme_char(max.0, max.1);
            (c0.min(c1), c0.max(c1))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ropey::Rope;

    fn buf(text: &str) -> Buffer {
        Buffer {
            path: None,
            rope: Rope::from_str(text),
            cursor: (0, 0),
            modified: false,
            rev: 0,
            indent: IndentKind::Spaces(4),
            untitled: true,
            ai: false,
            undo: Undo::default(),
            last_stroke: None,
        }
    }

    #[test]
    fn tab_es_1_unidad_no_4() {
        let mut b = buf("\tfoo\n");
        assert_eq!(b.line_utf16_len(0), 4);
        // grapheme 1 (f) = col 1 aunque display = 4
        assert_eq!(b.lsp_pos_of(0, 1), (0, 1));
        assert_eq!(b.pos_of_lsp(0, 1), (0, 1));
        assert_eq!(b.pos_of_lsp(0, 3), (0, 3));
    }

    #[test]
    fn emoji_smp_2_unidades() {
        let mut b = buf("e\u{1F600}g\n"); // 😀
        assert_eq!(b.line_utf16_len(0), 4);
        assert_eq!(b.lsp_pos_of(0, 0), (0, 0));
        assert_eq!(b.lsp_pos_of(0, 1), (0, 1));
        assert_eq!(b.lsp_pos_of(0, 2), (0, 3), "emoji = 2 unidades");
        assert_eq!(b.lsp_pos_of(0, 3), (0, 4));
        // col en medio del surrogate pair clampa al grafo que lo contiene
        assert_eq!(b.pos_of_lsp(0, 2), (0, 1));
        assert_eq!(b.pos_of_lsp(0, 3), (0, 2));
    }

    #[test]
    fn familia_zwj_1_grapheme_8_unidades() {
        // 👨‍👩‍👧: 1 grafo, 4 BMP + 1 emoji extra... exactamente 8 unidades
        let mut b = buf("\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\n");
        assert_eq!(b.line_utf16_len(0), 8);
        assert_eq!(b.line_len(0), 1);
        // col 3 cae dentro del grafo -> grafo 0
        assert_eq!(b.pos_of_lsp(0, 3), (0, 0));
        assert_eq!(b.pos_of_lsp(0, 8), (0, 1), "eol = grafo tras el clan");
        assert_eq!(b.lsp_pos_of(0, 1), (0, 8));
    }

    #[test]
    fn combining_1_grapheme_2_unidades() {
        let mut b = buf("e\u{301}f\n"); // é combinante
        assert_eq!(b.line_utf16_len(0), 3);
        assert_eq!(b.line_len(0), 2);
        assert_eq!(b.pos_of_lsp(0, 1), (0, 0), "col dentro de é -> grafo 0");
        assert_eq!(b.pos_of_lsp(0, 2), (0, 1));
    }

    #[test]
    fn cjk_1_unidad_por_char() {
        let mut b = buf("中文\n");
        assert_eq!(b.line_utf16_len(0), 2);
        assert_eq!(b.lsp_pos_of(0, 2), (0, 2));
        assert_eq!(b.pos_of_lsp(0, 2), (0, 2));
    }

    #[test]
    fn crlf_roundtrip() {
        let mut b = buf("uno\r\ndos\r\n");
        // ropey conservará \r\n; line_text los strippea
        let (l, c) = b.pos_of_lsp(1, 3); // fin de "dos" (col 3 = eol)
        assert_eq!((l, c), (1, 3));
        assert_eq!(b.lsp_pos_of(l, c), (1, 3));
        b.to(l, c);
        assert_eq!(b.lsp_pos_of(b.cursor.0, b.cursor.1), (1, 3));
    }

    #[test]
    fn roundtrip_y_clamps() {
        let mut b = buf("a\r\n\tf\u{1F600}中文\n");
        for l in 0..b.len_lines() {
            for g in 0..=b.line_len(l) {
                let (ll, col) = b.lsp_pos_of(l, g);
                assert_eq!(b.pos_of_lsp(ll, col), (l, g), "roundtrip l{l} g{g}");
            }
        }
        // fuera de rango clampa
        assert_eq!(b.pos_of_lsp(99, 99), (b.len_lines() - 1, b.line_len(b.len_lines() - 1)));
        assert_eq!(b.lsp_pos_of(0, 999).1 as usize, b.line_utf16_len(0));
    }

    #[test]
    fn strokes_de_undo_grupan_por_accion_y_tiempo() {
        let mut b = buf("hola\n");
        b.insert_text("a", "insert"); // al cursor (0,0): "ahola"
        b.insert_text("b", "insert"); // mismo stroke <1s -> UNA undo
        assert_eq!(b.undo.undo.len(), 1);
        b.insert_text("\n", "insert"); // split: "ab"/"hola"
        // mismo stroke sigue agrupado (instant-based)
        assert_eq!(b.undo.undo.len(), 1);
        b.delete_back(); // join: "abhola"
        // stroke distinto -> snapshot nuevo
        assert_eq!(b.undo.undo.len(), 2);
        b.undo();
        assert_eq!(b.line_text(0), "ab");
        b.undo();
        assert_eq!(b.line_text(0), "hola", "2 undos: delete + sesión insert");
    }

    #[test]
    fn cap_de_historia_ajusta_por_tamano() {
        assert_eq!(undo::history_cap(999_999), 100);
        assert_eq!(undo::history_cap(1_000_001), 20);
        let b = buf("");
        // buffer chico: cap 100 — el límite activo es el de snaps reales
        assert_eq!(undo::history_cap(b.rope.len_bytes()), 100);
    }

    #[test]
    fn rev_cuenta_toda_mutacion() {
        let mut b = buf("x\n");
        let r0 = b.rev;
        b.insert_text("a", "insert");
        assert_eq!(b.rev, r0 + 1);
        b.delete_line();
        assert_eq!(b.rev, r0 + 2);
        b.undo();
        assert_eq!(b.rev, r0 + 3, "undo también muta (didChange)");
    }
}

/// Indent detection: mayoría entre tabs y patrones de espacios (≤200 líneas).
fn detect_indent(rope: &Rope, fallback: usize) -> IndentKind {
    let mut tabs = 0usize;
    let mut spaces2 = 0;
    let mut spaces4 = 0;
    for line in rope.lines().take(200) {
        let s: String = line.chunks().collect();
        let lead_len = s.len() - s.trim_start().len();
        let lead: &str = &s[..lead_len];
        match lead.chars().next() {
            Some('\t') => tabs += 1,
            Some(' ') => {
                // run de espacios: mide el prefijo común (2/4 en múltiplos)
                match lead.chars().count() {
                    n if n % 4 == 0 && n > 0 => spaces4 += 1,
                    n if n % 2 == 0 && n > 0 => spaces2 += 1,
                    _ => {}
                }
            }
            _ => {}
        }
    }
    let best_spaces = spaces4.max(spaces2);
    if tabs >= best_spaces && tabs > 0 {
        IndentKind::Tabs
    } else if spaces4 > 0 && spaces4 >= spaces2 {
        IndentKind::Spaces(4)
    } else if spaces2 > 0 {
        IndentKind::Spaces(2)
    } else {
        IndentKind::Spaces(fallback)
    }
}

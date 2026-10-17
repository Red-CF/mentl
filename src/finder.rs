//! Finder difuso: walk del cwd con skip-list, score subsequecia.

use std::path::{Path, PathBuf};

pub(crate) const SKIP_DIRS: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    "vendor",
    ".herdr",
    ".venv",
];
pub(crate) const MAX_FILES: usize = 5000;
pub(crate) const MAX_DEPTH: usize = 10;
const TOP: usize = 40;

pub struct Finder {
    pub files: Vec<PathBuf>,
    pub query: String,
    pub cursor: usize,
    pub results: Vec<PathBuf>,
}

impl Finder {
    pub fn new(root: &Path) -> Self {
        Self {
            files: walk(root),
            query: String::new(),
            cursor: 0,
            results: Vec::new(),
        }
    }

    pub fn filter(&mut self) {
        if self.query.is_empty() {
            self.results = self.files.iter().take(TOP).cloned().collect();
        } else {
            let mut scored: Vec<(i64, &PathBuf)> = self
                .files
                .iter()
                .filter_map(|f| score(&self.query, &f.to_string_lossy()).map(|s| (s, f)))
                .collect();
            scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(b.1)));
            self.results = scored
                .into_iter()
                .take(TOP)
                .map(|(_, f)| f.clone())
                .collect();
        }
        self.cursor = self.cursor.min(self.results.len().saturating_sub(1));
    }

    pub fn key(&mut self, k: crate::input::InputKey) -> Option<PathBuf> {
        // solo flechas navegan — chars SIEMPRE van a la query
        use crossterm::event::KeyCode as K;

        match k.code {
            K::Char(c) => {
                self.query.push(c);
                self.filter();
                None
            }
            K::Backspace => {
                self.query.pop();
                self.filter();
                None
            }
            K::Down => {
                if !self.results.is_empty() {
                    self.cursor = (self.cursor + 1).min(self.results.len() - 1);
                }
                None
            }
            K::Up => {
                self.cursor = self.cursor.saturating_sub(1);
                None
            }
            K::Enter => self.results.get(self.cursor).cloned(),
            K::Esc => None,
            _ => None,
        }
    }
}

/// Subsecuencia case-insensitive: score = consecutivos - penal saltos.
fn score(pat: &str, hay: &str) -> Option<i64> {
    let p: Vec<char> = pat.to_lowercase().chars().collect();
    let h: Vec<char> = hay.to_lowercase().chars().collect();
    if p.is_empty() {
        return Some(0);
    }
    let mut score = 0i64;
    let mut idx = 0usize;
    let mut last_hit: Option<usize> = None;
    for c in &p {
        let mut found = None;
        for (i, hc) in h.iter().enumerate().skip(idx) {
            if hc == c {
                found = Some(i);
                break;
            }
        }
        let fi = found?;
        score += match last_hit {
            Some(l) if fi == l + 1 => 3, // consecutivo
            Some(l) if fi > l => -((fi - l - 1) as i64),
            _ => 0,
        };
        idx = fi + 1;
        last_hit = Some(fi);
    }
    Some(score)
}

fn walk(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk_dir(root, 0, &mut out);
    out
}

fn walk_dir(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > MAX_DEPTH || out.len() >= MAX_FILES {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    for e in entries.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if is_dir {
            if SKIP_DIRS.contains(&name.as_str()) {
                continue;
            }
            dirs.push(p);
        } else {
            files.push(p);
        }
    }
    files.sort();
    dirs.sort();
    out.extend(files);
    for d in dirs {
        walk_dir(&d, depth + 1, out);
    }
}

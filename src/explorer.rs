//! Explorer: árbol LAZY del proyecto. Rebuild deriva las filas visibles
//! (recursión solo por dirs expandidos). Diag marker viene del caller.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct TreeRow {
    pub path: PathBuf,
    pub depth: u8,
    pub is_dir: bool,
}

pub struct Sidebar {
    pub root: PathBuf,
    pub rows: Vec<TreeRow>,
    pub cursor: usize,
    pub scroll: usize,
    pub expanded: HashMap<PathBuf, bool>,
}

/// listing de un dir ordenado dirs-first, skip-list compartido con finder.
pub fn list_dir(path: &Path) -> Vec<(PathBuf, bool)> {
    let mut dirs: Vec<(PathBuf, bool)> = Vec::new();
    let mut files: Vec<(PathBuf, bool)> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(path) {
        for e in rd.flatten() {
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            let name = e.file_name().to_string_lossy().into_owned();
            if is_dir {
                if crate::finder::SKIP_DIRS.contains(&name.as_str()) {
                    continue;
                }
                dirs.push((e.path(), true));
            } else {
                files.push((e.path(), false));
            }
        }
    }
    dirs.sort();
    files.sort();
    dirs.extend(files);
    dirs
}

impl Sidebar {
    pub fn new(root: PathBuf) -> Self {
        let mut s = Self {
            root: root.clone(),
            rows: Vec::new(),
            cursor: 0,
            scroll: 0,
            expanded: HashMap::new(),
        };
        s.expanded.insert(root, true); // raíz visible siempre
        s.rebuild();
        s
    }

    /// Deriva las filas visibles: solo dirs expandidos descienden.
    pub fn rebuild(&mut self) {
        let mut rows = Vec::new();
        let mut budget = crate::finder::MAX_FILES;
        Self::walk_dir(&self.root, 0, &self.expanded, &mut rows, &mut budget);
        self.rows = rows;
        self.cursor = self.cursor.min(self.rows.len().saturating_sub(1));
    }

    fn walk_dir(
        dir: &Path,
        depth: u8,
        expanded: &HashMap<PathBuf, bool>,
        rows: &mut Vec<TreeRow>,
        budget: &mut usize,
    ) {
        if *budget == 0 {
            return;
        }
        for (path, is_dir) in list_dir(dir) {
            if *budget == 0 {
                return;
            }
            *budget -= 1;
            rows.push(TreeRow { path: path.clone(), depth, is_dir });
            if is_dir && expanded.get(&path).copied().unwrap_or(false) && depth < crate::finder::MAX_DEPTH as u8 {
                Self::walk_dir(&path, depth + 1, expanded, rows, budget);
            }
        }
    }

    /// Enter/l sobre la fila del cursor: expandir/colapsar dir, o abrir file.
    pub fn toggle_row(&mut self) -> Option<&TreeRow> {
        let Some(row) = self.rows.get(self.cursor) else { return None };
        if row.is_dir {
            let open = self.expanded.get(&row.path).copied().unwrap_or(false);
            self.expanded.insert(row.path.clone(), !open);
            self.rebuild();
            return self.rows.get(self.cursor);
        }
        None
    }

    /// h sobre dir colapsado = sube al padre.
    pub fn collapse_or_parent(&mut self) {
        if let Some(row) = self.rows.get(self.cursor) {
            if row.is_dir && self.expanded.get(&row.path).copied().unwrap_or(false) {
                self.expanded.insert(row.path.clone(), false);
                self.rebuild();
                return;
            }
        }
        if let Some(row) = self.rows.get(self.cursor) {
            if let Some(parent) = row.path.parent() {
                if let Some(i) = self.rows.iter().position(|r| r.path == parent) {
                    self.cursor = i;
                    self.clamp();
                }
            }
        }
    }

    pub fn move_cursor(&mut self, d: isize) {
        let c = self.cursor as isize + d;
        self.cursor = c.clamp(0, self.rows.len().saturating_sub(1) as isize) as usize;
        self.clamp();
    }

    pub fn clamp(&mut self) {
        if self.rows.is_empty() {
            self.cursor = 0;
            self.scroll = 0;
            return;
        }
        self.cursor = self.cursor.min(self.rows.len() - 1);
        self.scroll = self.scroll.min(self.cursor);
        self.scroll = self.scroll.max(self.cursor.saturating_sub(99999));
    }
}

/// Sincroniza scroll con cursor para un viewport de `h` filas.
pub fn sync_scroll(cursor: usize, scroll: &mut usize, h: usize) {
    if h == 0 {
        return;
    }
    if cursor < *scroll {
        *scroll = cursor;
    }
    if cursor >= *scroll + h {
        *scroll = cursor + 1 - h;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fixture(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("mentl-xpl-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("target/inner")).unwrap();
        fs::create_dir_all(root.join("pkg")).unwrap();
        fs::write(root.join("Cargo.toml"), "x").unwrap();
        fs::write(root.join("pkg/mod.rs"), "x").unwrap();
        fs::write(root.join("target/junk"), "x").unwrap();
        root
    }

    #[test]
    fn list_dir_filtra_skipdirs_y_orden_dirs_primero() {
        let root = fixture("list");
        let mut entries = list_dir(&root);
        // 2 dirs (pkg) — target fuera. orden: dirs first, luego files
        let names: Vec<String> = entries
            .drain(..)
            .map(|(p, _)| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["pkg", "Cargo.toml"], "target filtrado, dirs primero");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn expand_rebuild_y_clamp() {
        let root = fixture("tree");
        let mut sb = Sidebar::new(root.clone());
        // solo fila raíz?? no: raíz expandida => sus hijos directos
        let names: Vec<String> = sb
            .rows
            .iter()
            .map(|r| r.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["pkg", "Cargo.toml"]);
        // expand pkg -> aparece mod.rs
        sb.cursor = 0;
        sb.toggle_row();
        let names: Vec<String> = sb
            .rows
            .iter()
            .map(|r| r.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"mod.rs".to_string()));
        // colapsar: cursor a mod.rs (file) -> sube al padre pkg
        sb.move_cursor(1);
        sb.collapse_or_parent();
        assert_eq!(
            sb.rows[sb.cursor].path.file_name().unwrap().to_string_lossy(),
            "pkg"
        );
        let _ = fs::remove_dir_all(&root);
    }
}
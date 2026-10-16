//! Config propia: ~/.config/mentl/config.toml. Todo tiene default en código;
//! el TOML del usuario añade/sobreescribe. TOML roto = error visible, config
//! anterior se mantiene.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use serde::Deserialize;

pub fn config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    Path::new(&home)
        .join(".config")
        .join("mentl")
        .join("config.toml")
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Editor {
    pub tab_width: usize,
    pub scrolloff: usize,
    pub chord_timeout_ms: u64,
    pub cursor_normal: String,
    pub cursor_insert: String,
}

impl Default for Editor {
    fn default() -> Self {
        Self {
                tab_width: 4,
                scrolloff: 3,
                chord_timeout_ms: 500,
                cursor_normal: "block".into(),
                cursor_insert: "bar".into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Keys {
    pub leader: String,
    pub normal: HashMap<String, String>,
    pub insert: HashMap<String, String>,
}

impl Default for Keys {
    fn default() -> Self {
        let normal = [
            ("h", "cursor_left"), ("j", "cursor_down"), ("k", "cursor_up"),
            ("l", "cursor_right"), ("w", "word_next"), ("b", "word_prev"),
            ("Up", "cursor_up"), ("Down", "cursor_down"),
            ("Left", "cursor_left"), ("Right", "cursor_right"),
            ("0", "line_start"), ("$", "line_end"),
            ("g g", "file_start"), ("G", "file_end"),
            ("i", "insert_mode"), ("I", "insert_line_start"), ("A", "insert_line_end"),
            ("o", "open_below"), ("O", "open_above"),
            ("x", "delete_char"), ("d d", "delete_line"),
            ("u", "undo"), ("C-r", "redo"),
            ("/", "search_prompt"), ("n", "search_next"), ("N", "search_prev"),
            ("C-s", "save"), ("leader w", "save"), ("leader q", "quit"),
            ("leader v", "split_vertical"), ("leader s", "split_horizontal"),
            ("leader x", "close_pane"),
            ("leader h", "focus_left"), ("leader l", "focus_right"),
            ("leader K", "focus_up"), ("leader j", "focus_down"),
            ("leader f", "find_file"), ("leader c", "cursor_style_next"),
            ("leader k", "hover"), ("leader g", "goto_definition"),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect();
        let insert = [
            ("Esc", "normal_mode"),
            ("Del", "delete_char"),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect();
        Self { leader: " ".into(), normal, insert }
    }
}

/// Colores opcionales; hex "#rrggbb". Fallback en theme.rs.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct ThemeConfig {
    pub fg: Option<String>,
    pub bg: Option<String>,
    pub gutter: Option<String>,
    pub gutter_active: Option<String>,
    pub accent: Option<String>,
    pub status_bg: Option<String>,
    pub status_insert_bg: Option<String>,
    pub match_bg: Option<String>,
    pub cursor_fg: Option<String>,
    pub diag_error: Option<String>,
    pub diag_warn: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default)]
pub struct ServerCfg {
    pub command: Vec<String>,
}
impl Default for ServerCfg {
    fn default() -> Self {
        Self { command: Vec::new() }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct LspCfg {
    pub enabled: bool,
    pub debounce_ms: u64,
    pub rust: Option<ServerCfg>,
    pub java: Option<ServerCfg>,
}
impl Default for LspCfg {
    fn default() -> Self {
        Self { enabled: true, debounce_ms: 300, rust: None, java: None }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub editor: Editor,
    pub keys: Keys,
    pub theme: ThemeConfig,
    pub lsp: LspCfg,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            editor: Editor::default(),
            keys: Keys::default(),
            theme: ThemeConfig::default(),
            lsp: LspCfg::default(),
        }
    }
}


impl Config {
    pub fn lsp_server(&self, lang: &str) -> Option<&ServerCfg> {
        match lang {
            "rust" => self.lsp.rust.as_ref(),
            "java" => self.lsp.java.as_ref(),
            _ => None,
        }
    }
}

/// Ok(config) si se cargó/es default; Err(msg) si el TOML está roto (caller
/// mantiene la config anterior y muestra el error).
pub fn load() -> Result<Config, String> {
    let path = config_path();
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return Ok(Config::default()),
    };
    toml::from_str(&content).map_err(|e| format!("config inválido: {e}"))
}
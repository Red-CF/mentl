//! Config propia: ~/.config/mentl/config.toml. Todo tiene default en código;
//! el TOML del usuario añade/sobreescribe. TOML roto = error visible, config
//! anterior se mantiene.

use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

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
    pub visual: HashMap<String, String>,
}

impl Default for Keys {
    fn default() -> Self {
        let mut normal: HashMap<String, String> = [
            ("h", "cursor_left"),
            ("j", "cursor_down"),
            ("k", "cursor_up"),
            ("l", "cursor_right"),
            ("w", "word_next"),
            ("b", "word_prev"),
            ("Up", "cursor_up"),
            ("Down", "cursor_down"),
            ("Left", "cursor_left"),
            ("Right", "cursor_right"),
            ("0", "line_start"),
            ("$", "line_end"),
            ("g g", "file_start"),
            ("G", "file_end"),
            ("i", "insert_mode"),
            ("I", "insert_line_start"),
            ("A", "insert_line_end"),
            ("o", "open_below"),
            ("O", "open_above"),
            ("x", "delete_char"),
            ("d d", "delete_line"),
            ("u", "undo"),
            ("C-r", "redo"),
            ("/", "search_prompt"),
            ("n", "search_next"),
            ("N", "search_prev"),
            ("C-s", "save"),
            ("leader w", "save"),
            ("leader q", "quit"),
            ("leader v", "split_vertical"),
            ("leader s", "split_horizontal"),
            ("leader x", "close_pane"),
            ("leader h", "focus_left"),
            ("leader l", "focus_right"),
            ("leader K", "focus_up"),
            ("leader j", "focus_down"),
            ("leader f", "find_file"),
            ("leader c", "cursor_style_next"),
            ("leader k", "hover"),
            ("leader g", "goto_definition"),
            ("leader e", "toggle_explorer"),
            ("leader n", "tab_next"),
            ("leader p", "tab_prev"),
            ("leader d", "close_buffer"),
            ("leader b", "toggle_problems"),
            ("leader i", "toggle_ai"),
            ("leader a", "ai_context"),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect();
        let insert: HashMap<String, String> = [("Esc", "normal_mode"), ("Del", "delete_char")]
            .into_iter()
            .map(|(k, v)| (k.into(), v.into()))
            .collect();
        let visual: HashMap<String, String> = [
            ("Esc", "normal_mode"),
            ("v", "visual_char"),
            ("V", "visual_line"),
            ("d", "visual_delete"),
            ("y", "visual_yank"),
            (">", "visual_indent"),
            ("<", "visual_dedent"),
            ("c", "visual_comment"),
            // mismo set de motions
            ("h", "cursor_left"),
            ("j", "cursor_down"),
            ("k", "cursor_up"),
            ("l", "cursor_right"),
            ("Up", "cursor_up"),
            ("Down", "cursor_down"),
            ("Left", "cursor_left"),
            ("Right", "cursor_right"),
            ("w", "word_next"),
            ("b", "word_prev"),
            ("0", "line_start"),
            ("$", "line_end"),
            ("g g", "file_start"),
            ("G", "file_end"),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect();
        // bindings nuevos de lote de edición (todos libres, verificado)
        normal.insert("v".into(), "visual_mode".into());
        normal.insert("V".into(), "visual_line_mode".into());
        normal.insert("y y".into(), "yank_line".into());
        normal.insert("p".into(), "paste_after".into());
        normal.insert("leader %".into(), "bracket_match".into());
        normal.insert("leader r".into(), "replace_prompt".into());
        normal.insert("leader G".into(), "goto_line_prompt".into());
        Self {
            leader: " ".into(),
            normal,
            insert,
            visual,
        }
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
    pub sidebar_bg: Option<String>,
    pub visual_bg: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default)]
#[derive(Default)]
pub struct ServerCfg {
    pub command: Vec<String>,
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
        Self {
            enabled: true,
            debounce_ms: 300,
            rust: None,
            java: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct UiCfg {
    pub sidebar_width: u16,
    pub panel_height: u16,
    pub tabs: bool,
    pub completion_debounce_ms: u64,
}
impl Default for UiCfg {
    fn default() -> Self {
        Self {
            sidebar_width: 24,
            panel_height: 6,
            tabs: true,
            completion_debounce_ms: 200,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct AiCfg {
    pub enabled: bool,
    pub command: Vec<String>,
    pub max_context_lines: usize,
    pub timeout_secs: u64,
}
impl Default for AiCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            command: vec!["claude".into(), "--output-format".into(), "text".into()],
            max_context_lines: 400,
            timeout_secs: 120,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct WelcomeCfg {
    pub anim: bool,
    pub anim_ms: u64,
}
impl Default for WelcomeCfg {
    fn default() -> Self {
        Self {
            anim: true,
            anim_ms: 150,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct Config {
    pub editor: Editor,
    pub keys: Keys,
    pub theme: ThemeConfig,
    pub lsp: LspCfg,
    pub ui: UiCfg,
    pub ai: AiCfg,
    pub welcome: WelcomeCfg,
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

//! Tema: paleta por defecto (inspiración noche, tu [theme] sobreescribe).

use ratatui::style::Color;
use crate::config::ThemeConfig;

/// Estilo de cursor geek: block / bar / underline (cicla con leader c).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CursorStyle {
    Block,
    Bar,
    Underline,
}

impl CursorStyle {
    pub fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "bar" => Self::Bar,
            "underline" => Self::Underline,
            _ => Self::Block,
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Block => Self::Bar,
            Self::Bar => Self::Underline,
            Self::Underline => Self::Block,
        }
    }
}

#[derive(Clone, Copy)]
pub struct Theme {
    pub fg: Color,
    pub bg: Color,
    pub gutter: Color,
    pub gutter_active: Color,
    pub accent: Color,
    pub status_bg: Color,
    pub status_insert_bg: Color,
    pub match_bg: Color,
    pub cursor_fg: Color,
}

fn hex(s: &str) -> Option<Color> {
    let s = s.trim().strip_prefix("#")?;
    if s.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some(Color::Rgb(r, g, b))
}

impl Theme {
    pub fn build(cfg: &ThemeConfig) -> Self {
        let pick = |opt: &Option<String>, default: Color| opt.as_deref().and_then(hex).unwrap_or(default);
        Self {
            fg: pick(&cfg.fg, Color::Rgb(0xc0, 0xca, 0xf5)),
            bg: pick(&cfg.bg, Color::Rgb(0x13, 0x17, 0x22)),
            gutter: pick(&cfg.gutter, Color::Rgb(0x3b, 0x42, 0x61)),
            gutter_active: pick(&cfg.gutter_active, Color::Rgb(0x7a, 0xa2, 0xf7)),
            accent: pick(&cfg.accent, Color::Rgb(0x7a, 0xa2, 0xf7)),
            status_bg: pick(&cfg.status_bg, Color::Rgb(0x16, 0x16, 0x1e)),
            status_insert_bg: pick(&cfg.status_insert_bg, Color::Rgb(0x2a, 0x31, 0x46)),
            match_bg: pick(&cfg.match_bg, Color::Rgb(0x4f, 0xc4, 0x9c)),
            cursor_fg: pick(&cfg.cursor_fg, Color::Rgb(0x1a, 0x1b, 0x26)),
        }
    }
}
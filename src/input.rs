//! Normalización de input: crossterm event -> InputKey, y parseo del formato
//! TOML ("C-r", "Esc", "g g" — espacio separa secuencia).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct InputKey {
    pub code: KeyCode,
    pub ctrl: bool,
    pub alt: bool,
}

impl InputKey {
    pub fn simple(code: KeyCode) -> Self {
        Self { code, ctrl: false, alt: false }
    }

    /// None para eventos que no son pulsados por el usuario.
    pub fn from_event(ev: KeyEvent) -> Option<Self> {
        if ev.kind == crossterm::event::KeyEventKind::Release {
            return None;
        }
        Some(Self {
            code: ev.code,
            ctrl: ev.modifiers.contains(KeyModifiers::CONTROL),
            alt: ev.modifiers.contains(KeyModifiers::ALT),
        })
    }

    /// Parsea un token del TOML. Las mayúsculas llevan su propio case
    /// ("G" != "g"); "S-" no se modela para chars.
    pub fn parse(token: &str) -> Result<Self, String> {
        // el leader es a menudo un espacio — no trims lo vacíes
        if token == " " {
            return Ok(Self { code: KeyCode::Char(' '), ctrl: false, alt: false });
        }
        let t = token.trim();
        if t.is_empty() {
            return Err("token de tecla vacío".into());
        }
        let mut ctrl = false;
        let mut alt = false;
        let mut rest = t;
        loop {
            if let Some(r) = rest.strip_prefix("C-") {
                ctrl = true;
                rest = r;
            } else if let Some(r) = rest.strip_prefix("A-") {
                alt = true;
                rest = r;
            } else {
                break;
            }
        }
        let code = match rest.to_ascii_lowercase().as_str() {
            "esc" => KeyCode::Esc,
            "enter" | "cr" => KeyCode::Enter,
            "tab" => KeyCode::Tab,
            "space" => KeyCode::Char(' '),
            "bsp" | "backspace" => KeyCode::Backspace,
            "del" => KeyCode::Delete,
            "up" => KeyCode::Up,
            "down" => KeyCode::Down,
            "left" => KeyCode::Left,
            "right" => KeyCode::Right,
            _ => {
                let mut chars = rest.chars();
                let first = chars.next().unwrap();
                if chars.next().is_some() {
                    return Err(format!("clave desconocida: «{t}»"));
                }
                KeyCode::Char(first)
            }
        };
        Ok(Self { code, ctrl, alt })
    }
}
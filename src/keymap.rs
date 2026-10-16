//! Keymap: bindings (lista plana, MVP scale) + matching longest-prefix con
//! secuencia pendiente. "g g" = chord, "leader q" expande el leader del config.

use std::collections::HashMap;
use crate::action;
use crate::config::Keys;
use crate::input::InputKey;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mode {
    Normal,
    Insert,
    Visual,
}

#[derive(Debug)]
pub struct Bind {
    pub keys: Vec<InputKey>,
    pub action: String,
}

pub enum Match {
    Action(action::Action),
    Pending,
    None,
}

#[derive(Default)]
pub struct KeyMap {
    pub modes: HashMap<Mode, Vec<Bind>>,
}

impl KeyMap {
    /// Construye desde la sección [keys] del TOML. Los binds con acción
    /// desconocida se descartan con mensaje.
    pub fn build(cfg: &Keys) -> (Self, Vec<String>) {
        let mut map = Self::default();
        let mut warnings = Vec::new();
        for (mode, table) in [
            (Mode::Normal, &cfg.normal),
            (Mode::Insert, &cfg.insert),
            (Mode::Visual, &cfg.visual),
        ] {
            for (seq, act) in table {
                let ids: Vec<&str> = seq.split_whitespace().collect();
                let mut keys = Vec::new();
                let mut ok = true;
                for tok in ids {
                    if tok == "leader" {
                        match InputKey::parse(&cfg.leader) {
                            Ok(k) => keys.push(k),
                            Err(e) => {
                                warnings.push(format!("leader inválido: {e}"));
                                ok = false;
                                break;
                            }
                        }
                    } else {
                        match InputKey::parse(tok) {
                            Ok(k) => keys.push(k),
                            Err(e) => {
                                warnings.push(format!("bind «{seq}»: {e}"));
                                ok = false;
                                break;
                            }
                        }
                    }
                }
                if !ok {
                    continue;
                }
                if action::parse(act).is_none() {
                    warnings.push(format!("acción desconocida «{act}» para «{seq}»"));
                    continue;
                }
                map.modes.entry(mode).or_default().push(Bind { keys, action: act.clone() });
            }
        }
        (map, warnings)
    }

    /// Match de secuencia. Longitud completa -> Action; prefijo de algún bind
    /// más largo -> Pending; nada -> None.
    pub fn match_seq(&self, mode: Mode, seq: &[InputKey]) -> Match {
        if seq.is_empty() {
            return Match::None;
        }
        let Some(binds) = self.modes.get(&mode) else {
            return Match::None;
        };
        let mut prefix = false;
        for b in binds {
            if b.keys.len() == seq.len() && b.keys.as_slice() == seq {
                return Match::Action(action::parse(&b.action).unwrap());
            }
            if b.keys.len() > seq.len() && b.keys[..seq.len()] == *seq {
                prefix = true;
            }
        }
        if prefix {
            Match::Pending
        } else {
            Match::None
        }
    }
}
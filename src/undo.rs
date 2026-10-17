//! Undo por snapshot del rope: ropey clona barato, corrección trivial.
//! Stroke con agrupación: misma marca en <1s = un solo undo (la ventana se
//! refresca con cada edición de la misma marca, agrupa sesiones de tecleo).

use ropey::Rope;
use std::time::{Duration, Instant};

pub type Snap = (Rope, (usize, usize));

pub const STROKE_GAP: Duration = Duration::from_secs(1);
pub const MAX_HISTORY: usize = 100;
/// Buffers gordos: 100 snaps de rope completa = decenas de MB. Cap proporcional.
pub const BIG_BUFFER_BYTES: usize = 1_000_000;
pub const MAX_HISTORY_BIG: usize = 20;

pub fn history_cap(rope_bytes: usize) -> usize {
    if rope_bytes > BIG_BUFFER_BYTES {
        MAX_HISTORY_BIG
    } else {
        MAX_HISTORY
    }
}

#[derive(Default)]
pub struct Undo {
    pub undo: Vec<Snap>,
    pub redo: Vec<Snap>,
}

/// Devuelve true si hay que snapshotar ANTES de editar:
/// nueva marca, marca distinta a la anterior, o pasadas >1s de la última.
pub fn mark(last: &mut Option<(&'static str, Instant)>, stroke: &'static str) -> bool {
    let boundary = !matches!(
        last,
        Some((s, t)) if *s == stroke && t.elapsed() <= STROKE_GAP
    );
    match last {
        Some((s, _)) if *s == stroke => *last = Some((s, Instant::now())),
        _ => *last = Some((stroke, Instant::now())),
    }
    boundary
}

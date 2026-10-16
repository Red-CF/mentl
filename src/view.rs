//! View: scroll vertical + reglas de visibilidad. Fase 1 añade splits = más
//! views apuntando a los mismos buffers.

#[derive(Default)]
pub struct View {
    pub scroll: usize, // primera línea visible
}

pub struct Geometry {
    pub height: usize,
    pub scrolloff: usize,
}

impl View {
    /// Mantiene la línea del cursor visible respetando scrolloff.
    pub fn ensure_visible(&mut self, cursor_line: usize, geo: &Geometry) {
        if geo.height == 0 {
            return;
        }
        let h = geo.height;
        let so = geo.scrolloff.min(h.saturating_sub(1) / 2);
        if cursor_line < self.scroll.saturating_add(so.min(cursor_line + 1)) {
            // por arriba: scroll = cursor - so
            self.scroll = cursor_line.saturating_sub(so);
        }
        if cursor_line + so >= self.scroll.saturating_add(h) {
            // por abajo: scroll = cursor + so + 1 - h
            self.scroll = (cursor_line + so + 1).saturating_sub(h);
        }
    }

    /// Rango de líneas a renderizar en un viewport de `height`.
    pub fn range(&self, height: usize, len_lines: usize) -> std::ops::Range<usize> {
        let start = self.scroll.min(len_lines.saturating_sub(1));
        start..(start + height).min(len_lines)
    }
}

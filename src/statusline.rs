//! Spans del statusline: chip de modo, nombre, msg/prompt de búsqueda,
//! posición a la derecha.

use ratatui::style::{Color as C, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::app::App;
use crate::keymap::Mode;

pub fn build(app: &mut App, width: usize) -> Line<'static> {
    let t = app.theme;
    let mut spans: Vec<Span> = Vec::new();

    // chip de modo (8 celdas fijas)
    let (label, chip_bg) = match app.mode {
        Mode::Normal => (" NORMAL ", t.status_bg),
        Mode::Insert => (" INSERT ", t.status_insert_bg),
    };
    spans.push(Span::styled(label, Style::new().bg(chip_bg).fg(t.accent)));

    // centro: prompt búsqueda / mensaje / nombre
    let center = if app.search_prompt {
        format!("buscar: {}▏", app.search_input)
    } else {
        app.live_msg().unwrap_or_default()
    };
    let name = app.buffer().display_name();
    let name_span = format!(" {}{}", name, if app.buffer().modified { " [+] " } else { " " });
    spans.push(Span::styled(name_span, Style::new().bg(t.status_bg).fg(t.fg)));

    if !center.is_empty() {
        spans.push(Span::styled(
            format!("{center} "),
            Style::new().bg(t.status_bg).fg(t.accent),
        ));
    }

    // derecha: Ln/Col
    let (l, g) = app.buffer().cursor;
    let right = format!("ln {}, col {} ", l + 1, g + 1);
    let used: usize = spans.iter().map(|s| s.content.width()).sum();
    let pad = width.saturating_sub(used + right.width());
    spans.push(Span::styled(" ".repeat(pad), Style::new().bg(t.status_bg)));
    spans.push(Span::styled(right, Style::new().bg(t.status_bg).fg(t.gutter)));

    Line::from(spans).style(Style::new().bg(t.status_bg))
}

/// Color de fondo usado por todo el statusline (para un rect de fondo).
pub fn bg(app: &App) -> C {
    app.theme.status_bg
}
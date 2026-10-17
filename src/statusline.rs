//! Spans del statusline: chip de modo, nombre, msg/prompt de búsqueda,
//! posición a la derecha.

use ratatui::style::Style;
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
        Mode::Visual => match app.selection.map(|s| s.kind) {
            Some(crate::buffer::SelKind::Line) => (" V-LINE ", t.status_insert_bg),
            _ => (" VISUAL ", t.status_insert_bg),
        },
    };
    spans.push(Span::styled(label, Style::new().bg(chip_bg).fg(t.accent)));

    // centro: prompt búsqueda / mensaje / nombre
    let center = if app.search_prompt {
        format!("buscar: {}▏", app.search_input)
    } else {
        app.live_msg().unwrap_or_default()
    };
    // centro: solo el nombre (+ [+]) — los chips van como spans aparte
    let name = app.buffer().display_name();
    let modified = if app.buffer().modified { " [+]" } else { "" };
    let name_span = format!(" {name}{modified} ");
    spans.push(Span::styled(
        name_span,
        Style::new().bg(t.status_bg).fg(t.fg),
    ));

    // chip de branch (solo si hay repo)
    let branch_chip = app.git_branch.clone().unwrap_or_default();
    // chips de diagnósticos + estado lsp (spans coloreados aparte)
    let path = app.buffer().path.clone();
    let (nd_err, nd_warn) = path
        .as_ref()
        .and_then(|p| app.diagnostics.get(p))
        .map(|ds| {
            (
                ds.iter().filter(|d| d.severity == 1).count(),
                ds.iter().filter(|d| d.severity == 2).count(),
            )
        })
        .unwrap_or((0, 0));
    let diag_chip = match (nd_err, nd_warn) {
        (0, 0) => String::new(),
        (e, 0) => format!("E{e}"),
        (0, w) => format!("W{w}"),
        (e, w) => format!("E{e} W{w}"),
    };
    let lang = path.as_ref().and_then(|p| app.lsp.language_of(p));
    let lsp_chip = match lang {
        Some(l) if app.lsp.is_ready(l) => String::new(),
        Some(l) if app.lsp.ready(l).is_some() => format!("lsp:{l} ⋯"),
        _ => String::new(),
    };
    if !branch_chip.is_empty() {
        spans.push(Span::styled(
            format!(" {branch_chip} "),
            Style::new().bg(t.status_bg).fg(t.gutter),
        ));
    }
    if !diag_chip.is_empty() {
        let style = if nd_err > 0 {
            Style::new().bg(t.status_bg).fg(t.diag_error)
        } else {
            Style::new().bg(t.status_bg).fg(t.diag_warn)
        };
        spans.push(Span::styled(format!("{diag_chip} "), style));
    }
    if !lsp_chip.is_empty() {
        spans.push(Span::styled(
            format!("{lsp_chip} "),
            Style::new().bg(t.status_bg).fg(t.gutter),
        ));
    }

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
    spans.push(Span::styled(
        right,
        Style::new().bg(t.status_bg).fg(t.gutter),
    ));

    Line::from(spans).style(Style::new().bg(t.status_bg))
}

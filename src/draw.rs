//! Render: panes (árbol), separadores, finder overlay, statusline, cursor.

use crate::app::{App, Screen};
use crate::app::find_all;
use crate::buffer::Buffer;
use crate::keymap::Mode;
use crate::panes::Dir;
use crate::statusline;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;
use unicode_segmentation::UnicodeSegmentation;

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    f.render_widget(
        Block::default().style(Style::new().bg(app.theme.bg)),
        area,
    );
    match app.screen {
        Screen::Welcome => draw_welcome(f, app),
        Screen::Picker => draw_picker(f, area, app),
        Screen::Editor => draw_editor(f, area, app),
    }
}

fn draw_editor(f: &mut Frame, area: Rect, app: &mut App) {
    let h = area.height.saturating_sub(1) as usize;
    app.last_area = area;
    app.last_view_h = h;
    app.follow_cursor();

    let editor = Rect { height: area.height - 1, ..area };
    let mut rects = Vec::new();
    let mut seps = Vec::new();
    app.panes.layout(editor, &mut rects, &mut seps);

    // separadores primero (para que el texto no los tape si acaso: no, texto
    // no sale de su rect; orden irrelevante — aquí van después por claridad)
    for (dir, mid, r) in &seps {
        let style = Style::new().fg(app.theme.gutter);
        match dir {
            Dir::Vert => {
                if r.width >= 2 {
                    let v: Vec<Span> = (r.y..r.y + r.height)
                        .map(|_| Span::styled("│", style))
                        .collect();
                    f.render_widget(
                        Paragraph::new(Line::from(v)),
                        Rect { x: *mid, y: r.y, width: 1, height: r.height },
                    );
                }
            }
            Dir::Horiz => {
                if r.height >= 2 {
                    let line: String = "─".repeat(r.width as usize);
                    f.render_widget(
                        Paragraph::new(line).style(style),
                        Rect { x: r.x, y: *mid, width: r.width, height: 1 },
                    );
                }
            }
        }
    }

    // panes
    let mut cursor_pos: Option<(u16, u16)> = None;
    let pane_specs: Vec<(usize, Rect, usize)> = rects
        .iter()
        .filter_map(|(id, r)| {
            let pane = app.panes.panes.get(*id)?;
            Some((*id, *r, pane.buffer))
        })
        .collect();
    for (pane_id, rect, buffer_idx) in pane_specs {
        let active = pane_id == app.panes.active;
        let Some(pane) = app.panes.panes.get(pane_id) else { continue };
        let scroll = pane.view.scroll;
        // severidad por línea (min = más grave primero): 1 error > 2 warn > resto
        let path = app.buffers[buffer_idx].path.clone();
        let mut diag_by_line: std::collections::HashMap<usize, u32> = std::collections::HashMap::new();
        if let Some(p) = path.as_ref() {
            if let Some(ds) = app.diagnostics.get(p) {
                for d in ds {
                    let e = diag_by_line.entry(d.line).or_insert(u32::MAX);
                    *e = (*e).min(d.severity);
                }
            }
        }
        draw_pane(
            f,
            rect,
            &app.buffers[buffer_idx],
            scroll,
            active,
            app.config.editor.tab_width,
            app.mode,
            app.search.clone(),
            app.theme,
            &diag_by_line,
            &mut cursor_pos,
        );
    }

    // statusline
    let st_rect = Rect {
        x: area.x,
        y: area.y + area.height - 1,
        width: area.width,
        height: 1,
    };
    let line = statusline::build(app, area.width as usize);
    f.render_widget(Paragraph::new(line), st_rect);

    // finder overlay último (encima de todo)
    if app.finder.is_some() {
        cursor_pos = None;
        draw_finder(f, area, app);
    } else if app.hover.is_some() {
        cursor_pos = None;
        draw_hover(f, area, app);
    }

    if let Some((x, y)) = cursor_pos {
        f.set_cursor_position(ratatui::layout::Position::from((x, y)));
    }
}

fn draw_pane(
    f: &mut Frame,
    rect: Rect,
    buf: &Buffer,
    scroll: usize,
    active: bool,
    tab_w: usize,
    mode: Mode,
    pat: Option<String>,
    theme: crate::theme::Theme,
    diag_by_line: &std::collections::HashMap<usize, u32>,
    cursor_pos: &mut Option<(u16, u16)>,
) {
    let t = theme;
    let len = buf.len_lines();
    let gutter_w = (len.to_string().len()).max(2) + 1;
    let cursor = buf.cursor;

    // gutter
    let mut gut: Vec<Line> = Vec::new();
    for i in scroll..(scroll + rect.height as usize).min(len) {
        // símbolo diagnóstico en el hueco del separador (sin shift de layout)
        let (sep_char, sep_style) = match diag_by_line.get(&i) {
            Some(1) => ("!", Style::new().fg(t.diag_error)),
            Some(2) => ("!", Style::new().fg(t.diag_warn)),
            Some(_) => ("i", Style::new().fg(t.gutter)),
            None => (" ", Style::new()),
        };
        let base = if i == cursor.0 { t.gutter_active } else { t.gutter };
        let style = match diag_by_line.get(&i) {
            Some(1) | Some(2) => sep_style, // número coloreado por severidad
            _ => Style::new().fg(base),
        };
        gut.push(Line::styled(
            format!("{:>width$}{}", i + 1, sep_char, width = gutter_w - 1),
            style,
        ));
    }
    f.render_widget(
        Paragraph::new(gut),
        Rect { x: rect.x, y: rect.y, width: gutter_w as u16, height: rect.height },
    );

    // texto
    let smart = pat.as_ref().is_some_and(|p| p.chars().any(|c| c.is_uppercase()));
    let mut rows: Vec<Line> = Vec::new();
    for i in scroll..(scroll + rect.height as usize).min(len) {
        let text = buf.line_text(i);
        let cursor_byte = if i == cursor.0 {
            Some((Buffer::grapheme_byte_of(&text, cursor.1), mode))
        } else {
            None
        };
        let matches = pat
            .as_ref()
            .map(|p| find_all(&text, p, smart))
            .unwrap_or_default();
        rows.push(render_line(&text, cursor_byte, &matches, &t));
    }
    let text_rect = Rect {
        x: rect.x + gutter_w as u16,
        y: rect.y,
        width: rect.width.saturating_sub(gutter_w as u16),
        height: rect.height,
    };
    f.render_widget(Paragraph::new(rows), text_rect);

    // posición del cursor terminal (pane activo)
    if active {
        let row = cursor
            .0
            .saturating_sub(scroll)
            .min(rect.height.saturating_sub(1) as usize);
        let line_text = buf.line_text(cursor.0);
        let byte = Buffer::grapheme_byte_of(&line_text, cursor.1);
        let col = display_width(line_text.get(..byte).unwrap_or(""), tab_w);
        *cursor_pos = Some((rect.x + gutter_w as u16 + col as u16, rect.y + row as u16));
    }
}

/// Ancho de display de un segmento de str con tabs expandidos.
fn display_width(seg: &str, tab_w: usize) -> usize {
    let w: usize = seg
        .graphemes(true)
        .map(|g| if g == "\t" { tab_w } else { unicode_width::UnicodeWidthStr::width(g) })
        .sum();
    w
}

fn draw_hover(f: &mut Frame, area: Rect, app: &mut App) {
    let Some(hover) = &app.hover else { return };
    let t = app.theme;
    let w = ((area.width as f32 * 0.5) as u16).min(70).max(30);
    let h = 8.min(area.height.saturating_sub(2));
    let x = area.x + area.width.saturating_sub(w + 2);
    let y = area.y + 1;
    let rect = Rect { x, y, width: w, height: h };
    let block = Block::default()
        .title(" hover ")
        .borders(ratatui::widgets::Borders::ALL)
        .border_style(Style::new().fg(t.accent))
        .style(Style::new().bg(t.bg));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    f.render_widget(
        Paragraph::new(hover.clone()).wrap(ratatui::widgets::Wrap { trim: false }),
        inner,
    );
}

fn draw_finder(f: &mut Frame, area: Rect, app: &mut App) {
    let Some(finder) = &mut app.finder else { return };
    let t = app.theme;
    let (w, h) = (
        ((area.width as f32 * 0.6) as u16).min(70).max(30),
        14.min(area.height.saturating_sub(2)),
    );
    let x = area.x + (area.width - w) / 2;
    let y = area.y + area.height / 8;
    let rect = Rect { x, y, width: w, height: h };

    let block = Block::default()
        .title(" archivo ")
        .borders(ratatui::widgets::Borders::ALL)
        .border_style(Style::new().fg(t.accent))
        .style(Style::new().bg(t.bg));
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let mut lines = vec![Line::styled(format!("▸ {}", finder.query), Style::new().fg(t.accent))];
    let rows = inner.height.saturating_sub(2) as usize;
    for (i, p) in finder.results.iter().enumerate().take(rows) {
        let name = p.display().to_string();
        if i == finder.cursor {
            lines.push(Line::styled(
                format!("> {name}"),
                Style::new().bg(t.accent).fg(t.cursor_fg),
            ));
        } else {
            lines.push(Line::styled(format!("  {name}"), Style::new().fg(t.fg)));
        }
    }
    f.render_widget(Paragraph::new(lines), inner);
}

/// Spans de una línea con estilo por segmentos: cursor > match > normal.
/// Cubre TODA la línea (0..len) — ancla en 0 y len obligatoriamente.
fn render_line(
    text: &str,
    cursor_byte: Option<(usize, Mode)>,
    matches: &[(usize, usize)],
    t: &crate::theme::Theme,
) -> Line<'static> {
    if text.is_empty() {
        // línea vacía: el cursor sigue visible (celda en blanco con estilo)
        return match cursor_byte {
            Some((0, Mode::Normal)) => Line::raw(" ")
                .style(Style::new().bg(t.accent).fg(t.cursor_fg)),
            Some((0, Mode::Insert)) => Line::raw(" ")
                .style(Style::new().underlined().fg(t.fg).underline_color(t.accent)),
            _ => Line::styled("", Style::new().fg(t.fg)),
        };
    }
    let cursor_range = cursor_byte.map(|(cb, m)| {
        let next = text[cb..].graphemes(true).next().map(|g| g.len()).unwrap_or(0);
        (cb, (cb + next).min(text.len()), m)
    });
    let mut marks: Vec<(usize, usize, bool)> = Vec::new();
    if let Some((s, e, _)) = cursor_range {
        marks.push((s, e, true));
    }
    for (s, e) in matches {
        marks.push((*s, *e, false));
    }
    let mut pts: Vec<usize> = marks.iter().flat_map(|m| [m.0, m.1]).collect();
    pts.push(0);
    pts.push(text.len());
    pts.sort_unstable();
    pts.dedup();
    let mut spans = Vec::new();
    for win in pts.windows(2) {
        let (a, b) = (win[0], win[1]);
        if b <= a {
            continue;
        }
        let cursor_mode = cursor_range.filter(|(s, e, _)| a >= *s && a < *e).map(|(_, _, m)| m);
        let is_match = matches.iter().any(|(s, e)| a >= *s && a < *e);
        let piece = text.get(a..b).unwrap_or("").to_string();
        let style = match cursor_mode {
            // NORMAL: bloque sólido; INSERT: underline sobre el char real
            Some(Mode::Normal) => Style::new().bg(t.accent).fg(t.cursor_fg),
            Some(Mode::Insert) => {
                Style::new().underlined().fg(t.fg).underline_color(t.accent)
            }
            _ if is_match => Style::new().bg(t.match_bg).fg(t.cursor_fg),
            _ => Style::new().fg(t.fg),
        };
        spans.push(Span::styled(piece, style));
    }
    Line::from(spans)
}

fn draw_welcome(f: &mut Frame, app: &App) {
    let area = f.area();
    let t = app.theme;
    let center_row = area.height / 2;
    let lines = vec![
        Line::styled("MENTL v0.2.0", Style::new().fg(t.accent)),
        Line::styled("", Style::new()),
        Line::styled("tu editor, tu configuración", Style::new().fg(t.fg)),
        Line::styled("", Style::new()),
        Line::styled("abre un archivo:  mentl ruta/al/archivo", Style::new().fg(t.gutter)),
        Line::styled("abre un dir:      mentl .", Style::new().fg(t.gutter)),
        Line::styled("config:           ~/.config/mentl/config.toml", Style::new().fg(t.gutter)),
        Line::styled("", Style::new()),
        Line::styled("q / Esc para salir", Style::new().fg(t.gutter)),
    ];
    let y0 = center_row.saturating_sub(lines.len() as u16 / 2);
    let rect = Rect { x: area.x, y: area.y + y0, width: area.width, height: lines.len() as u16 };
    f.render_widget(Paragraph::new(lines).centered(), rect);
}

fn draw_picker(f: &mut Frame, area: Rect, app: &mut App) {
    let t = app.theme;
    let h = area.height.saturating_sub(1).max(1) as usize;
    if app.picker_cursor < app.picker_scroll {
        app.picker_scroll = app.picker_cursor;
    }
    if app.picker_cursor >= app.picker_scroll + h {
        app.picker_scroll = app.picker_cursor + 1 - h;
    }
    let mut lines = Vec::new();
    for (i, p) in app.picker.iter().enumerate().skip(app.picker_scroll).take(h) {
        let is_dir = p.is_dir();
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| p.display().to_string());
        let label = if is_dir { format!("{name}/") } else { name };
        if i == app.picker_cursor {
            lines.push(Line::styled(
                format!("> {label}"),
                Style::new().bg(t.accent).fg(t.cursor_fg),
            ));
        } else {
            lines.push(Line::styled(format!("  {label}"), Style::new().fg(t.fg)));
        }
    }
    f.render_widget(
        Paragraph::new(lines),
        Rect { x: area.x, y: area.y, width: area.width, height: h as u16 },
    );
    let line = statusline::build(app, area.width as usize);
    f.render_widget(
        Paragraph::new(line),
        Rect { x: area.x, y: area.y + area.height - 1, width: area.width, height: 1 },
    );
}
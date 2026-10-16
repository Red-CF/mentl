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
    // filas: [tabs?] [editor(+sidebar col, +panel)] [statusline]
    let y = area.y;
    let tabs_h: u16 = if app.config.ui.tabs { 1 } else { 0 };
    let editor_top = y + tabs_h;
    let status_y = area.y + area.height - 1;
    let editor_h = status_y - editor_top;
    let panel_h: u16 = if app.problems_open {
        app.config.ui.panel_height.min(editor_h.saturating_sub(4))
    } else {
        0
    };
    let pane_h = editor_h - panel_h;
    let sidebar_w: u16 = if app.sidebar.is_some() {
        app.config.ui.sidebar_width.min(area.width.saturating_sub(20))
    } else {
        0
    };

    // pane tree dentro del editor rect (solo filas de panes, panel fuera)
    let editor = Rect {
        x: area.x + sidebar_w,
        y: editor_top,
        width: area.width - sidebar_w,
        height: pane_h,
    };
    app.last_editor_rect = editor;
    app.last_view_h = pane_h as usize;
    app.follow_cursor();

    // tabs bar
    if tabs_h == 1 {
        let tabs_rect = Rect { x: area.x, y: area.y, width: area.width, height: 1 };
        draw_tabs(f, tabs_rect, app);
    }

    // sidebar
    if sidebar_w > 0 {
        let sb_rect = Rect { x: area.x, y: editor_top, width: sidebar_w, height: pane_h };
        draw_sidebar(f, sb_rect, app);
    }

    // panes
    let mut rects = Vec::new();
    let mut seps = Vec::new();
    app.panes.layout(editor, &mut rects, &mut seps);

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
        let path = app.buffers[buffer_idx].as_ref().and_then(|b| b.path.clone());
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
            app.buffers[buffer_idx].as_ref().expect("buffer vivo"),
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

    // problems panel (bajo los panes, sobre statusline)
    if panel_h > 0 {
        let panel_rect = Rect { x: area.x, y: editor_top + pane_h, width: area.width, height: panel_h };
        draw_problems(f, panel_rect, app);
    }

    // statusline
    let st_rect = Rect { x: area.x, y: status_y, width: area.width, height: 1 };
    let line = statusline::build(app, area.width as usize);
    f.render_widget(Paragraph::new(line), st_rect);

    // completions popup bajo el cursor del pane activo
    if app.completion.is_some() && cursor_pos.is_some() {
        let (cx, cy) = cursor_pos.unwrap();
        let pane_rect = app
            .pane_rects()
            .into_iter()
            .find(|(p, _)| *p == app.panes.active)
            .map(|(_, r)| r)
            .unwrap_or(editor);
        draw_completion(f, pane_rect, cx, cy, app);
    }

    // overlays último (encima de todo)
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

// -------------------------------------------------------------- tabs bar ---

fn draw_tabs(f: &mut Frame, rect: Rect, app: &mut App) {
    let t = app.theme;
    let active = app.active_buffer();
    let labels: Vec<(String, bool)> = app
        .buffers
        .iter()
        .flatten()
        .enumerate()
        .map(|(i, b)| {
            let dot = if b.modified && b.is_persistent() { " •" } else { "" };
            (format!("[{}]{} ", b.display_name(), dot), i == active)
        })
        .collect();
    // truncado por la izquierda: la tab activa SIEMPRE visible
    let mut line = String::new();
    for (label, _) in &labels {
        line.push_str(label);
    }
    if line.chars().count() > rect.width as usize {
        // recorta: mantén el rango alrededor de la activa — MVP: suffix
        let keep: String = labels
            .iter()
            .skip_while(|(_l, a)| !*a)
            .chain(labels.iter().take_while(|(_, a)| !*a))
            .map(|(l, _)| l.clone())
            .take(0)
            .collect();
        let _ = keep;
        // versión sencilla: solo activa + prev
        line = format!("…{}", labels[active].0);
    }
    f.render_widget(
        Paragraph::new(line).style(Style::new().fg(t.gutter).bg(t.status_bg)),
        rect,
    );
    // resalta la activa re-renderizando su label con accent bg
    let mut x = rect.x as usize;
    for (label, is_active) in &labels {
        let w = label.chars().count();
        if *is_active && x + w <= rect.x as usize + rect.width as usize {
            f.render_widget(
                Paragraph::new(label.clone()).style(Style::new().bg(t.accent).fg(t.cursor_fg)),
                Rect { x: x as u16, y: rect.y, width: w as u16, height: 1 },
            );
        }
        x += w;
    }
}

// -------------------------------------------------------------- sidebar ----

fn draw_sidebar(f: &mut Frame, rect: Rect, app: &mut App) {
    let t = app.theme;
    let Some(sb) = app.sidebar.as_mut() else { return };
    let rows_h = rect.height as usize;
    crate::explorer::sync_scroll(sb.cursor, &mut sb.scroll, rows_h);
    let mut lines = Vec::new();
    // severidad por archivo (min) para el marker trailing
    let mut i = sb.scroll;
    while i < sb.rows.len() && lines.len() < rows_h {
        let row = &sb.rows[i];
        let indent = (row.depth as usize) * 2;
        let glyph = if row.is_dir {
            if sb.expanded.get(&row.path).copied().unwrap_or(false) { "▾ " } else { "▸ " }
        } else {
            "  "
        };
        let name = row
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| row.path.display().to_string());
        let label = format!("{}{}{}{name}", " ".repeat(indent), glyph, "");
        let is_cursor = i == sb.cursor && app.sidebar_focus;
        // diag severity del archivo (min = más grave)
        let sev = app
            .diagnostics
            .get(&row.path)
            .map(|ds| ds.iter().map(|d| d.severity).min())
            .flatten();
        let style = if is_cursor {
            Style::new().bg(t.accent).fg(t.cursor_fg)
        } else if !row.is_dir && sev.is_some() {
            match sev.unwrap() {
                1 => Style::new().fg(t.diag_error),
                2 => Style::new().fg(t.diag_warn),
                _ => Style::new().fg(t.fg),
            }
        } else if row.is_dir {
            Style::new().fg(t.accent)
        } else {
            Style::new().fg(t.fg)
        };
        lines.push(Line::styled(label, style));
        i += 1;
    }
    f.render_widget(Paragraph::new(lines), rect);
}

// -------------------------------------------------------------- problems ---

fn draw_problems(f: &mut Frame, rect: Rect, app: &mut App) {
    let t = app.theme;
    let block = Block::default()
        .borders(ratatui::widgets::Borders::TOP)
        .border_style(Style::new().fg(t.gutter))
        .style(Style::new().bg(t.status_bg));
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let rows = app.problem_rows();
    let rows_h = inner.height as usize;
    crate::explorer::sync_scroll(app.problems_cursor, &mut app.problems_scroll, rows_h);
    let mut lines = Vec::new();
    let mut count = 0;
    for (i, p) in rows.iter().enumerate().skip(app.problems_scroll) {
        if count >= rows_h {
            break;
        }
        let name = p.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let label = format!(" {}:{}:{} {} {} ", name, p.line_display + 1, p.col_u16,
            match p.severity { 1 => "E", 2 => "W", _ => "i" }, p.msg);
        let style = if app.problems_focus && i == app.problems_cursor {
            Style::new().bg(t.accent).fg(t.cursor_fg)
        } else {
            match p.severity {
                1 => Style::new().bg(t.status_bg).fg(t.diag_error),
                2 => Style::new().bg(t.status_bg).fg(t.diag_warn),
                _ => Style::new().bg(t.status_bg).fg(t.fg),
            }
        };
        lines.push(Line::styled(label, style));
        count += 1;
    }
    if rows.is_empty() {
        lines.push(Line::styled(" sin problemas ✓", Style::new().bg(t.status_bg).fg(t.gutter)));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

// ------------------------------------------------------------ completión ---

fn draw_completion(f: &mut Frame, pane: Rect, cursor_x: u16, cursor_y: u16, app: &mut App) {
    let Some(p) = app.completion.as_ref() else { return };
    let t = app.theme;
    let max_w = pane.width.saturating_sub(1);
    let width = p
        .items
        .iter()
        .map(|i| i.label.chars().count() as u16)
        .max()
        .unwrap_or(1)
        .clamp(8, max_w.max(8));
    // height = items + 2 borders; bajo el cursor; si no cabe, arriba
    let raw_h = (p.filtered.len() as u16).saturating_add(2).min(10);
    let below = pane.y + pane.height - cursor_y;
    let (y, h) = if below > raw_h {
        (cursor_y + 1, raw_h)
    } else if cursor_y >= raw_h && cursor_y >= pane.y + raw_h {
        (cursor_y - raw_h, raw_h) // encima del cursor
    } else {
        (pane.y, raw_h.min(pane.height))
    };
    // x: a la derecha del cursor, clamp al pane
    let x = (cursor_x).min(pane.x + pane.width.saturating_sub(width + 1)).max(pane.x);
    let rect = Rect { x, y, width: width.min(max_w), height: h };
    let block = Block::default()
        .borders(ratatui::widgets::Borders::ALL)
        .border_style(Style::new().fg(t.gutter_active))
        .style(Style::new().bg(t.bg));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let start = app.completion.as_ref().unwrap();
    let mut lines = Vec::new();
    for &idx in start.filtered.iter().skip(if start.cursor >= inner.height as usize { start.cursor + 1 - inner.height as usize } else { 0 }).take(inner.height as usize) {
        let item = &start.items[idx];
        let pos = start.filtered.iter().position(|&f| f == idx).unwrap_or(0);
        let style = if pos == start.cursor {
            Style::new().bg(t.accent).fg(t.cursor_fg)
        } else {
            Style::new().fg(t.fg)
        };
        lines.push(Line::styled(item.label.clone(), style));
    }
    f.render_widget(Paragraph::new(lines), inner);
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
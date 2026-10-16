//! Render: welcome hub (menú + anim), editor (tabs/sidebar/panes/problems/
//! popups), statusline y overlays. draw muta &mut App solo para lo visual.

use crate::app::{App, Screen};
use crate::app::find_all;
use crate::buffer::Buffer;
use crate::keymap::Mode;
use crate::panes::Dir;
use crate::statusline;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Wrap};
use ratatui::Frame;
use unicode_segmentation::UnicodeSegmentation;

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    f.render_widget(Block::default().style(Style::new().bg(app.theme.bg)), area);
    match app.screen {
        Screen::Welcome => draw_welcome(f, area, app),
        Screen::Editor => draw_editor(f, area, app),
    }
}

// ================================================================= welcome ==

fn draw_welcome(f: &mut Frame, area: Rect, app: &mut App) {
    let t = app.theme;
    let lines_n = 14u16;
    let y0 = area.height.saturating_sub(lines_n) / 2;
    let top = Rect { x: area.x, y: area.y + y0, width: area.width, height: lines_n };
    let inner = Rect { x: top.x + 4, y: top.y + 1, width: top.width.saturating_sub(8), height: lines_n - 2 };

    // logo animado: cada letra cicla el color con el frame
    let logo = "◆ MENTL ◆";
    let mut logo_spans = Vec::new();
    for (i, c) in logo.chars().enumerate() {
        let phase = (app.anim_frame as usize + i) % 3;
        let color = match phase {
            0 => t.accent,
            1 => t.diag_warn,
            _ => t.fg,
        };
        logo_spans.push(Span::styled(c.to_string(), Style::new().fg(color).bold()));
        logo_spans.push(Span::raw(" "));
    }
    let mut rows = vec![Line::from(logo_spans)]; // logo espaciado

    let animating = app.welcome_input.is_none();

    rows.push(Line::styled("", Style::new()));
    if let Some(inp) = &app.welcome_input {
        rows.push(Line::styled(
            format!("ruta: {inp}▏"),
            Style::new().fg(t.accent),
        ));
        rows.push(Line::styled("Enter abre · Esc cancela", Style::new().fg(t.gutter)));
    } else if animating {
        for (i, (label, hint)) in App::MENU.iter().enumerate() {
            let sel = i == app.welcome_cursor;
            let style = if sel {
                Style::new().bg(t.accent).fg(t.cursor_fg)
            } else if i % 2 == 0 {
                Style::new().fg(t.fg)
            } else {
                Style::new().fg(t.gutter)
            };
            let cursor = if sel { "❯ " } else { "  " };
            let hint_s = if hint.is_empty() {
                String::new()
            } else {
                format!(" · {hint}")
            };
            rows.push(Line::styled(format!("{cursor}{label}{hint_s}"), style));
        }
    }
    rows.push(Line::styled("", Style::new()));
    if let Some(e) = app.live_msg() {
        rows.push(Line::styled(format!("✖ {e}"), Style::new().fg(t.diag_error)));
    }
    rows.push(Line::styled(
        "j/k mueve · Enter elige · q sale",
        Style::new().fg(t.gutter),
    ));
    rows.push(Line::styled("config: ~/.config/mentl/config.toml", Style::new().fg(t.gutter)));

    f.render_widget(Paragraph::new(rows).centered(), inner);
}

// ================================================================= editor ===

fn draw_editor(f: &mut Frame, area: Rect, app: &mut App) {
    // filas: [tabs?] [editor(+sidebar col, +panel)] [statusline]
    let tabs_h: u16 = if app.config.ui.tabs { 1 } else { 0 };
    let editor_top = area.y + tabs_h;
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

    // pane tree dentro del editor rect (panel fuera)
    let editor = Rect {
        x: area.x + sidebar_w,
        y: editor_top,
        width: area.width - sidebar_w,
        height: pane_h,
    };
    app.last_editor_rect = editor;
    app.last_view_h = pane_h as usize;
    app.follow_cursor();

    // fondo de la columna sidebar (tintado, distinto del editor)
    if sidebar_w > 0 {
        let sb_bg = Rect { x: area.x, y: editor_top, width: sidebar_w, height: pane_h };
        let t = app.theme;
        f.render_widget(
            Block::default().style(Style::new().bg(t.sidebar_bg)),
            sb_bg,
        );
    }

    // tabs bar
    if tabs_h == 1 {
        let tabs_rect = Rect { x: area.x, y: area.y, width: area.width, height: 1 };
        draw_tabs(f, tabs_rect, app);
    }

    // sidebar (árbol)
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
    // selección visual (buffer activo) normalizada
    let sel_norm: Option<((usize, usize), (usize, usize), bool)> = {
        let head = app.buffers[app.active_buffer()].as_ref().map(|b| b.cursor);
        match (app.selection, head) {
            (Some(s), Some(h)) if app.mode == Mode::Visual => {
                let (a, b) = s.anchor;
                if (a, b) <= h {
                    Some(((a, b), h, s.kind == crate::buffer::SelKind::Line))
                } else {
                    Some((h, (a, b), s.kind == crate::buffer::SelKind::Line))
                }
            }
            _ => None,
        }
    };
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
        // severidad por línea (min = más grave): 1 error > 2 warn > resto
        let path = app.buffers[buffer_idx].as_ref().and_then(|b| b.path.clone());
        let mut diag_by_line: std::collections::HashMap<usize, u32> =
            std::collections::HashMap::new();
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
            sel_norm.filter(|_| buffer_idx == app.active_buffer()),
            &mut cursor_pos,
        );
    }

    // problems panel (bajo panes, sobre statusline)
    if panel_h > 0 {
        let panel_rect = Rect {
            x: area.x,
            y: editor_top + pane_h,
            width: area.width,
            height: panel_h,
        };
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

    // overlays último
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

// ------------------------------------------------------------------ tabs ----

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
    let mut line = String::new();
    for (label, _) in &labels {
        line.push_str(label);
    }
    if line.chars().count() > rect.width as usize {
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

// --------------------------------------------------------------- sidebar ----

fn draw_sidebar(f: &mut Frame, rect: Rect, app: &mut App) {
    let t = app.theme;
    let Some(sb) = app.sidebar.as_mut() else { return };
    // header: nombre del proyecto
    let proj = sb
        .root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| sb.root.display().to_string());
    let header_rect = Rect { x: rect.x + 1, y: rect.y, width: rect.width - 1, height: 1 };
    f.render_widget(
        Paragraph::new(Line::styled(
            format!("◆ {proj}"),
            Style::new().fg(t.accent).bold(),
        )),
        header_rect,
    );
    let body = Rect {
        x: rect.x,
        y: rect.y + 1,
        width: rect.width,
        height: rect.height.saturating_sub(1),
    };

    let rows_h = body.height as usize;
    crate::explorer::sync_scroll(sb.cursor, &mut sb.scroll, rows_h);
    let mut lines = Vec::new();
    let mut i = sb.scroll;
    while i < sb.rows.len() && lines.len() < rows_h {
        let row = &sb.rows[i];
        let indent = "│ ".repeat(row.depth as usize);
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
        let label = format!(" {indent}{glyph}{name}");
        let sel = i == sb.cursor && app.sidebar_focus;
        let sev = app
            .diagnostics
            .get(&row.path)
            .map(|ds| ds.iter().map(|d| d.severity).min())
            .flatten();
        let style = if sel {
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
    f.render_widget(Paragraph::new(lines), body);
}

// ---------------------------------------------------------------- problems --

fn draw_problems(f: &mut Frame, rect: Rect, app: &mut App) {
    let t = app.theme;
    let rows = app.problem_rows();
    let (err, warn) = (
        rows.iter().filter(|p| p.severity == 1).count(),
        rows.iter().filter(|p| p.severity == 2).count(),
    );
    let title = if err + warn == 0 {
        "PROBLEMAS — sin problemas ✓".to_string()
    } else {
        format!("PROBLEMAS — {err} error(es), {warn} warn")
    };
    let block = Block::default()
        .borders(ratatui::widgets::Borders::TOP)
        .border_style(Style::new().fg(t.gutter))
        .style(Style::new().bg(t.status_bg))
        .title(Line::styled(format!(" {title} "), Style::new().fg(t.accent)));
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let rows_h = inner.height as usize;
    crate::explorer::sync_scroll(app.problems_cursor, &mut app.problems_scroll, rows_h);
    let mut lines = Vec::new();
    let mut count = 0;
    for (i, p) in rows.iter().enumerate().skip(app.problems_scroll) {
        if count >= rows_h {
            break;
        }
        let name = p
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let icon = match p.severity {
            1 => "✕",
            2 => "⚠",
            _ => "•",
        };
        let label = format!(" {icon} {name}:{}:{} {}", p.line_display + 1, p.col_u16, p.msg);
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
    f.render_widget(Paragraph::new(lines), inner);
}

// ------------------------------------------------------------- completion ---

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
    // height = items + 2 borders
    let raw_h = (p.filtered.len() as u16).saturating_add(2).min(10);
    let below = pane.y + pane.height - cursor_y;
    let (y, h) = if below > raw_h {
        (cursor_y + 1, raw_h)
    } else if cursor_y >= raw_h && cursor_y >= pane.y + raw_h {
        (cursor_y - raw_h, raw_h) // encima del cursor
    } else {
        (pane.y, raw_h.min(pane.height))
    };
    let x = cursor_x
        .min(pane.x + pane.width.saturating_sub(width + 1))
        .max(pane.x);
    let rect = Rect { x, y, width: width.min(max_w), height: h };
    let block = Block::default()
        .borders(ratatui::widgets::Borders::ALL)
        .border_style(Style::new().fg(t.gutter_active))
        .style(Style::new().bg(t.bg));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let Some(st) = app.completion.as_ref() else { return };
    let start_row = if st.cursor >= inner.height as usize {
        st.cursor + 1 - inner.height as usize
    } else {
        0
    };
    let mut lines = Vec::new();
    for &idx in st.filtered.iter().skip(start_row).take(inner.height as usize) {
        let item = &st.items[idx];
        let pos = st.filtered.iter().position(|&f2| f2 == idx).unwrap_or(0);
        let style = if pos == st.cursor {
            Style::new().bg(t.accent).fg(t.cursor_fg)
        } else {
            Style::new().fg(t.fg)
        };
        lines.push(Line::styled(item.label.clone(), style));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

// -------------------------------------------------------------------- pane --

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
    sel: Option<((usize, usize), (usize, usize), bool)>,
    cursor_pos: &mut Option<(u16, u16)>,
) {
    let t = theme;
    let len = buf.len_lines();
    let gutter_w = (len.to_string().len()).max(2) + 1;
    let cursor = buf.cursor;

    // gutter
    let mut gut: Vec<Line> = Vec::new();
    for i in scroll..(scroll + rect.height as usize).min(len) {
        let (sep_char, sep_style) = match diag_by_line.get(&i) {
            Some(1) => ("!", Style::new().fg(t.diag_error)),
            Some(2) => ("!", Style::new().fg(t.diag_warn)),
            Some(_) => ("i", Style::new().fg(t.gutter)),
            None => (" ", Style::new()),
        };
        let base = if i == cursor.0 { t.gutter_active } else { t.gutter };
        let style = match diag_by_line.get(&i) {
            Some(1) | Some(2) => sep_style,
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
        let sel_mark = sel.and_then(|(min, max, line_kind)| {
            sel_bytes_for_line(&text, i, min, max, line_kind, cursor.0)
        });
        rows.push(render_line(&text, cursor_byte, &matches, sel_mark, &t));
    }
    let text_rect = Rect {
        x: rect.x + gutter_w as u16,
        y: rect.y,
        width: rect.width.saturating_sub(gutter_w as u16),
        height: rect.height,
    };
    f.render_widget(Paragraph::new(rows), text_rect);

    // cursor terminal (pane activo)
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

fn display_width(seg: &str, tab_w: usize) -> usize {
    seg.graphemes(true)
        .map(|g| if g == "\t" { tab_w } else { unicode_width::UnicodeWidthStr::width(g) })
        .sum()
}


/// Byte range de la selección en esta línea (con límites correctos por línea).
fn sel_bytes_for_line(
    text: &str,
    line: usize,
    min: (usize, usize),
    max: (usize, usize),
    line_kind: bool,
    _cur_line: usize,
) -> Option<(usize, usize)> {
    if line_kind {
        if line >= min.0 && line <= max.0 {
            return Some((0, text.len()));
        }
        return None;
    }
    if line < min.0 || line > max.0 {
        return None;
    }
    let a = if line == min.0 {
        Buffer::grapheme_byte_of(text, min.1)
    } else {
        0
    };
    let b = if line == max.0 {
        Buffer::grapheme_byte_of(text, max.1)
    } else {
        text.len()
    };
    Some((a.min(b), a.max(b)))
}

/// Spans de línea con estilo por segmentos: normal > match > cursor.
/// Cubre TODA la línea (0..len).
fn render_line(
    text: &str,
    cursor_byte: Option<(usize, Mode)>,
    matches: &[(usize, usize)],
    sel: Option<(usize, usize)>,
    t: &crate::theme::Theme,
) -> Line<'static> {
    if text.is_empty() {
        return match cursor_byte {
            Some((0, Mode::Normal)) => {
                Line::raw(" ").style(Style::new().bg(t.accent).fg(t.cursor_fg))
            }
            Some((0, Mode::Insert)) => Line::raw(" ")
                .style(Style::new().underlined().fg(t.fg).underline_color(t.accent)),
            _ => Line::styled("", Style::new().fg(t.fg)),
        };
    }
    let cursor_range = cursor_byte.map(|(cb, m)| {
        let next = text[cb..].graphemes(true).next().map(|g| g.len()).unwrap_or(0);
        (cb, (cb + next).min(text.len()), m)
    });
    let mut marks: Vec<(usize, usize, u8)> = Vec::new(); // 0 sel, 1 cursor
    if let Some((s, e, _)) = cursor_range {
        marks.push((s, e, 1));
    }
    for (s, e) in matches {
        marks.push((*s, *e, 0));
    }
    if let Some((s, e)) = sel {
        marks.push((s, e, 0)); // mismo peso que match — el estilo difiere
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
        let cursor_mode = cursor_range
            .filter(|(s, e, _)| a >= *s && a < *e)
            .map(|(_, _, m)| m);
        let is_match = matches.iter().any(|(s, e)| a >= *s && a < *e);
        let is_sel = sel.is_some_and(|(s, e)| a >= s && a < e);
        let piece = text.get(a..b).unwrap_or("").to_string();
        let style = match cursor_mode {
            Some(Mode::Normal) => Style::new().bg(t.accent).fg(t.cursor_fg),
            Some(Mode::Insert) => {
                Style::new().underlined().fg(t.fg).underline_color(t.accent)
            }
            _ if is_sel => Style::new().bg(t.visual_bg).fg(t.fg),
            _ if is_match => Style::new().bg(t.match_bg).fg(t.cursor_fg),
            _ => Style::new().fg(t.fg),
        };
        spans.push(Span::styled(piece, style));
    }
    Line::from(spans)
}

// ----------------------------------------------------------------- hover ----

fn draw_hover(f: &mut Frame, area: Rect, app: &mut App) {
    let Some(hover) = &app.hover else { return };
    let t = app.theme;
    let w = ((area.width as f32 * 0.5) as u16).min(70).max(30);
    let h = 10.min(area.height.saturating_sub(2));
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
        Paragraph::new(hover.clone()).wrap(Wrap { trim: false }),
        inner,
    );
}

// ----------------------------------------------------------------- finder ---

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
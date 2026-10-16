//! Render. draw muta &mut App solo para: actualizar altura de viewport
//! conocida y decay de mensajes. El estado de texto nunca cambia aquí.

use crate::app::{App, Screen};
use crate::app::find_all;
use crate::buffer::Buffer;
use crate::keymap::Mode;
use crate::statusline;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;
use unicode_segmentation::UnicodeSegmentation;

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    // fondo global
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
    app.last_view_h = h;
    app.follow_cursor();
    let cursor = app.buffer().cursor;
    let len = app.buffer().len_lines();
    let scroll = app.view.scroll;
    let gutter_w = (len.to_string().len()).max(2) + 1;

    // gutter
    let mut gut: Vec<Line> = Vec::new();
    for i in scroll..(scroll + h).min(len) {
        let style = if i == cursor.0 {
            Style::new().fg(app.theme.gutter_active)
        } else {
            Style::new().fg(app.theme.gutter)
        };
        gut.push(Line::styled(format!("{:>width$}", i + 1, width = gutter_w - 1) + " ", style));
    }
    let gut_rect = Rect { x: area.x, y: area.y, width: gutter_w as u16, height: h as u16 };
    f.render_widget(Paragraph::new(gut), gut_rect);

    // texto con spans (cursor + matches de búsqueda)
    let mut rows: Vec<Line> = Vec::new();
    let pat = app.search.clone();
    let smart = pat.as_ref().is_some_and(|p| p.chars().any(|c| c.is_uppercase()));
    for i in scroll..(scroll + h).min(len) {
        let text = app.buffer().line_text(i);
        let cursor_byte = if i == cursor.0 && app.mode == Mode::Normal {
            let byte = Buffer::grapheme_byte_of(&text, cursor.1);
            Some(byte)
        } else {
            None
        };
        let matches = pat
            .as_ref()
            .map(|p| find_all(&text, p, smart))
            .unwrap_or_default();
        rows.push(render_line(&text, cursor_byte, &matches, app));
    }
    let text_rect = Rect {
        x: area.x + gutter_w as u16,
        y: area.y,
        width: area.width.saturating_sub(gutter_w as u16),
        height: h as u16,
    };
    f.render_widget(Paragraph::new(rows), text_rect);

    // statusline
    let st_rect = Rect {
        x: area.x,
        y: area.y + area.height - 1,
        width: area.width,
        height: 1,
    };
    let line = statusline::build(app, area.width as usize);
    f.render_widget(Paragraph::new(line), st_rect);
}

/// Spans de una línea con estilo por segmentos: cursor > match > normal.
fn render_line(
    text: &str,
    cursor_byte: Option<usize>,
    matches: &[(usize, usize)],
    app: &App,
) -> Line<'static> {
    let t = app.theme;
    if text.is_empty() && cursor_byte.is_none() {
        return Line::styled("", Style::new().fg(t.fg));
    }
    let cursor_range = cursor_byte.map(|cb| {
        let next = text[cb..].graphemes(true).next().map(|g| g.len()).unwrap_or(0);
        (cb, (cb + next).min(text.len()))
    });
    let mut marks: Vec<(usize, usize, bool)> = Vec::new();
    if let Some((s, e)) = cursor_range {
        marks.push((s, e, true));
    }
    for (s, e) in matches {
        marks.push((*s, *e, false));
    }
    // segmentos elementales: cubren TODA la línea (0..len)
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
        let is_cursor = cursor_range.is_some_and(|(s, e)| a >= s && a < e);
        let is_match = matches.iter().any(|(s, e)| a >= *s && a < *e);
        let piece = text.get(a..b).unwrap_or("").to_string();
        let style = if is_cursor {
            Style::new().bg(t.accent).fg(t.cursor_fg)
        } else if is_match {
            Style::new().bg(t.match_bg).fg(t.cursor_fg)
        } else {
            Style::new().fg(t.fg)
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
        Line::styled("MENTL v0.1.0", Style::new().fg(t.accent).bold()),
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
    // auto-scroll del picker
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
            lines.push(Line::styled(format!("> {label}"), Style::new().bg(t.accent).fg(t.cursor_fg)));
        } else {
            lines.push(Line::styled(format!("  {label}"), Style::new().fg(t.fg)));
        }
    }
    f.render_widget(Paragraph::new(lines), Rect { x: area.x, y: area.y, width: area.width, height: h as u16 });
    let line = statusline::build(app, area.width as usize);
    f.render_widget(
        Paragraph::new(line),
        Rect { x: area.x, y: area.y + area.height - 1, width: area.width, height: 1 },
    );
}
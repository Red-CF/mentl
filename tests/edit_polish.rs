//! fase 4: auto-close, autoindent, indent-detection.

use crossterm::event::KeyCode;
use mentl::app::App;
use mentl::buffer::IndentKind;
use mentl::input::InputKey;
use std::path::PathBuf;

fn key(s: &str) -> InputKey {
    InputKey::parse(s).unwrap()
}

fn k(code: KeyCode) -> InputKey {
    InputKey::simple(code)
}

fn app_with(name: &str, content: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("mentl-pl-{name}-{}.rs", std::process::id()));
    std::fs::write(&p, content).unwrap();
    p
}

#[test]
fn auto_close_par_y_undo_un_accion() {
    let p = app_with("par", "");
    let mut app = App::new(Some(p.clone()));
    app.on_key(key("i"));
    app.on_key(k(KeyCode::Char('(')));
    assert_eq!(app.buffer().line_text(0), "()", "par completo");
    assert_eq!(app.buffer().cursor, (0, 1), "cursor entre el par");
    // 1 undo = el par entero
    app.on_key(k(KeyCode::Esc));
    app.on_key(key("u"));
    assert_eq!(app.buffer().line_text(0), "", "el par se deshace junto");
    drop(app);
}

#[test]
fn skip_over_closer_sin_bump_rev() {
    let p = app_with("skip", "fn ()\n");
    let mut app = App::new(Some(p));
    app.on_key(key("$")); // cursor al final de línea 0
    // el siguiente grafo es \n; sitúese antes del ')': movemos manualmente
    app.buffer().to(0, 3); // f = 'f'; entre ( y 0... texto "fn ()": g3=' '
    app.on_key(key("i"));
    let r0 = app.buffer().rev;
    app.on_key(k(KeyCode::Char(')')));
    assert_eq!(app.buffer().rev, r0, "skip-over no muta (solo mv)");
    assert_eq!(app.buffer().cursor.1, 4, "sobre el )");
    drop(app);
}

#[test]
fn autoindent_herede_y_sube_con_llave() {
    let p = app_with("ind", "");
    let mut app = App::new(Some(p));
    app.on_key(key("i"));
    app.on_key(k(KeyCode::Char('{')));
    app.on_key(k(KeyCode::Enter)); // "a\n{"?
    // texto: '{' insertado → línea "x{"? — empezamos vacío: line0 "{"
    // tras enter: lead "" + unit por trailing {
    let t0 = app.buffer().line_text(1);
    assert_eq!(t0, "    ", "1 unidad con abridor");
    drop(app);
}

#[test]
fn indent_detection_tabs_y_spaces() {
    let p = app_with("tabs", "\tone\n\t\ttwo\n");
    let mut app = App::new(Some(p));
    assert_eq!(app.buffer().indent, IndentKind::Tabs);
    let p2 = app_with("sp2", "  a\n  b\n");
    let mut app2 = App::new(Some(p2));
    assert_eq!(app2.buffer().indent, IndentKind::Spaces(2));
    let p3 = app_with("mixed", "\ta\n        b\n");
    let mut app3 = App::new(Some(p3));
    // mayoría entre tabs(1) y espacios8(1): empate → tabs
    assert_eq!(app3.buffer().indent, IndentKind::Tabs);
    drop(app);
}

#[test]
fn tab_inserta_unidad_detectada() {
    let p = app_with("tabins", "\tx\n");
    let mut app = App::new(Some(p));
    app.on_key(key("i"));
    app.on_key(k(KeyCode::Tab));
    assert_eq!(
        app.buffer().line_text(0),
        "\t\tx",
        "inserta \t, no 4 espacios"
    );
    drop(app);
}

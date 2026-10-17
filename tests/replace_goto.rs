//! fase 4: replace-all, goto-line, bracket, clipboard.

use crossterm::event::KeyCode;
use mentl::app::App;
use mentl::input::InputKey;
use std::path::PathBuf;

fn key(s: &str) -> InputKey {
    InputKey::parse(s).unwrap()
}

fn k(code: KeyCode) -> InputKey {
    InputKey::simple(code)
}

fn app_with(name: &str, content: &str) -> (PathBuf, App) {
    let p = std::env::temp_dir().join(format!("mentl-rp-{name}-{}.rs", std::process::id()));
    std::fs::write(&p, content).unwrap();
    let app = App::new(Some(p.clone()));
    (p, app)
}

#[test]
fn replace_all_cuenta_y_un_undo() {
    let (p, mut app) = app_with("rep", "x foo x\nfoo\n  foo bar foo\n");
    app.on_key(key("space"));
    app.on_key(key("r"));
    // stage 0: patrón
    for c in "foo".chars() {
        app.on_key(k(KeyCode::Char(c)));
    }
    app.on_key(k(KeyCode::Enter));
    // stage 1: reemplazo
    for c in "quux".chars() {
        app.on_key(k(KeyCode::Char(c)));
    }
    app.on_key(k(KeyCode::Enter));
    assert_eq!(app.buffer().line_text(0), "x quux x");
    assert_eq!(app.buffer().line_text(1), "quux");
    assert!(!app.buffer().line_text(2).contains("foo"));
    // UN undo restaura todo (lote compartido)
    app.on_key(key("u"));
    assert_eq!(app.buffer().line_text(1), "foo");
    drop(app);
    let _ = std::fs::remove_file(p);
}

#[test]
fn replace_smart_case_y_vacio() {
    let (p, mut app) = app_with("sm", "Foo foo FOO\n");
    app.on_key(key("space"));
    app.on_key(key("r"));
    for c in "foo".chars() {
        app.on_key(k(KeyCode::Char(c)));
    }
    app.on_key(k(KeyCode::Enter));
    for c in "z".chars() {
        app.on_key(k(KeyCode::Char(c)));
    }
    app.on_key(k(KeyCode::Enter));
    assert_eq!(
        app.buffer().line_text(0),
        "z z z",
        "case-insensitive (pattern sin mayús)"
    );
    // patrón vacío: no-op
    app.on_key(key("space"));
    app.on_key(key("r"));
    app.on_key(k(KeyCode::Enter)); // enter con pattern vacío aborta
    assert_eq!(app.buffer().line_text(0), "z z z");
    drop(app);
    let _ = std::fs::remove_file(p);
}

#[test]
fn goto_line_abs_relativo_y_pct() {
    let body: String = (0..100).map(|i| format!("L{i}\n")).collect();
    let (p, mut app) = app_with("goto", &body);
    app.on_key(key("space"));
    app.on_key(key("G")); // prompt goto
    app.goto_input.push_str("42");
    app.on_key(k(KeyCode::Enter));
    assert_eq!(app.buffer().cursor.0, 41, "línea 42 0-based");
    app.on_key(key("space"));
    app.on_key(key("G"));
    app.goto_input.push_str("+3");
    app.on_key(k(KeyCode::Enter));
    assert_eq!(app.buffer().cursor.0, 44);
    app.on_key(key("space"));
    app.on_key(key("G"));
    app.goto_input.push_str("50%");
    app.on_key(k(KeyCode::Enter));
    assert_eq!(app.buffer().cursor.0, 50, "50% de 100 (len_lines-1)");
    // fuera de rango: sin mover
    let before = app.buffer().cursor;
    app.on_key(key("space"));
    app.on_key(key("G"));
    app.goto_input.push_str("9999");
    app.on_key(k(KeyCode::Enter));
    assert_eq!(app.buffer().cursor, before);
    drop(app);
    let _ = std::fs::remove_file(p);
}

#[test]
fn bracket_match_nidificado_multiline() {
    let (p, mut app) = app_with("br", "f(g\n  (x),\n)\n");
    // cursor sobre la 'g'? — sitúese sobre el ( de línea 0 tras "f("
    app.buffer().to(0, 1);
    app.on_key(key("space"));
    app.on_key(InputKey::parse("%").unwrap());
    assert_eq!(app.buffer().cursor.0, 2, "par en línea 2");
    assert_eq!(app.buffer().cursor.1, 0);
    // volver (ahora sobre ')')
    app.on_key(key("space"));
    app.on_key(InputKey::parse("%").unwrap());
    assert_eq!(app.buffer().cursor.0, 0);
    assert_eq!(app.buffer().cursor.1, 1);
    drop(app);
    let _ = std::fs::remove_file(p);
}

#[test]
fn bracket_unbalanceado_no_mueve() {
    let (p, mut app) = app_with("unb", ")([\n");
    app.buffer().to(0, 1);
    let before = app.buffer().cursor;
    app.on_key(key("space"));
    app.on_key(InputKey::parse("%").unwrap());
    assert_eq!(app.buffer().cursor, before, "unbalanceado = silencio");
    drop(app);
    let _ = std::fs::remove_file(p);
}

#[test]
fn clipboard_yank_paste_registro() {
    let (p, mut app) = app_with("clip", "aaa\nbbb\n");
    app.on_key(key("y")); // yank line (chord y y)
    app.on_key(key("y"));
    app.on_key(k(KeyCode::Down));
    app.on_key(key("p")); // paste line-kind debajo
    assert_eq!(app.buffer().line_text(0), "aaa");
    assert_eq!(app.buffer().line_text(1), "bbb");
    assert_eq!(app.buffer().line_text(2), "aaa", "paste debajo");
    drop(app);
    let _ = std::fs::remove_file(p);
}

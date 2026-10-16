//! Smoke: flujo completo de eventos sobre TestBackend — sin pty.

use std::path::PathBuf;
use crossterm::event::KeyCode;
use ratatui::{Terminal, backend::TestBackend};
use mentl::app::App;
use mentl::input::InputKey;
use mentl::draw::draw;

fn key(s: &str) -> InputKey {
    InputKey::parse(s).expect("token de tecla válido en test")
}

fn k(code: KeyCode) -> InputKey {
    InputKey::simple(code)
}

fn sample() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample.txt")
}

fn screen_str(t: &Terminal<TestBackend>) -> String {
    t.backend().buffer().content.iter().map(|c| c.symbol()).collect()
}

#[test]
fn abre_edita_deshace_guarda_sale() {
    // copia del fixture a tmp para no mutar el repo
    let tmp = std::env::temp_dir().join(format!("mentl-smoke-{}.txt", std::process::id()));
    std::fs::copy(sample(), &tmp).expect("copy fixture");
    let tmp_path = tmp.clone();

    let mut app = App::new(Some(tmp_path.clone()));
    assert_eq!(app.screen, mentl::app::Screen::Editor);
    assert_eq!(app.buffer().len_lines(), 4); // 3 líneas + cola

    let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
    term.draw(|f| draw(f, &mut app)).unwrap();

    // insert mode: escribe char + enter
    app.on_key(key("i"));
    app.on_key(k(KeyCode::Char('m')));
    app.on_key(k(KeyCode::Enter));
    app.on_key(k(KeyCode::Esc));
    assert_eq!(app.buffer().line_text(0), "m", "char + enter: línea nueva arriba");

    // undo: desaparece
    app.on_key(key("u"));
    assert_eq!(app.buffer().line_text(0), "hola mentl mundo");

    // redo lo vuelve a poner
    app.on_key(key("C-r"));
    assert_eq!(app.buffer().line_text(0), "m");

    // undo denuevo y guardamos el original
    app.on_key(key("u"));
    app.on_key(key("C-s"));
    assert!(!app.buffer().modified, "guardado = sin cambios pendientes");
    let on_disk = std::fs::read_to_string(&tmp_path).unwrap();
    assert_eq!(on_disk, "hola mentl mundo\nesto es un archivo de prueba\ncon tres líneas\n");

    // quit con leader q (leader = espacio)
    app.on_key(key("space"));
    app.on_key(key("q"));
    assert!(app.quit, "quit limpio tras guardar");
    drop(app);
    let _ = std::fs::remove_file(&tmp);
}

#[test]
fn quit_con_cambios_pide_doble() {
    let tmp = std::env::temp_dir().join(format!("mentl-smoke-mod-{}.txt", std::process::id()));
    std::fs::write(&tmp, "uno\n").unwrap();
    let mut app = App::new(Some(tmp.clone()));
    app.on_key(key("i"));
    app.on_key(k(KeyCode::Char('x')));
    app.on_key(k(KeyCode::Esc));
    assert!(app.buffer().modified);
    app.on_key(key("space"));
    app.on_key(key("q"));
    assert!(!app.quit, "primer quit = aviso");
    app.on_key(key("space"));
    app.on_key(key("q"));
    assert!(app.quit, "segundo quit sale");
    drop(app);
    let _ = std::fs::remove_file(&tmp);
}

#[test]
fn motes_y_cursor_edge_cases() {
    let mut app = App::new(None); // welcome
    app.on_key(k(KeyCode::Char('q')));
    assert!(app.quit, "welcome q sale");
}

#[test]
fn search_smart_case() {
    let mut app = App::new(Some(sample()));
    // baja a la última línea
    app.on_key(key("G"));
    app.on_key(key("/"));
    app.search_input.push_str("hola"); // todo minúsculas -> insensitive
    app.on_key(k(KeyCode::Enter));
    assert_eq!(app.buffer().cursor.0, 0, "wraps a la línea 1");

    app.on_key(key("G"));
    app.on_key(key("/"));
    app.search_input.push_str("HoLa"); // mixed -> case sensitive, no match
    app.on_key(k(KeyCode::Enter));
    // sin match: mensaje, cursor sigue en última línea
    let last = app.buffer().len_lines() - 1;
    assert_eq!(app.buffer().cursor.0, last);
}
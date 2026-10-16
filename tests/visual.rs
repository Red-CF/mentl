//! fase 4: visual mode y ops de rango.

use std::path::PathBuf;
use crossterm::event::KeyCode;
use mentl::app::App;
use mentl::buffer::SelKind;
use mentl::input::InputKey;
use mentl::keymap::Mode;

fn key(s: &str) -> InputKey {
    InputKey::parse(s).expect("token de test")
}

fn k(code: KeyCode) -> InputKey {
    InputKey::simple(code)
}

fn file(name: &str, content: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("mentl-vis-{}-{}", name, std::process::id()));
    std::fs::write(&p, content).unwrap();
    p
}

fn app_with(name: &str, content: &str) -> (PathBuf, App) {
    let p = file(name, content);
    let app = App::new(Some(p.clone()));
    (p, app)
}

#[test]
fn visual_selection_normalizada_y_delete() {
    let (p, mut app) = app_with("sel", "uno\ndos\ntres\n");
    // cursor al final de la línea 0, v, subir: selección hacia atrás (head < anchor)
    app.buffer().to(0, 2); // 'o' de "uno"
    app.on_key(key("v"));
    app.on_key(key("0")); // hacia atrás
    
    assert_eq!(app.mode, Mode::Visual);
    let sel = app.selection.expect("sel");
    assert_eq!(sel.kind, SelKind::Char);
    app.on_key(key("d"));
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.buffer().line_text(0), "o", "selección normalizada hacia atrás");
    assert_eq!(app.buffer().line_text(1), "dos");
    drop(app);
    let _ = std::fs::remove_file(p);
}

#[test]
fn visual_line_comenta_y_unde_hace_una() {
    // caso ADD: todas sin prefix
    let (p, mut app) = app_with("com", "fn a() {}\nlet b = 1;\n");
    app.on_key(key("V"));
    app.on_key(k(KeyCode::Down));
    app.on_key(key("c"));
    assert_eq!(app.buffer().line_text(0), "// fn a() {}");
    assert_eq!(app.buffer().line_text(1), "// let b = 1;");
    app.on_key(key("u"));
    assert_eq!(app.buffer().line_text(0), "fn a() {}", "1 undo restaura");
    assert_eq!(app.buffer().line_text(1), "let b = 1;");
    drop(app);
    let _ = std::fs::remove_file(p);

    // caso STRIP: primera con prefix (líneas sin prefix se quedan)
    let (p2, mut app2) = app_with("comstrip", "// x\ny\n");
    app2.on_key(key("V"));
    app2.on_key(k(KeyCode::Down));
    app2.on_key(key("c"));
    assert_eq!(app2.buffer().line_text(0), "x");
    assert_eq!(app2.buffer().line_text(1), "y", "sin prefix queda igual");
    drop(app2);
    let _ = std::fs::remove_file(p2);
}

#[test]
fn visual_con_emoji_no_rompe_grafos_y_crlf_no_duplica() {
    let (p, mut app) = app_with("emoji", "uno:😀 fin\r\nlínea2\r\n");
    app.on_key(key("v"));
    app.on_key(k(KeyCode::Down)); // char-select de línea 0 completa + inicio de 1
    app.on_key(key("$"));
    app.on_key(key("d"));
    // resultado: la línea 1 quedó vacía? — head $ = línea 1 eof
    assert!(!app.buffer().line_text(0).contains("😀"), "rango borrado completo");
    let joined: Vec<String> = (0..app.buffer().len_lines()).map(|l| app.buffer().line_text(l)).collect();
    assert_eq!(joined.join("|"), "|", "todo borrado");
    drop(app);
    let _ = std::fs::remove_file(p);
}

#[test]
fn indent_visual_y_esc() {
    let (p, mut app) = app_with("ind", "a\nb\nc\n");
    app.on_key(key("V"));
    app.on_key(k(KeyCode::Down));
    app.on_key(key(">"));
    assert_eq!(app.buffer().line_text(0), "    a", "unit default 4");
    assert_eq!(app.buffer().line_text(1), "    b");
    assert_eq!(app.mode, Mode::Normal);
    app.on_key(key("u"));
    assert_eq!(app.buffer().line_text(0), "a");
    drop(app);
    let _ = std::fs::remove_file(p);
}

#[test]
fn keymap_visual_match_sin_panic() {
    // match_seq en Visual sin binds NO parea (protección del indexing)
    let (_p, mut app) = app_with("km", "x\n");
    app.on_key(key("v"));
    app.on_key(key("Q")); // tecla no mapeada en visual
    assert_eq!(app.mode, Mode::Visual);
    // y los motions funcionan via keymap visual
    app.on_key(key("j"));
    assert_eq!(app.buffer().cursor.0, 1);
    drop(app);
}
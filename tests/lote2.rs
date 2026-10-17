//! fase 4 lote 2: mover/sort/format-on-save, git branch, hover scroll.

use crossterm::event::KeyCode;
use mentl::app::App;
use mentl::input::InputKey;
use std::path::{Path, PathBuf};

fn key(s: &str) -> InputKey {
    InputKey::parse(s).unwrap()
}

fn k(code: KeyCode) -> InputKey {
    InputKey::simple(code)
}

fn app_with(name: &str, content: &str) -> (PathBuf, App) {
    let p = std::env::temp_dir().join(format!("mentl-l2-{name}-{}.rs", std::process::id()));
    std::fs::write(&p, content).unwrap();
    let app = App::new(Some(p.clone()));
    (p, app)
}

#[test]
fn move_lines_abajo_arriba() {
    let (p, mut app) = app_with("mv", "uno\ndos\ntres\n");
    app.on_key(key("A-Down")); // mueve línea 0 abajo
    assert_eq!(app.buffer().line_text(0), "dos");
    assert_eq!(app.buffer().line_text(1), "uno");
    app.on_key(key("A-Down"));
    assert_eq!(app.buffer().line_text(1), "tres");
    assert_eq!(app.buffer().line_text(2), "uno", "cursando el final?");
    app.on_key(key("A-Up"));
    assert_eq!(app.buffer().line_text(2), "tres");
    drop(app);
    let _ = std::fs::remove_file(p);
}

#[test]
fn move_lines_bordes_noop() {
    let (p, mut app) = app_with("mvb", "uno\ndos\n");
    app.buffer().file_start();
    app.on_key(key("A-Up")); // primera línea: sin arriba
    assert_eq!(app.buffer().line_text(0), "uno");
    app.buffer().file_end();
    app.on_key(key("A-Down")); // última línea: sin abajo
    assert_eq!(app.buffer().line_text(1), "dos");
    drop(app);
    let _ = std::fs::remove_file(p);
}

#[test]
fn gcc_comenta_la_linea_del_cursor() {
    let (p, mut app) = app_with("gcc", "valor x;\nfn otra() {}\n");
    app.buffer().to(1, 0);
    app.on_key(key("g"));
    app.on_key(key("c"));
    app.on_key(key("c"));
    assert_eq!(app.buffer().line_text(1), "// fn otra() {}");
    assert_eq!(
        app.buffer().line_text(0),
        "valor x;",
        "solo la línea con cursor"
    );
    app.on_key(key("g"));
    app.on_key(key("c"));
    app.on_key(key("c"));
    assert_eq!(app.buffer().line_text(1), "fn otra() {}", "toggle ida");
    drop(app);
    let _ = std::fs::remove_file(p);
}

#[test]
fn hover_scroll_no_cierra_con_jk() {
    let (_p, mut app) = app_with("hov", "x\n");
    app.hover = Some("línea larga de doc\n".repeat(20));
    app.on_key(key("j"));
    assert_eq!(app.hover_scroll, 1, "j enrola");
    assert!(app.hover.is_some(), "hover sigue vivo");
    app.on_key(key("k"));
    assert_eq!(app.hover_scroll, 0);
    app.on_key(k(KeyCode::Esc));
    assert!(app.hover.is_none());
    drop(app);
}

#[test]
fn sort_lines_alfabetico() {
    let (p, mut app) = app_with("sort", "zeta\nalfa\nmango\n");
    app.buffer().sort_lines(0, 2);
    assert_eq!(app.buffer().line_text(0), "alfa");
    assert_eq!(app.buffer().line_text(2), "zeta");
    app.on_key(key("u"));
    assert_eq!(app.buffer().line_text(0), "zeta", "1 undo restaura el sort");
    drop(app);
    let _ = std::fs::remove_file(p);
}

#[test]
fn git_branch_leido_del_head_sin_procesos() {
    let (p, app) = app_with("br", "x\n");
    // el fixture está en /tmp: sin repo → sin branch (silencioso ✓)
    assert!(app.git_branch.is_none());
    drop(app);
    // repo real: mentl — main
    let app2 = App::new(Some(Path::new(env!("CARGO_MANIFEST_DIR")).join("src")));
    let b = app2.git_branch.clone();
    assert!(b.is_some(), "repo propio tiene branch");
    drop(app2);
    let _ = std::fs::remove_file(p);
}

#[test]
fn apply_text_edits_con_utf16_y_emoji() {
    use mentl::buffer::{Buffer, TextLspEdit};
    let f = std::env::temp_dir().join(format!("l2-apply-{}.txt", std::process::id()));
    std::fs::write(&f, "uno\n😀 x\nfin\n").unwrap();
    let mut b = Buffer::open(&f).unwrap();
    // edit que reemplaza la línea del emoji entera (UTF-16: 😀=2)
    let edits = vec![TextLspEdit {
        start_line: 1,
        start_u16: 0,
        end_line: 1,
        end_u16: 4, // "😀 x" = 2+1+1 chars utf16
        new_text: "✓ cambiada".into(),
    }];
    let n = b.apply_text_edits(&edits, "format");
    assert_eq!(n, 1);
    assert_eq!(b.line_text(1), "✓ cambiada");
    assert_eq!(b.line_text(0), "uno", "lo demás intacto");
}

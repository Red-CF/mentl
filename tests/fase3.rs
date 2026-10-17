//! Fase 3: tabs lifecycle, popup Esc ≠ Esc-keymap, guard de Quit con AI.

use crossterm::event::KeyCode;
use mentl::app::{App, CompletionPopup};
use mentl::input::InputKey;
use std::path::PathBuf;

fn key(s: &str) -> InputKey {
    InputKey::parse(s).expect("token válido en test")
}

fn k(code: KeyCode) -> InputKey {
    InputKey::simple(code)
}

fn file(name: &str, content: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("mentl-f3-{}-{}.rs", name, std::process::id()));
    std::fs::write(&p, content).unwrap();
    p
}

#[test]
fn open_cicla_y_cierra_tabs() {
    let a = file("a", "uno\n");
    let b = file("b", "dos\n");
    let mut app = App::new(Some(a.clone()));
    assert_eq!(app.open_count(), 1);
    app.open_in_pane(&b).expect("abre B");
    assert_eq!(app.open_count(), 2, "push+retarget: las dos tabs");
    // B activo
    assert!(mentl::app::same_path(app.buffer().path.as_ref(), Some(&b)));
    app.on_key(key("space"));
    app.on_key(key("p")); // tab prev -> A
    assert!(mentl::app::same_path(app.buffer().path.as_ref(), Some(&a)));
    app.on_key(key("space"));
    app.on_key(key("n")); // next -> B
    assert!(mentl::app::same_path(app.buffer().path.as_ref(), Some(&b)));
    app.on_key(key("space"));
    app.on_key(key("d")); // close B
    assert_eq!(app.open_count(), 1);
    assert!(mentl::app::same_path(app.buffer().path.as_ref(), Some(&a)));
    drop(app);
    let _ = std::fs::remove_file(a);
    let _ = std::fs::remove_file(b);
}

#[test]
fn esc_popup_solo_cierra_popup_no_insert() {
    let a = file("c", "uno\n");
    let mut app = App::new(Some(a));
    app.on_key(key("i")); // insert mode
    app.completion = Some(CompletionPopup {
        items: vec![mentl::lsp::types::CompletionItem {
            label: "mock_fn".into(),
            kind: 3,
            insert: "mock_fn(x)".into(),
        }],
        filtered: vec![0],
        cursor: 0,
    });
    app.on_key(k(KeyCode::Esc));
    assert!(app.completion.is_none(), "popup cerrado");
    assert_eq!(app.mode, mentl::keymap::Mode::Insert, "sigue en insert");
    // y Up/Down navegan sin escribir texto
    app.completion = Some(mentl::app::CompletionPopup {
        items: vec![
            mentl::lsp::types::CompletionItem {
                label: "a".into(),
                kind: 3,
                insert: "a".into(),
            },
            mentl::lsp::types::CompletionItem {
                label: "b".into(),
                kind: 3,
                insert: "b".into(),
            },
        ],
        filtered: vec![0, 1],
        cursor: 0,
    });
    app.on_key(k(KeyCode::Down));
    assert_eq!(app.completion.as_ref().map(|p| p.cursor), Some(1));
    app.on_key(k(KeyCode::Enter));
    assert_eq!(
        app.buffer().line_text(0),
        "buno",
        "acepta el item con cursor"
    );
    drop(app);
}

#[test]
fn ai_buffer_no_bloquea_quit_y_no_lsp() {
    let a = file("d", "x\n");
    let mut app = App::new(Some(a));
    app.buffers.push(Some(mentl::buffer::Buffer::ai()));
    app.panes.panes[app.panes.active].buffer = 1; // activo = AI
    app.on_key(key("i"));
    app.on_key(k(KeyCode::Char('h')));
    app.on_key(k(KeyCode::Char('o')));
    app.on_key(k(KeyCode::Enter)); // prompt con ai sin config real -> job echo fallará; ok
    assert!(app.buffer().ai);
    app.on_key(k(KeyCode::Esc)); // sal de insert antes del quit
    // quit: AI modified no cuenta
    app.on_key(key("space"));
    app.on_key(key("q"));
    assert!(app.quit, "AI buffer no bloquea el guard de quit");
    // y nunca tocó LSP: language_of(None) = None
    let path = app.buffer().path.clone();
    assert!(path.is_none(), "AI buffer sin path");
    drop(app);
}

#[test]
fn explorer_abre_en_tab() {
    let dir = std::env::temp_dir().join(format!("mentl-f3-dir-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("pkg")).unwrap();
    std::fs::write(dir.join("main.rs"), "x\n").unwrap();
    std::fs::write(dir.join("pkg/util.rs"), "y\n").unwrap();
    let mut app = App::new(Some(dir.join("main.rs")));
    assert_eq!(app.project_root, dir, "root = dir del file arg");
    app.on_key(key("space"));
    app.on_key(key("e"));
    assert!(app.sidebar.is_some() && app.sidebar_focus);
    // filas: [pkg/, main.rs] -> l expande pkg, j a util.rs, Enter abre
    app.on_key(key("l"));
    app.on_key(key("j"));
    app.on_key(k(KeyCode::Enter));
    assert!(!app.sidebar_focus, "foco vuelve al editor");
    assert!(app.sidebar.is_some(), "la sidebar se queda abierta");
    assert_eq!(app.open_count(), 2);
    assert_eq!(
        app.buffer()
            .path
            .clone()
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy(),
        "util.rs"
    );
    let _ = std::fs::remove_dir_all(dir);
}

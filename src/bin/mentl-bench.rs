//! mentl-bench — timing de rutas calientes, std puro (medianas).
//! Uso: cargo run --release --bin mentl-bench [iters]

use std::time::{Duration, Instant};

use ropey::Rope;
use ratatui::{backend::TestBackend, Terminal};

use mentl::app::App;
use mentl::buffer::Buffer;
use mentl::input::InputKey;
use mentl::lsp::types;

fn median(mut xs: Vec<Duration>) -> Duration {
    xs.sort();
    match xs.is_empty() {
        true => Duration::ZERO,
        false => xs[xs.len() / 2],
    }
}

fn bench(name: &str, iters: usize, mut f: impl FnMut()) -> Duration {
    f(); // warmup
    let mut samples = Vec::new();
    for _ in 0..iters {
        let t0 = Instant::now();
        f();
        samples.push(t0.elapsed());
    }
    let m = median(samples);
    println!("{name}: {m:?}");
    m
}

fn big_rope(lines: usize) -> String {
    (0..lines)
        .map(|i| format!("fn fn_{i}(x: u32) {{\n    let y = x + {i}; // 😀 中\n}}\n"))
        .collect()
}

fn main() {
    let iters = std::env::args()
        .nth(1)
        .and_then(|a| a.parse::<usize>().ok())
        .filter(|&i| i > 0)
        .unwrap_or(200);
    println!("mentl-bench (iters={iters}, mediana)\n");

    // ---------------------------------------------------------- buffer core --
    let text = big_rope(5_000); // ~190KB, archivo monorepo
    bench("rope empty", iters, || {
        Rope::from_str("");
    });
    bench("rope from_str 5k (~190KB)", iters, || {
        Rope::from_str(&text);
    });

    let mut b = Buffer::empty(None);
    b.insert_text(&text, "insert");
    bench("line_text línea media", iters, || {
        b.line_text(2_500).chars().count();
    });
    bench("mv cursor ±1 (clamp incl.)", iters, || {
        b.mv(0, 1);
        b.mv(0, -1);
    });
    bench("insert+backspace (par undo)", iters, || {
        b.insert_text("x", "insert");
        b.delete_back();
    });
    bench("lsp roundtrip pos (UTF-16)", iters, || {
        let (l, c) = b.lsp_pos_of(2_500, 10);
        b.pos_of_lsp(l, c);
    });
    // ciclo honesto: snap (rope clone) + pop — el pop a secas estaba
    // midiendo mayormente no-ops contra la pila vacía (hallazgo del review)
    bench("undo ciclo (snap+pop, mismo stroke)", iters, || {
        b.insert_text("x", "insert");
        b.undo();
    });

    // ------------------------------------------------- fase 4: lote de edición --
    bench("comment_toggle 500 líneas", iters, || {
        b.comment_toggle_lines(0, 499);
        b.comment_toggle_lines(0, 499); // toggle ida+ vuelta: buffer estable
    });
    bench("replace_all 5k (~1.2k matches)", iters, || {
        b.replace_all("fn fn_", "funcion_");
        b.replace_all("funcion_", "fn fn_"); // ida y vuelta
    });

    // -------------------------------------------------------------- keymap --
    let f = std::env::temp_dir().join("mentl-bench-fixture.txt");
    std::fs::write(&f, &text).unwrap();
    let mut app = App::new(Some(f.clone()));
    app.last_editor_rect = ratatui::layout::Rect::new(0, 0, 120, 40);
    app.last_view_h = 40;
    let key_j = InputKey::simple(crossterm::event::KeyCode::Char('j'));
    bench("on_key j (keymap→dispatch→follow)", iters, || {
        app.on_key(key_j);
    });

    // ------------------------------------------------------------ LSP parse --
    let mut diags = serde_json::json!([]);
    for i in 0..300 {
        diags
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "range": {"start": {"line": i, "character": 3}},
                "severity": 2,
                "message": format!("warn {i} con texto medio largo"),
            }));
    }
    let params = serde_json::json!({"uri": "file:///mock.rs", "diagnostics": diags});
    bench("parse_diagnostics 300", iters, || {
        types::parse_diagnostics(&params);
    });

    // ---------------------------------------------------------------- render --
    {
        let mut app2 = App::new(Some(f.clone()));
        let mut term = Terminal::new(TestBackend::new(120, 40)).unwrap();
        let _ = term.draw(|fr| mentl::draw::draw(fr, &mut app2));
        bench("draw frame 5k (120×40)", iters, || {
            let _ = term.draw(|fr| mentl::draw::draw(fr, &mut app2));
        });
    }

    let _ = std::fs::remove_file(&f);
    println!("\nfin — compara siempre en release (docs/bench.md)");
}
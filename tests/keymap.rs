//! Chord timeout: secuencia incompleta se limpia en tick tras el vencimiento.

use std::path::PathBuf;
use std::time::{Duration, Instant};
use mentl::app::App;
use mentl::input::InputKey;

#[test]
fn chord_timeout_limpia_pending() {
    let f = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample.txt");
    let mut app = App::new(Some(f));
    // leader a secas: pendiente "espacio" (prefix de leader w/q/v/s/...)
    app.on_key(InputKey::parse("space").unwrap());
    assert_eq!(app.pending.len(), 1, "esperando tail del chord");
    let t0 = Instant::now();
    // tick antes del vencimiento: pendiente sigue
    app.tick();
    assert_eq!(app.pending.len(), 1);
    // durmimos hasta vencer (500ms default) + margen
    let wait = Duration::from_millis(550).saturating_sub(t0.elapsed());
    std::thread::sleep(wait);
    app.tick();
    assert!(app.pending.is_empty(), "timeout limpia el chord");
    // y la siguiente tecla funciona normal
    app.on_key(InputKey::parse("j").unwrap());
    assert_eq!(app.buffer().cursor.0, 1, "j baja tras el timeout");
}
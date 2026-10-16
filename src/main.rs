//! mentl — editor TUI. main = arg parse, terminal guard, loop.

use std::time::Duration;
use crossterm::event::{self, Event};
use ratatui::Terminal;

use mentl::app::App;
use mentl::config;
use mentl::watch;

fn main() {
    // ratatui::init(): raw mode + alternate screen — PRIMERA sentencia para
    // que el terminal nunca quede con flow control activo a medias.
    let mut terminal = ratatui::init();

    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        ratatui::restore(); // terminal restaurable tras panic
        prev_hook(info);
    }));

    let arg = std::env::args().nth(1).map(std::path::PathBuf::from);
    let mut app = App::new(arg);

    let cfg_path = config::config_path();
    let mut watcher = watch::Watcher::new(&cfg_path);

    let res = run(&mut terminal, &mut app, &mut watcher, &cfg_path);
    ratatui::restore();
    if let Err(e) = res {
        eprintln!("mentl: {e}");
        std::process::exit(1);
    }
}

fn run(
    terminal: &mut Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    app: &mut App,
    watcher: &mut watch::Watcher,
    cfg_path: &std::path::Path,
) -> std::io::Result<()> {
    // draw barato de mantener: redibujo solo con cambios reales
    let mut need_draw = true;
    loop {
        // config hot-reload por mtime (poll barato cada frame)
        if watcher.changed(cfg_path) {
            app.reload_config();
            need_draw = true;
        }

        // timeouts de chord / decay de mensajes
        need_draw |= app.tick();

        if need_draw {
            need_draw = false;
            terminal.draw(|f| mentl::draw::draw(f, app))?;
        }

        if app.quit {
            break;
        }

        // drena eventos: con draw pendiente, poll(0) — latencia de tecla ~0.
        // Con todo idle, 120ms para el tick. key-repeat se agrupa en 1 draw.
        let idle = Duration::from_millis(120);
        let now = Duration::from_millis(0);
        while event::poll(if need_draw { now } else { idle })? {
            match event::read()? {
                Event::Key(k) => {
                    if let Some(k) = mentl::input::InputKey::from_event(k) {
                        app.on_key(k);
                        need_draw = true;
                    }
                }
                Event::Resize(_, _) => {} // layout se recalcula en cada draw
                _ => {}
            }
        }
    }
    Ok(())
}
//! AI pane: impl sobre App — transcript = Buffer con ai:true.

use std::time::Instant;

use crate::app::{App, AiJob};
use crate::keymap::Mode;
use crate::panes::Dir;
use crate::buffer::Buffer;

impl App {
    pub(super) fn ai_toggle(&mut self) {
        // el pane activo ya muestra AI? -> ciérralo
        if self.buffer().ai {
            let _ = self.panes.close();
            self.follow_cursor();
            return;
        }
        self.panes.split(Dir::Horiz);
        let idx = self.buffers.len();
        self.buffers.push(Some(Buffer::ai()));
        self.panes.panes[self.panes.active].buffer = idx;
        self.mode = Mode::Insert;
        self.cursor_style_dirty = true;
    }

pub(super) fn ai_context(&mut self) {
        let path = self.buffer().path.clone();
        let Some(p) = path else {
            self.status_msg("buffer sin archivo para contexto");
            return;
        };
        let max = self.config.ai.max_context_lines;
        let content = self
            .buffer()
            .rope
            .lines()
            .take(max)
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("");
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.ai_ctx = Some(format!("Archivo: {}\n```\n{content}\n```", p.display()));
        self.status_msg(format!("ctx: {name} (máx {max} líneas)"));
    }

pub(super) fn ai_submit(&mut self) {
        let (line, _) = self.buffer().cursor;
        let prompt = self.buffer().line_text(line).trim().to_string();
        if prompt.is_empty() || prompt.starts_with("▸") || prompt.starts_with("…") {
            return;
        }
        if let Some(job) = self.ai_jobs.last() {
            let _ = job; // hay un job en vuelo — espera respuesta
            return;
        }
        let ctx = self.ai_ctx.take();
        let final_prompt = match ctx {
            Some(c) => format!("{c}\n\n{prompt}"),
            None => prompt.clone(),
        };
        let cmd = self.config.ai.command.clone();
        let timeout = std::time::Duration::from_secs(self.config.ai.timeout_secs);
        let (tx, rx) = std::sync::mpsc::channel();
        let _ = std::thread::Builder::new()
            .name("ai-job".into())
            .spawn(move || ai_run(cmd, final_prompt, timeout, tx));
        // transcript: bloque de prompt + marcador pendiente en el EOF
        self.buffer().file_end();
        self.buffer().insert_text(&format!("\n▸ {prompt}\n"), "insert");
        self.buffer().file_end();
        self.buffer().insert_text("\n… thinking …", "insert");
        self.buffer().file_end();
        let marker = self.buffer().cursor;
        self.ai_jobs.push(AiJob { rx, started: Instant::now(), marker });
    }

pub(super) fn replace_marker_line(&mut self, line: usize, text: &str) {
        let b = self.buffer();
        let len_lines = b.len_lines();
        let start = b.rope.line_to_char(line.min(len_lines.saturating_sub(1)));
        let end = b
            .rope
            .line_to_char((line + 1).min(len_lines))
            .min(b.rope.len_chars());
        b.rope.remove(start..end);
        b.insert_text(&format!("\n{text}\n"), "insert");
        b.file_end();
        b.modified = true;
    }

pub(super) fn ai_tick(&mut self) -> bool {
        let mut changed = false;
        let timeout = std::time::Duration::from_secs(self.config.ai.timeout_secs);
        for job in std::mem::take(&mut self.ai_jobs) {
            match job.rx.try_recv() {
                Ok(Ok(body)) => {
                    self.replace_marker_line(job.marker.0, &body.trim_end());
                    self.status_msg("ai: respuesta lista");
                    changed = true;
                }
                Ok(Err(e)) => {
                    self.replace_marker_line(job.marker.0, &format!("✖ {e}"));
                    self.status_msg(format!("ai: {e}"));
                    changed = true;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    if job.started.elapsed() > timeout {
                        self.replace_marker_line(job.marker.0, "✖ timeout");
                        changed = true;
                    } else {
                        self.ai_jobs.push(job); // sigue esperando
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    if job.started.elapsed() > timeout {
                        self.replace_marker_line(job.marker.0, "✖ sin respuesta");
                        changed = true;
                    }
                }
            }
        }
        changed
    }
}

/// Corre el comando AI en threads: lector de stdout + watchdog de timeout.
/// Nunca bloquea el main loop; el kill del watchdog cierra stdout y el
/// lector termina.
fn ai_run(
    cmd: Vec<String>,
    prompt: String,
    timeout: std::time::Duration,
    tx: std::sync::mpsc::Sender<std::io::Result<String>>,
) {
    use std::process::Stdio;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    if cmd.is_empty() {
        let _ = tx.send(Err(std::io::Error::other("[ai] sin command")));
        return;
    }
    let child = match std::process::Command::new(&cmd[0])
        .args(&cmd[1..])
        .arg(&prompt)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .stdin(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(Err(e));
            return;
        }
    };
    let killed = Arc::new(AtomicBool::new(false));
    let child_shared = Arc::new(Mutex::new(child));

    // watchdog: mata pasado el timeout
    let w_child = Arc::clone(&child_shared);
    let w_killed = Arc::clone(&killed);
    std::thread::spawn(move || {
        std::thread::sleep(timeout);
        w_killed.store(true, Ordering::SeqCst);
        if let Ok(mut c) = w_child.lock() {
            let _ = c.kill();
        }
    });

    // lector: stdout hasta EOF
    let mut stdout = child_shared.lock().unwrap().stdout.take().expect("stdout");
    let mut buf = String::new();
    let mut io_err: Option<std::io::Error> = None;
    {
        use std::io::Read;
        let mut chunk = [0u8; 8192];
        loop {
            match stdout.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => buf.push_str(&String::from_utf8_lossy(&chunk[..n])),
                Err(e) => {
                    io_err = Some(e);
                    break;
                }
            }
        }
    }
    let _ = child_shared.lock().unwrap().wait();
    let _ = tx.send(match (io_err, killed.load(Ordering::SeqCst)) {
        (Some(e), _) => Err(e),
        (None, true) => Err(std::io::Error::other("timeout")),
        (None, false) => Ok(buf),
    });
}

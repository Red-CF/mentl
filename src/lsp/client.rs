//! Cliente LSP por lenguaje: thread propio, handshake, loop cmd/notify.
//! El main loop JAMÁS se bloquea. Latencia: cmd drain no-bloqueante + inbound
//! vía reader-thread (un server silencioso NUNCA congela los comandos).

use std::collections::HashMap;
use std::io::{BufReader, BufWriter};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::Duration;

use serde_json::json;

use super::LspEvent;
use super::types::{self, InMsg, OutMsg};

const TICK: Duration = Duration::from_millis(50);

pub enum Cmd {
    Request {
        req_id: u64,
        method: String,
        params: serde_json::Value,
    },
    Notify {
        method: String,
        params: serde_json::Value,
    },
    Shutdown,
}

pub struct ClientHandle {
    pub tx: Sender<Cmd>,
    pub lang: &'static str,
    pub ready: bool,
}

pub fn spawn(
    lang: &'static str,
    command: &[String],
    tx_events: Sender<LspEvent>,
) -> Option<ClientHandle> {
    let (cmd_tx, cmd_rx) = mpsc::channel::<Cmd>();
    let mut child = match Command::new(&command[0])
        .args(&command[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            let _ = tx_events.send(LspEvent::StartedFailed {
                lang,
                err: format!("{}: {e}", command[0]),
            });
            return None;
        }
    };
    let stdin = child.stdin.take().expect("stdin piped");
    let stdout = child.stdout.take().expect("stdout piped");

    // reader thread: frames entrantes -> canal
    let (msg_tx, msg_rx) = mpsc::channel::<Option<std::io::Result<String>>>();
    std::thread::Builder::new()
        .name(format!("lsp-{lang}-reader"))
        .spawn(move || {
            let mut inp = BufReader::new(stdout);
            loop {
                match super::transport::read_msg(&mut inp) {
                    Ok(Some(s)) => {
                        if msg_tx.send(Some(Ok(s))).is_err() {
                            return; // hub cayó
                        }
                    }
                    Ok(None) => {
                        let _ = msg_tx.send(None);
                        return;
                    }
                    Err(e) => {
                        let _ = msg_tx.send(Some(Err(e)));
                        return;
                    }
                }
            }
        })
        .ok()?;

    let lang_t = lang;
    std::thread::Builder::new()
        .name(format!("lsp-{lang_t}"))
        .spawn(move || {
            client_loop(
                child,
                BufWriter::new(stdin),
                msg_rx,
                cmd_rx,
                lang_t,
                tx_events,
            )
        })
        .ok()?;
    Some(ClientHandle {
        tx: cmd_tx,
        lang,
        ready: false,
    })
}

fn client_loop(
    mut child: Child,
    mut out: BufWriter<ChildStdin>,
    msg_rx: Receiver<Option<std::io::Result<String>>>,
    cmd_rx: Receiver<Cmd>,
    lang: &'static str,
    tx_events: Sender<LspEvent>,
) {
    let mut next_id: u64 = 1;
    let mut pending: HashMap<u64, u64> = HashMap::new(); // wire id -> app req id

    // handshake initialize
    let params = json!({
        "processId": std::process::id(),
        "rootUri": types::path_to_uri(&std::env::current_dir().unwrap_or_default()),
        "capabilities": {
            "textDocument": {
                "synchronization": {"didOpen": true, "didChange": true, "didSave": true},
                "hover": {"contentFormat": ["markdown", "plaintext"]},
                "definition": {"linkSupport": true},
            },
            "general": {"positionEncodings": ["utf-16"]},
        }
    });
    let _ = transport_write(
        &mut out,
        &OutMsg::Request {
            id: next_id,
            method: "initialize".into(),
            params,
        },
    );
    next_id += 1;
    // handshake: solo esperamos la respuesta initialize
    loop {
        match msg_rx.recv_timeout(Duration::from_secs(30)) {
            Ok(Some(Ok(msg))) => match types::parse_in(&msg) {
                InMsg::Response { .. } => break,
                InMsg::Request { id, .. } => answer_request(&mut out, id),
                _ => {}
            },
            Ok(Some(Err(_))) | Ok(None) | Err(_) => {
                let _ = tx_events.send(LspEvent::Died {
                    lang,
                    err: "sin responder initialize".into(),
                });
                return;
            }
        }
    }
    let _ = transport_write(
        &mut out,
        &OutMsg::Notification {
            method: "initialized".into(),
            params: json!({}),
        },
    );
    let _ = tx_events.send(LspEvent::Started { lang });

    // main client loop: cmds drain no-bloqueante + inbound con tick 50ms
    loop {
        let mut shutdown = false;
        loop {
            match cmd_rx.try_recv() {
                Ok(Cmd::Request {
                    req_id,
                    method,
                    params,
                }) => {
                    let wire = next_id;
                    next_id += 1;
                    pending.insert(wire, req_id);
                    let _ = transport_write(
                        &mut out,
                        &OutMsg::Request {
                            id: wire,
                            method,
                            params,
                        },
                    );
                }
                Ok(Cmd::Notify { method, params }) => {
                    let _ = transport_write(&mut out, &OutMsg::Notification { method, params });
                }
                Ok(Cmd::Shutdown) => {
                    shutdown = true;
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    let _ = tx_events.send(LspEvent::Died {
                        lang,
                        err: "hub cayó".into(),
                    });
                    return;
                }
            }
        }
        if shutdown {
            shutdown_exit(&mut child, &mut out, &mut next_id);
            return;
        }

        // inbound disponible?
        match msg_rx.try_recv() {
            Ok(Some(Ok(msg))) => {
                match types::parse_in(&msg) {
                    InMsg::Notification { method, params } => {
                        if method == "textDocument/publishDiagnostics"
                            && let Some((uri, diags)) = types::parse_diagnostics(&params)
                        {
                            let _ = tx_events.send(LspEvent::Diagnostics { uri, diags });
                        }
                        // $/progress, window/logMessage etc: silencio
                    }
                    InMsg::Request { id, .. } => answer_request(&mut out, id),
                    InMsg::Response { id, result, .. } => {
                        if let Some(req_id) = pending.remove(&id) {
                            let _ = tx_events.send(LspEvent::Response {
                                req_id,
                                body: result,
                            });
                        }
                    }
                    InMsg::Unknown => {}
                }
            }
            Ok(None) => {
                let _ = tx_events.send(LspEvent::Died {
                    lang,
                    err: "server cerró stdio".into(),
                });
                return;
            }
            Ok(Some(Err(_))) => {
                let _ = tx_events.send(LspEvent::Died {
                    lang,
                    err: "stdio roto".into(),
                });
                return;
            }
            Err(TryRecvError::Empty) => {
                // nada inbound: espera comandos o tick
                match cmd_rx.recv_timeout(TICK) {
                    Ok(Cmd::Request {
                        req_id,
                        method,
                        params,
                    }) => {
                        let wire = next_id;
                        next_id += 1;
                        pending.insert(wire, req_id);
                        let _ = transport_write(
                            &mut out,
                            &OutMsg::Request {
                                id: wire,
                                method,
                                params,
                            },
                        );
                    }
                    Ok(Cmd::Notify { method, params }) => {
                        let _ = transport_write(&mut out, &OutMsg::Notification { method, params });
                    }
                    Ok(Cmd::Shutdown) => {
                        shutdown_exit(&mut child, &mut out, &mut next_id);
                        return;
                    }
                    Err(_) => {} // Empty o Disconnected — sigue el loop
                }
            }
            Err(TryRecvError::Disconnected) => {
                let _ = tx_events.send(LspEvent::Died {
                    lang,
                    err: "hub cayó".into(),
                });
                return;
            }
        }
    }
}

/// Secuencia shutdown+exit+wait — UNA sola fuente (spec LSP).
fn shutdown_exit(child: &mut Child, out: &mut BufWriter<ChildStdin>, next_id: &mut u64) {
    let _ = transport_write(
        out,
        &OutMsg::Request {
            id: *next_id,
            method: "shutdown".into(),
            params: json!(null),
        },
    );
    *next_id += 1;
    let _ = transport_write(
        out,
        &OutMsg::Notification {
            method: "exit".into(),
            params: json!(null),
        },
    );
    let _ = child.wait();
}

/// workspace/configuration, client/registerCapability etc: result nulo y
/// el server sigue. MVP suficiente.
fn answer_request<W: std::io::Write>(out: &mut W, id: u64) {
    let body = serde_json::json!({"jsonrpc":"2.0","id":id,"result":null}).to_string();
    let _ = super::transport::write_msg(
        out,
        &format!("Content-Length: {}\r\n\r\n{}", body.len(), body),
    );
}

fn transport_write<W: std::io::Write>(w: &mut W, msg: &OutMsg) -> std::io::Result<()> {
    super::transport::write_msg(w, &msg.to_frame())
}

//! Mock LSP server (stdio JSON-RPC) — smoke tool y fixture de tests.
//! initialize ok; hover con fences; definition 2 líneas abajo; diagnósticos
//! con posición UTF-16 DENTRO de una línea con emoji.

use std::io::{BufRead, BufReader, Write};

/// Framing LSP reutilizado de la lib — una sola fuente de verdad.
fn send<W: std::io::Write>(w: &mut W, body: &str) {
    let frame = format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
    let _ = mentl::lsp::transport::write_msg(w, &frame);
}

fn read_msg<R: BufRead>(r: &mut R) -> Option<String> {
    mentl::lsp::transport::read_msg(r).ok().flatten()
}

fn main() {
    let mut inp = BufReader::new(std::io::stdin());
    let mut out = std::io::stdout();
    let mut opened_first = false;
    let mut doc_uri = "file:///mock.rs".to_string();
    let close_count = std::cell::RefCell::new(0u64);
    loop {
        let Some(body) = read_msg(&mut inp) else {
            return;
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) else {
            continue;
        };
        let Some(method) = v.get("method").and_then(|m| m.as_str()) else {
            continue;
        };
        let id = v.get("id").cloned().unwrap_or(serde_json::Value::Null);

        match method {
            "initialize" => reply(
                &mut out,
                &id,
                serde_json::json!({
                    "capabilities": {
                        "textDocumentSync": { "openClose": true, "change": 1 },
                        "hoverProvider": true,
                        "definitionProvider": true,
                    }
                }),
            ),
            "initialized" => {}
            "textDocument/didOpen" => {
                doc_uri = v
                    .pointer("/params/textDocument/uri")
                    .and_then(|u| u.as_str())
                    .unwrap_or("file:///mock.rs")
                    .to_string();
                if !opened_first {
                    opened_first = true;
                    // línea 2 contiene "😀", col UTF-16 3 = entre emoji y "fin"
                    let _ = notify(
                        &mut out,
                        "textDocument/publishDiagnostics",
                        serde_json::json!({
                            "uri": doc_uri.clone(),
                            "diagnostics": [
                                {"range": {"start": {"line": 2, "character": 3},
                                           "end": {"line": 2, "character": 5}},
                                 "severity": 2, "message": "mock: warn en línea 3"},
                                {"range": {"start": {"line": 5, "character": 0},
                                           "end": {"line": 5, "character": 4}},
                                 "severity": 1, "message": "mock: error"},
                            ],
                        }),
                    );
                }
            }
            "textDocument/didChange" | "textDocument/didSave" => {}
            "textDocument/didClose" => {
                *close_count.borrow_mut() += 1;
            }
            "textDocument/completion" => reply(
                &mut out,
                &id,
                serde_json::json!([
                    {"label": "mock_fn", "kind": 3, "insertText": "mock_fn(x)"},
                    {"label": "mock_var", "kind": 13, "insertText": "mock_var: u32 = 1"},
                    {"label": "MockStruct", "kind": 23, "insertText": "MockStruct{}"},
                ]),
            ),
            "mentl/closeCount" => {
                let n = *close_count.borrow();
                reply(&mut out, &id, serde_json::json!(n))
            }
            "textDocument/hover" => reply(
                &mut out,
                &id,
                serde_json::json!({
                    "contents": {"kind": "markdown", "value": "# Título\n```rust\nfn mock(x: u32)\n```\nEl *hover* con `markdown`"}
                }),
            ),
            "textDocument/definition" => reply(
                &mut out,
                &id,
                serde_json::json!({
                    "uri": doc_uri,
                    "range": {"start": {"line": 4, "character": 2}}
                }),
            ),
            "shutdown" => reply(&mut out, &id, serde_json::Value::Null),
            "exit" => return,
            m if m.starts_with("$/") => {}
            _ => {
                if !id.is_null() {
                    reply(&mut out, &id, serde_json::Value::Null)
                }
            }
        }
    }
}

fn reply<W: Write>(w: &mut W, id: &serde_json::Value, result: serde_json::Value) {
    send(
        w,
        &serde_json::json!({"jsonrpc":"2.0","id":id,"result":result}).to_string(),
    );
}

fn notify<W: Write>(w: &mut W, method: &str, params: serde_json::Value) -> std::io::Result<()> {
    send(
        w,
        &serde_json::json!({"jsonrpc":"2.0","method":method,"params":params}).to_string(),
    );
    Ok(())
}

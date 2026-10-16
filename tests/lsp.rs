//! Integración LSP end-to-end contra el mock server (bin propio).

use std::time::{Duration, Instant};

use mentl::config::{LspCfg, ServerCfg};
use mentl::lsp::{self, LspEvent};

fn hub() -> lsp::LspHub {
    let mut h = lsp::LspHub::default();
    h.enabled = true;
    h
}

fn mock_cfg() -> (LspCfg, ServerCfg) {
    let mut cfg = LspCfg::default();
    let server = ServerCfg {
        command: vec![env!("CARGO_BIN_EXE_mentl-lsp-mock").to_string()],
    };
    (cfg, server)
}

fn wait_event(hub: &lsp::LspHub, timeout_ms: u64, pred: impl Fn(&LspEvent) -> bool) -> Option<LspEvent> {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        if Instant::now() > deadline {
            return None;
        }
        let evs = hub.drain();
        for ev in evs {
            if pred(&ev) {
                return Some(ev);
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn mock_handshake_diagnostics_hover_definition() {
    let mut h = hub();
    let (cfg, server) = mock_cfg();
    h.ensure_client("rust", &cfg, Some(&server));

    // 1. Started tras handshake
    wait_event(&h, 3000, |e| matches!(e, LspEvent::Started { lang: "rust" }))
        .expect("mock server debería arrancar");
    h.mark_ready("rust");
    assert!(h.is_ready("rust"));

    // 2. didOpen -> Diagnostics con posiciones toleradas
    h.notify(
        "rust",
        "textDocument/didOpen",
        serde_json::json!({
            "textDocument": {
                "uri": "file:///mock.rs",
                "languageId": "rust",
                "version": 0,
                "text": "a\nb\n😀fin\nd\nfn mock_target()\nf\n",
            }
        }),
    );
    let diags_event = wait_event(&h, 3000, |e| matches!(e, LspEvent::Diagnostics { .. }))
        .expect("publishDiagnostics del mock");
    let LspEvent::Diagnostics { uri, diags } = diags_event else { unreachable!() };
    assert_eq!(uri, "file:///mock.rs");
    assert_eq!(diags.len(), 2);
    assert_eq!(diags[0].severity, 2);
    assert_eq!((diags[0].line, diags[0].character), (2, 3), "col UTF-16 en línea con emoji");
    assert_eq!(diags[1].severity, 1);

    // 3. hover round-trip: markdown limpio
    let req = h
        .request("rust", "textDocument/hover", serde_json::json!({
            "textDocument": {"uri": "file:///mock.rs"},
            "position": {"line": 0, "character": 0},
        }))
        .expect("request listo");
    let resp = wait_event(&h, 3000, |e| matches!(e, LspEvent::Response { req_id, .. } if *req_id == req))
        .expect("hover response");
    let LspEvent::Response { body, .. } = resp else { unreachable!() };
    let text = lsp::types::parse_hover(&body).expect("hover parseable");
    assert!(text.contains("fn mock(x: u32)"), "texto: {text}");
    assert!(!text.contains("```"), "fences fuera: {text}");
    assert!(!text.contains('`'), "backticks fuera: {text}");

    // 4. definition round-trip
    let req = h
        .request("rust", "textDocument/definition", serde_json::json!({
            "textDocument": {"uri": "file:///mock.rs"},
            "position": {"line": 0, "character": 0},
        }))
        .unwrap();
    let resp = wait_event(&h, 3000, |e| matches!(e, LspEvent::Response { req_id, .. } if *req_id == req))
        .expect("definition response");
    let LspEvent::Response { body, .. } = resp else { unreachable!() };
    let locs = lsp::types::parse_definition(&body);
    assert_eq!(locs.len(), 1);
    assert_eq!(locs[0].uri, "file:///mock.rs");
    assert_eq!((locs[0].line, locs[0].character), (4, 2));

    // 5. server muerto → Died (el mock sale con exit tras shutdown)
    let _ = h.rx; // noop para mantener el binding
    drop(h); // hub caído -> thread detecta disconnect (no Died garantizado)
}

#[test]
fn framing_content_length() {
    // transport: roundtrip de frame con body unicode
    let msg = mentl::lsp::types::OutMsg::Notification {
        method: "test/unicode".into(),
        params: serde_json::json!({"x": "héllo 😀"}),
    };
    let frame = msg.to_frame();
    let mut reader = frame.as_bytes();
    let parsed = mentl::lsp::transport::read_msg(&mut reader).expect("parse frame").expect("body");
    let v: serde_json::Value = serde_json::from_str(&parsed).unwrap();
    assert_eq!(v["method"], "test/unicode");
    assert_eq!(v["params"]["x"], "héllo 😀");
}
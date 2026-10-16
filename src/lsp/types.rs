//! Tipos LSP tolerantes: `Value` fallback en todo lo que no entendemos —
//! jdtls manda payloads raros y NUNCA matamos un batch de diagnósticos por 1.

use serde::Deserialize;
use serde_json::Value;

// ------------------------------------------------------------ JSON-RPC -----

#[derive(Debug)]
pub enum OutMsg {
    Request { id: u64, method: String, params: Value },
    Notification { method: String, params: Value },
}

impl OutMsg {
    pub fn to_frame(&self) -> String {
        let (method, params, id) = match self {
            OutMsg::Request { id, method, params } => (method, params, Some(*id)),
            OutMsg::Notification { method, params } => (method, params, None),
        };
        let body = match id {
            Some(id) => serde_json::json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}),
            None => serde_json::json!({"jsonrpc":"2.0","method":method,"params":params}),
        };
        let body = body.to_string();
        format!("Content-Length: {}\r\n\r\n{}", body.len(), body)
    }
}

/// Mensaje entrante parseado de forma tolerante.
pub enum InMsg {
    Response { id: u64, result: Value, error: Option<Value> },
    Request { id: u64, method: String, params: Value },
    Notification { method: String, params: Value },
    Unknown,
}

pub fn parse_in(body: &str) -> InMsg {
    let Ok(v) = serde_json::from_str::<Value>(body) else {
        return InMsg::Unknown;
    };
    let obj = match v.as_object() {
        Some(o) => o,
        None => return InMsg::Unknown,
    };
    let method = obj.get("method").and_then(|m| m.as_str()).map(String::from);
    match (method, obj.get("id")) {
        (Some(_), Some(id)) if obj.get("result").is_some() || obj.get("error").is_some() => {
            // respuesta a request nuestro (id numérico)
            match id.as_u64() {
                Some(id) => InMsg::Response {
                    id,
                    result: obj.get("result").cloned().unwrap_or(Value::Null),
                    error: obj.get("error").cloned(),
                },
                None => InMsg::Unknown,
            }
        }
        (Some(m), Some(id)) => InMsg::Request {
            id: id.as_u64().unwrap_or(0),
            method: m,
            params: obj.get("params").cloned().unwrap_or(Value::Null),
        },
        (Some(m), None) => InMsg::Notification {
            method: m,
            params: obj.get("params").cloned().unwrap_or(Value::Null),
        },
        (None, Some(id)) => InMsg::Response {
            id: id.as_u64().unwrap_or(0),
            result: obj.get("result").cloned().unwrap_or(Value::Null),
            error: obj.get("error").cloned(),
        },
        _ => InMsg::Unknown,
    }
}

impl InMsg {
    pub fn method(&self) -> Option<&str> {
        match self {
            InMsg::Request { method, .. } | InMsg::Notification { method, .. } => Some(method),
            _ => None,
        }
    }
}

// ------------------------------------------------------------ LSP ligero ---

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct LspPosition {
    pub line: u32,
    pub character: u32,
}
impl Default for LspPosition {
    fn default() -> Self {
        Self { line: 0, character: 0 }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct DiagSev(u32);
impl Default for DiagSev {
    fn default() -> Self {
        DiagSev(1) // LSP Default = error-ish
    }
}

#[derive(Debug, Clone)]
pub struct Diag {
    pub line: u32,
    pub character: u32,
    pub severity: u32, // 1 error 2 warn 3 info 4 hint
    pub msg: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawDiag {
    range: Option<LspRange>,
    severity: Option<u32>,
    message: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct LspRange {
    pub start: LspPosition,
    pub end: LspPosition,
}
impl Default for LspRange {
    fn default() -> Self {
        Self { start: LspPosition::default(), end: LspPosition::default() }
    }
}

/// publishDiagnostics.params: {uri, diagnostics: [..]} — tolerante.
pub fn parse_diagnostics(params: &Value) -> Option<(String, Vec<Diag>)> {
    let uri = params.get("uri")?.as_str()?.to_string();
    let raw = params
        .get("diagnostics")
        .and_then(|d| d.as_array())
        .cloned()
        .unwrap_or_default();
    let diags = raw
        .iter()
        .filter_map(|d| {
            let rd: RawDiag = serde_json::from_value(d.clone()).ok()?;
            let range = rd.range;
            Some(Diag {
                line: range.as_ref().map(|r| r.start.line).unwrap_or(0),
                character: range.as_ref().map(|r| r.start.character).unwrap_or(0),
                severity: rd.severity.unwrap_or(1),
                msg: rd.message.unwrap_or_default(),
            })
        })
        .collect();
    Some((uri, diags))
}

/// Hover.contents tolerante -> texto plano.
pub fn parse_hover(result: &Value) -> Option<String> {
    let contents = result.get("contents")?;
    extract_contents(contents)
}

fn extract_contents(v: &Value) -> Option<String> {
    // MarkupContent {value, kind}
    if let Some(s) = v.get("value").and_then(|x| x.as_str()) {
        return Some(strip_markdown(s));
    }
    // MarkedString string
    if let Some(s) = v.as_str() {
        return Some(strip_markdown(s));
    }
    // MarkedString {language, value}
    if let Some(s) = v.get("value").and_then(|x| x.as_str()) {
        return Some(strip_markdown(s));
    }
    // array de cualquiera de los anteriores
    if let Some(arr) = v.as_array() {
        let parts: Vec<String> = arr.iter().filter_map(extract_contents).collect();
        if !parts.is_empty() {
            return Some(parts.join("\n\n"));
        }
    }
    None
}

/// Markdown a plano MVP: fuera fences y backticks, colapsa vacíos.
pub fn strip_markdown(s: &str) -> String {
    let mut out = Vec::new();
    let mut fence = false;
    for line in s.lines() {
        let t = line.trim();
        if t.starts_with("```") {
            fence = !fence;
            continue;
        }
        let clean = if fence { line.to_string() } else { line.replace("`", "") };
        out.push(clean);
    }
    let mut res = String::new();
    for l in out {
        if l.trim().is_empty() && (res.is_empty() || res.ends_with('\n')) {
            continue;
        }
        res.push_str(&l);
        res.push('\n');
    }
    res.trim().to_string()
}

/// definition result tolerante: null | Location | [Location] | [LocationLink].
#[derive(Debug, Clone)]
pub struct DefLocation {
    pub uri: String,
    pub line: u32,
    pub character: u32,
}

pub fn parse_definition(result: &Value) -> Vec<DefLocation> {
    let mut out = Vec::new();
    match result {
        Value::Null => {}
        Value::Array(arr) => {
            for item in arr {
                out.extend(loc_from(item));
            }
        }
        other => out.extend(loc_from(other)),
    }
    out
}

fn loc_from(v: &Value) -> Vec<DefLocation> {
    // Location {uri, range:{start}}
    if let (Some(uri), Some(range)) = (v.get("uri"), v.get("range")) {
        if let Some(uri) = uri.as_str() {
            let start = range.get("start").unwrap_or(&Value::Null);
            return vec![DefLocation {
                uri: uri.to_string(),
                line: start.get("line").and_then(|l| l.as_u64()).unwrap_or(0) as u32,
                character: start.get("character").and_then(|c| c.as_u64()).unwrap_or(0) as u32,
            }];
        }
    }
    // LocationLink {targetUri, targetSelectionRange|targetRange}
    if v.get("targetUri").is_some() {
        let pick = v
            .get("targetSelectionRange")
            .or_else(|| v.get("targetRange"));
        if let Some(r) = pick {
            if let Some(uri) = v.get("targetUri").and_then(|u| u.as_str()) {
                return vec![DefLocation {
                    uri: uri.to_string(),
                    line: r.pointer("/start/line").and_then(|l| l.as_u64()).unwrap_or(0) as u32,
                    character: r
                        .pointer("/start/character")
                        .and_then(|c| c.as_u64())
                        .unwrap_or(0) as u32,
                }];
            }
        }
    }
    vec![]
}

#[derive(Debug, Clone)]
pub struct CompletionItem {
    pub label: String,
    pub kind: u32,
    pub insert: String,
}

/// completion result tolerante: [item] | {items} | null.
/// insert = insertText -> textEdit.newText -> label. Snippets RAW (deferred).
pub fn parse_completion(result: &Value) -> Vec<CompletionItem> {
    let items = match result {
        Value::Array(a) => some_items(a),
        Value::Object(_) => {
            if result.get("isIncomplete").is_some() || result.get("items").is_some() {
                some_items(
                    result
                        .get("items")
                        .and_then(|i| i.as_array())
                        .map(|a| a.as_ref())
                        .unwrap_or(&[]),
                )
            } else {
                vec![]
            }
        }
        _ => vec![],
    };
    items
}

fn some_items(arr: &[Value]) -> Vec<CompletionItem> {
    arr.iter()
        .filter_map(|it| {
            let label = match it.get("label") {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Object(o)) => o.get("label")?.as_str()?.to_string(),
                _ => return None,
            };
            let insert = it
                .get("insertText")
                .and_then(|v| v.as_str())
                .map(String::from)
                .or_else(|| {
                    it.get("textEdit")
                        .and_then(|t| t.get("newText"))
                        .and_then(|v| v.as_str())
                        .map(String::from)
                })
                .unwrap_or_else(|| label.clone());
            let kind = it.get("kind").and_then(|k| k.as_u64()).unwrap_or(1) as u32;
            Some(CompletionItem { label, kind, insert })
        })
        .collect()
}

// ------------------------------------------------------------- uri/path ---

pub fn uri_to_path(uri: &str) -> Option<std::path::PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let bytes = rest.as_bytes();
    let mut out: Vec<u8> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Some(h) = bytes.get(i + 1..i + 3) {
                if let Ok(n) = u8::from_str_radix(std::str::from_utf8(h).ok()?, 16) {
                    out.push(n);
                    i += 3;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    Some(std::path::PathBuf::from(String::from_utf8_lossy(&out).into_owned()))
}

pub fn path_to_uri(p: &std::path::Path) -> String {
    // encode mínimo: espacios y no-ascii simples
    let s = p.to_string_lossy();
    let mut out = String::from("file://");
    for c in s.chars() {
        match c {
            ' ' => out.push_str("%20"),
            c if (c as u32) < 128 => out.push(c),
            c => {
                let mut buf = [0u8; 4];
                for b in c.encode_utf8(&mut buf).as_bytes() {
                    out.push_str(&format!("%{:02X}", b));
                }
            }
        }
    }
    out
}
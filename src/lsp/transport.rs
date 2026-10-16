//! Framing LSP: Content-Length sobre stdio, I/O puro sin threads.

use std::io::{BufRead, Write};

/// Escribe un frame al writer.
pub fn write_msg<W: Write>(w: &mut W, frame: &str) -> std::io::Result<()> {
    w.write_all(frame.as_bytes())?;
    w.flush()
}

/// Lee un frame del reader: cabeceras hasta línea vacía + Content-Length bytes.
pub fn read_msg<R: BufRead>(r: &mut R) -> std::io::Result<Option<String>> {
    let mut content_len: Option<usize> = None;
    loop {
        let mut line = String::new();
        match r.read_line(&mut line) {
            Ok(0) => return Ok(None), // EOF
            Ok(_) => {
                let t = line.trim();
                if t.is_empty() {
                    break;
                }
                if let Some(v) = t.strip_prefix("Content-Length:") {
                    content_len = v.trim().parse().ok();
                }
                // otros headers (Content-Type) se ignoran
            }
            Err(e) => return Err(e),
        }
    }
    let Some(len) = content_len else {
        return Ok(None);
    };
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    let s = String::from_utf8(buf).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    Ok(Some(s))
}
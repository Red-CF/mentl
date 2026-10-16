# Benchmarks — mentl

`cargo run --release --bin mentl-bench [iters]` (std puro, medianas).

## Baseline 2026-10-17 (M-Series Mac, release, 500 iters, archivo 5k líneas ~190KB)

| Ruta | Mediana | Nota |
|---|---|---|
| rope from_str 5k | 145 µs | carga de archivo entero |
| line_text línea media | 667 ns | **hottest alloc**: clona String por llamada — se usa en clamp por movimiento y draw por línea/pane |
| mv cursor ±1 (clamp) | 541 ns | pasa por line_len → line_text ×2 |
| insert+backspace (par undo) | 5.0 µs | incluye snap-rope-clone en boundary |
| undo ciclo (snap+pop) | 2.0 µs | pop a secas era mayormente no-op (review); ciclo honesto = snap-rope-clone + restore |
| on_key j (keymap→dispatch→follow) | 1.3 µs | 100% de input-path |
| lsp roundtrip pos (UTF-16) | 1.7 µs | |
| parse_diagnostics 300 | 236 µs | solo en publish |
| draw frame 5k (120×40) | 172 µs | solo con need_draw — cap teórico ~5-6k fps |

| comment_toggle 500 líneas (ida+vuelta) | 2.0 ms | replace_range + snap por línea |
| replace_all 5k (~1.2k matches) | 46.5 ms | síncrono — sobre el umbral de 50ms al doblar tamaño: async queda para lote 2 |

## Roadmap de optimización (por coste real medido)

1. **line_text sin alloc** — `line_len` ya clona String por call; mover el cursor
   con `j` en archivos gordos hace 2 allocs/movimiento. Fix: contar graphemes
   caminando `chunks()` sin colección. Est. 667ns → ~50-100ns.
2. **Presupuesto de memoria en undo** — 100 snaps × rope completa; buffer de
   190KB = hasta ~19MB por buffer. Fix: cap por bytes (drop más viejo) o
   diff-based undo cuando midamos que duele.
3. **draw**: ya gated por need_draw; el diff per-cell de ratatui hace la parte
   pesada. No tocar salvo medición contraria.
4. **parse_diagnostics**: 236µs solo en publish — no está en el input path. No tocar.
5. **AI/LSP**: threads + mpsc ya; el main loop nunca bloquea (bench de on_key
   confirma que ni clavado a un server puede frenar el input path).
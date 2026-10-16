//! Panes: árbol binario — split, close (medio/raíz), layout, relink.

use mentl::panes::{Dir, Panes};
use ratatui::layout::Rect;

fn area() -> Rect {
    Rect::new(0, 0, 80, 24)
}

#[test]
fn split_vert_layout_dos_leafs() {
    let mut p = Panes::first(0);
    p.split(Dir::Vert);
    let mut rects = Vec::new();
    let mut seps = Vec::new();
    p.layout(area(), &mut rects, &mut seps);
    assert_eq!(rects.len(), 2);
    // lhs a la izquierda de rhs, sin solape
    let (a, b) = (rects[0].1, rects[1].1);
    assert_eq!(a.x + a.width, b.x, "lhs | rhs");
    assert_eq!(a.x + a.width + b.x + b.width - b.x, 80);
    assert_eq!(seps.len(), 1, "un separador vertical");
}

#[test]
fn split_hierz_apila_arriva_abajo() {
    let mut p = Panes::first(0);
    p.split(Dir::Horiz);
    let mut rects = Vec::new();
    let mut seps = Vec::new();
    p.layout(area(), &mut rects, &mut seps);
    assert_eq!(rects.len(), 2);
    assert_eq!(rects[0].1.y + rects[0].1.height, rects[1].1.y);
}

#[test]
fn close_leaf_medio_relink_al_hermano() {
    let mut p = Panes::first(0);
    p.split(Dir::Vert); // panes 0 y 1; split crea y activa 1
    p.split(Dir::Vert); // pane 2 (activo)
    assert_eq!(p.active, 2);
    let closed = p.close();
    assert_eq!(closed, Some(1), "foco al hermano");
    let mut rects = Vec::new();
    let mut seps = Vec::new();
    p.layout(area(), &mut rects, &mut seps);
    assert_eq!(rects.len(), 2, "el subárbol del hermano sobrevive");
    // el pane huérfano (2) no aparece en el layout
    assert!(!rects.iter().any(|(id, _)| *id == 2));
}

#[test]
fn close_root_leaf_es_noop() {
    let mut p = Panes::first(0);
    assert_eq!(p.close(), None, "último pane no se cierra");
    let mut rects = Vec::new();
    let mut seps = Vec::new();
    p.layout(area(), &mut rects, &mut seps);
    assert_eq!(rects.len(), 1);
}

#[test]
fn buffer_ids_estables_tras_close() {
    let mut p = Panes::first(5);
    p.split(Dir::Vert); // ambos panes comparten buffer 5 (split comparte)
    let new_buffer = p.panes[p.active].buffer;
    assert_eq!(new_buffer, 5, "split comparte buffer");
    let closed = p.close();
    assert!(closed.is_some());
    assert_eq!(p.panes[p.active].buffer, 5, "buffer id intacto");
}

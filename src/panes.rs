//! Panes: árbol binario de splits. Cada hoja = pane con su buffer y su View.
//! Cerrar = el split padre pasa a ser el hermano. Nodos huérfanos quedan en
//! Vec sin referencia — nunca se alcanzan ni compactan.

use crate::view::View;
use ratatui::layout::Rect;

pub type PaneId = usize;
pub type NodeId = usize;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dir {
    /// columnas: lhs | rhs
    Vert,
    /// filas: lhs arriba, rhs abajo
    Horiz,
}

pub struct Pane {
    pub buffer: usize,
    pub view: View,
}

pub enum Node {
    Split { dir: Dir, lhs: NodeId, rhs: NodeId },
    Leaf(PaneId),
}

pub struct Panes {
    pub panes: Vec<Pane>,
    pub nodes: Vec<Node>,
    root: NodeId,
    pub active: PaneId,
}

impl Panes {
    pub fn first(buffer_idx: usize) -> Self {
        Self {
            panes: vec![Pane {
                buffer: buffer_idx,
                view: View::default(),
            }],
            nodes: vec![Node::Leaf(0)],
            root: 0,
            active: 0,
        }
    }

    pub fn buffer_of(&self, pane: PaneId) -> usize {
        self.panes[pane].buffer
    }

    /// Divide el pane activo en dir: lhs = pane actual, rhs = pane nuevo con
    /// el mismo buffer. Foco en el nuevo.
    pub fn split(&mut self, dir: Dir) {
        let buffer = self.panes[self.active].buffer;
        let new_pane = self.panes.len();
        self.panes.push(Pane {
            buffer,
            view: View::default(),
        });
        let lhs_leaf = self.nodes.len(); // leaf que conserva el pane actual
        self.nodes.push(Node::Leaf(self.active));
        let rhs_leaf = self.nodes.len(); // leaf del pane nuevo
        self.nodes.push(Node::Leaf(new_pane));
        let old_leaf = self
            .leaf_of(self.root, self.active)
            .expect("pane activo debe tener leaf en el árbol");
        self.nodes[old_leaf] = Node::Split {
            dir,
            lhs: lhs_leaf,
            rhs: rhs_leaf,
        };
        self.active = new_pane;
    }

    /// Cierra el pane activo. None si es el último.
    pub fn close(&mut self) -> Option<PaneId> {
        // leaf dentro del árbol del pane activo
        let leaf = self.leaf_of(self.root, self.active)?;
        if self.root == leaf {
            self.root = leaf; // último pane — sin close
            return None;
        }
        let (parent, is_rhs) = self.parent_of(self.root, leaf)?;
        let sibling = match self.nodes[parent] {
            Node::Split { lhs, rhs, .. } => {
                if is_rhs {
                    lhs
                } else {
                    rhs
                }
            }
            Node::Leaf(_) => unreachable!("parent es Leaf"),
        };
        if parent == self.root {
            self.root = sibling;
        } else {
            // el abuelo apunta al sibling en el lado correspondiente
            let (grand, g_side) = self
                .parent_of(self.root, parent)
                .expect("parent tiene padre si no es root");
            self.relink(grand, g_side, sibling);
        }
        let active = self.first_leaf_pane(sibling);
        self.active = active;
        Some(active)
    }

    fn relink(&mut self, node: NodeId, side: bool, new_child: NodeId) {
        if let Node::Split { lhs, rhs, .. } = &mut self.nodes[node] {
            if side {
                *rhs = new_child;
            } else {
                *lhs = new_child;
            }
        } else {
            unreachable!("relink sobre Split");
        }
    }

    /// Id del nodo leaf que posee `pane` dentro del árbol desde `node`.
    fn leaf_of(&self, node: NodeId, pane: PaneId) -> Option<NodeId> {
        match &self.nodes[node] {
            Node::Leaf(p) if *p == pane => Some(node),
            Node::Leaf(_) => None,
            Node::Split { lhs, rhs, .. } => self
                .leaf_of(*lhs, pane)
                .or_else(|| self.leaf_of(*rhs, pane)),
        }
    }

    /// (padre, es_lado_derecho) de `target` dentro del árbol.
    fn parent_of(&self, node: NodeId, target: NodeId) -> Option<(NodeId, bool)> {
        if let Node::Split { lhs, rhs, .. } = &self.nodes[node] {
            if *lhs == target {
                return Some((node, false));
            }
            if *rhs == target {
                return Some((node, true));
            }
            return self
                .parent_of(*lhs, target)
                .or_else(|| self.parent_of(*rhs, target));
        }
        None
    }

    /// PaneId del primer leaf del subárbol.
    fn first_leaf_pane(&self, node: NodeId) -> PaneId {
        match &self.nodes[node] {
            Node::Leaf(p) => *p,
            Node::Split { lhs, .. } => self.first_leaf_pane(*lhs),
        }
    }

    /// Rects por pane en orden; además emite separadores (línea frontera) en
    /// cada split para que draw los pinte.
    pub fn layout(
        &self,
        area: Rect,
        out: &mut Vec<(PaneId, Rect)>,
        seps: &mut Vec<(Dir, u16, Rect)>,
    ) {
        self.layout_node(self.root, area, out, seps);
    }

    fn layout_node(
        &self,
        node: NodeId,
        rect: Rect,
        out: &mut Vec<(PaneId, Rect)>,
        seps: &mut Vec<(Dir, u16, Rect)>,
    ) {
        match &self.nodes[node] {
            Node::Leaf(p) => out.push((*p, rect)),
            Node::Split { dir, lhs, rhs } => match dir {
                Dir::Vert => {
                    let mid = rect.x + rect.width / 2;
                    seps.push((Dir::Vert, mid, rect));
                    let (a, b) = Self::split_rect(rect, Dir::Vert);
                    self.layout_node(*lhs, a, out, seps);
                    self.layout_node(*rhs, b, out, seps);
                }
                Dir::Horiz => {
                    let mid = rect.y + rect.height / 2;
                    seps.push((Dir::Horiz, mid, rect));
                    let (a, b) = Self::split_rect(rect, Dir::Horiz);
                    self.layout_node(*lhs, a, out, seps);
                    self.layout_node(*rhs, b, out, seps);
                }
            },
        }
    }

    fn split_rect(rect: Rect, dir: Dir) -> (Rect, Rect) {
        match dir {
            Dir::Vert => (
                Rect {
                    width: rect.width / 2,
                    ..rect
                },
                Rect {
                    x: rect.x + rect.width / 2,
                    width: rect.width - rect.width / 2,
                    ..rect
                },
            ),
            Dir::Horiz => (
                Rect {
                    height: rect.height / 2,
                    ..rect
                },
                Rect {
                    y: rect.y + rect.height / 2,
                    height: rect.height - rect.height / 2,
                    ..rect
                },
            ),
        }
    }
}

/// Dirección de foco — mapea a Dir según la geometría adyacente.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Left,
    Right,
    Up,
    Down,
}

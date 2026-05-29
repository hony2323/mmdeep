//! Compact in-memory graph store.
//!
//! Node names are interned to `u32` ids. Edges live in flat parallel arrays.
//! Forward and reverse adjacency are kept in CSR (compressed-sparse-row) form
//! so neighbour lookups are cache-friendly and allocation-free. At 1M edges
//! the whole structure is only tens of MB.

use serde::Serialize;
use std::collections::HashMap;

pub type NodeId = u32;
pub type EdgeId = u32;

pub const NO_LABEL: u32 = u32::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Shape {
    Default,
    Rect,
    RoundRect,
    Stadium,
    Subroutine,
    Cylinder,
    Circle,
    Diamond,
    Hexagon,
    Parallelogram,
    Trapezoid,
    Asymmetric,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum EdgeStyle {
    Normal,
    Thick,
    Dotted,
}

/// A nesting of nodes declared inside a `subgraph ... end` block.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Subgraph {
    pub id: String,
    pub title: String,
    pub nodes: Vec<NodeId>,
}

#[derive(Default)]
pub struct GraphBuilder {
    names: Vec<String>,
    name_index: HashMap<String, NodeId>,
    text: Vec<Option<String>>,
    shape: Vec<Shape>,

    src: Vec<NodeId>,
    dst: Vec<NodeId>,
    edge_style: Vec<EdgeStyle>,
    edge_label: Vec<u32>,
    label_pool: Vec<String>,

    pub direction: String,
    subgraphs: Vec<Subgraph>,
}

impl GraphBuilder {
    pub fn new() -> Self {
        Self {
            direction: "TD".to_string(),
            ..Default::default()
        }
    }

    /// Intern a node name, creating it if unseen.
    pub fn intern(&mut self, name: &str) -> NodeId {
        if let Some(&id) = self.name_index.get(name) {
            return id;
        }
        let id = self.names.len() as NodeId;
        self.names.push(name.to_string());
        self.text.push(None);
        self.shape.push(Shape::Default);
        self.name_index.insert(name.to_string(), id);
        id
    }

    pub fn set_node(&mut self, id: NodeId, shape: Shape, text: Option<String>) {
        let i = id as usize;
        if shape != Shape::Default {
            self.shape[i] = shape;
        }
        if let Some(t) = text {
            self.text[i] = Some(t);
        }
    }

    pub fn add_edge(&mut self, a: NodeId, b: NodeId, style: EdgeStyle, label: Option<String>) {
        self.src.push(a);
        self.dst.push(b);
        self.edge_style.push(style);
        match label {
            Some(l) => {
                let idx = self.label_pool.len() as u32;
                self.label_pool.push(l);
                self.edge_label.push(idx);
            }
            None => self.edge_label.push(NO_LABEL),
        }
    }

    pub fn add_subgraph(&mut self, sg: Subgraph) {
        self.subgraphs.push(sg);
    }

    /// Finalise: build CSR adjacency for both directions.
    pub fn build(self) -> Graph {
        let n = self.names.len();
        let m = self.src.len();

        let (out_offsets, out_edges) = build_csr(n, m, &self.src);
        let (in_offsets, in_edges) = build_csr(n, m, &self.dst);

        Graph {
            names: self.names,
            name_index: self.name_index,
            text: self.text,
            shape: self.shape,
            src: self.src,
            dst: self.dst,
            edge_style: self.edge_style,
            edge_label: self.edge_label,
            label_pool: self.label_pool,
            direction: self.direction,
            subgraphs: self.subgraphs,
            out_offsets,
            out_edges,
            in_offsets,
            in_edges,
        }
    }
}

/// Build a CSR (offsets, edge-ids) keyed by the `key` endpoint of each edge.
fn build_csr(n: usize, m: usize, key: &[NodeId]) -> (Vec<u32>, Vec<EdgeId>) {
    let mut offsets = vec![0u32; n + 1];
    for &k in key {
        offsets[k as usize + 1] += 1;
    }
    for i in 0..n {
        offsets[i + 1] += offsets[i];
    }
    let mut edges = vec![0u32; m];
    let mut cursor = offsets.clone();
    for (e, &k) in key.iter().enumerate() {
        let slot = &mut cursor[k as usize];
        edges[*slot as usize] = e as EdgeId;
        *slot += 1;
    }
    (offsets, edges)
}

/// The finalised, queryable graph.
pub struct Graph {
    pub names: Vec<String>,
    pub name_index: HashMap<String, NodeId>,
    pub text: Vec<Option<String>>,
    pub shape: Vec<Shape>,

    pub src: Vec<NodeId>,
    pub dst: Vec<NodeId>,
    pub edge_style: Vec<EdgeStyle>,
    pub edge_label: Vec<u32>,
    pub label_pool: Vec<String>,

    pub direction: String,
    pub subgraphs: Vec<Subgraph>,

    out_offsets: Vec<u32>,
    out_edges: Vec<EdgeId>,
    in_offsets: Vec<u32>,
    in_edges: Vec<EdgeId>,
}

impl Graph {
    pub fn node_count(&self) -> usize {
        self.names.len()
    }
    pub fn edge_count(&self) -> usize {
        self.src.len()
    }

    pub fn name(&self, id: NodeId) -> &str {
        &self.names[id as usize]
    }
    pub fn id_of(&self, name: &str) -> Option<NodeId> {
        self.name_index.get(name).copied()
    }
    pub fn shape(&self, id: NodeId) -> Shape {
        self.shape[id as usize]
    }
    /// Display text for a node: its explicit label if any, else its name.
    pub fn display(&self, id: NodeId) -> &str {
        self.text[id as usize]
            .as_deref()
            .unwrap_or(&self.names[id as usize])
    }

    pub fn edge_endpoints(&self, e: EdgeId) -> (NodeId, NodeId) {
        (self.src[e as usize], self.dst[e as usize])
    }
    pub fn edge_label(&self, e: EdgeId) -> Option<&str> {
        let l = self.edge_label[e as usize];
        if l == NO_LABEL {
            None
        } else {
            Some(&self.label_pool[l as usize])
        }
    }
    pub fn edge_style(&self, e: EdgeId) -> EdgeStyle {
        self.edge_style[e as usize]
    }

    /// Outgoing edge ids for a node.
    pub fn out_edges(&self, id: NodeId) -> &[EdgeId] {
        let i = id as usize;
        &self.out_edges[self.out_offsets[i] as usize..self.out_offsets[i + 1] as usize]
    }
    /// Incoming edge ids for a node.
    pub fn in_edges(&self, id: NodeId) -> &[EdgeId] {
        let i = id as usize;
        &self.in_edges[self.in_offsets[i] as usize..self.in_offsets[i + 1] as usize]
    }

    pub fn out_degree(&self, id: NodeId) -> usize {
        let i = id as usize;
        (self.out_offsets[i + 1] - self.out_offsets[i]) as usize
    }
    pub fn in_degree(&self, id: NodeId) -> usize {
        let i = id as usize;
        (self.in_offsets[i + 1] - self.in_offsets[i]) as usize
    }
    pub fn degree(&self, id: NodeId) -> usize {
        self.out_degree(id) + self.in_degree(id)
    }

    /// Approximate retained heap size in bytes (for the perf HUD / benchmarks).
    pub fn approx_bytes(&self) -> usize {
        let names: usize = self.names.iter().map(|s| s.len() + 24).sum();
        let labels: usize = self.label_pool.iter().map(|s| s.len() + 24).sum();
        let text: usize = self
            .text
            .iter()
            .map(|t| t.as_ref().map_or(0, |s| s.len()) + 16)
            .sum();
        names
            + labels
            + text
            + self.src.len() * 4
            + self.dst.len() * 4
            + self.edge_style.len()
            + self.edge_label.len() * 4
            + self.out_offsets.len() * 4
            + self.out_edges.len() * 4
            + self.in_offsets.len() * 4
            + self.in_edges.len() * 4
            + self.shape.len()
    }
}

//! Graph discovery — the "pagination for graphs" core.
//!
//! Instead of ever materialising the whole graph, the UI starts from seed
//! nodes and expands neighbourhoods on demand. `expand` runs a budgeted BFS:
//! it returns at most `limit` nodes, and where a node's neighbours were cut off
//! by the budget it emits a `truncated` count so the UI can show a `+N more`
//! stub that the user can click to page in the rest.

use crate::store::{EdgeStyle, Graph, NodeId, Shape};
use serde::Serialize;
use std::collections::{HashSet, VecDeque};

#[derive(Serialize)]
pub struct SubNode {
    pub id: NodeId,
    pub name: String,
    pub label: String,
    pub shape: Shape,
    pub out_degree: u32,
    pub in_degree: u32,
    /// Neighbours not included in this response (frontier truncated by budget).
    pub truncated: u32,
    /// Layout coordinates (filled in by the layout pass; 0,0 until then).
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub y: f32,
}

#[derive(Serialize)]
pub struct SubEdge {
    pub source: NodeId,
    pub target: NodeId,
    pub label: Option<String>,
    pub style: EdgeStyle,
}

#[derive(Serialize)]
pub struct SubGraphPayload {
    pub nodes: Vec<SubNode>,
    pub edges: Vec<SubEdge>,
    pub truncated: bool,
}

/// Direction to follow when expanding.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Out,
    In,
    Both,
}

impl Dir {
    pub fn from_str(s: &str) -> Dir {
        match s {
            "out" => Dir::Out,
            "in" => Dir::In,
            _ => Dir::Both,
        }
    }
}

/// Default discovery seeds: source nodes (in-degree 0). If the graph has none
/// (e.g. fully cyclic), fall back to the highest-degree nodes.
pub fn roots(g: &Graph, limit: usize) -> Vec<NodeId> {
    let mut sources: Vec<NodeId> = (0..g.node_count() as NodeId)
        .filter(|&n| g.in_degree(n) == 0)
        .collect();
    if sources.is_empty() {
        let mut all: Vec<NodeId> = (0..g.node_count() as NodeId).collect();
        all.sort_unstable_by_key(|&n| std::cmp::Reverse(g.degree(n)));
        all.truncate(limit);
        return all;
    }
    // Most-connected sources first — they make the most useful entry points.
    sources.sort_unstable_by_key(|&n| std::cmp::Reverse(g.out_degree(n)));
    sources.truncate(limit);
    sources
}

/// Budgeted BFS from one or more seed nodes.
pub fn expand(
    g: &Graph,
    seeds: &[NodeId],
    depth: u32,
    limit: usize,
    dir: Dir,
) -> SubGraphPayload {
    let mut visited: HashSet<NodeId> = HashSet::new();
    let mut order: Vec<NodeId> = Vec::new();
    let mut queue: VecDeque<(NodeId, u32)> = VecDeque::new();

    for &s in seeds {
        if (s as usize) < g.node_count() && visited.insert(s) {
            order.push(s);
            queue.push_back((s, 0));
        }
    }

    let mut budget_hit = false;
    while let Some((node, d)) = queue.pop_front() {
        if d >= depth {
            continue;
        }
        // Visit neighbours; stop adding new nodes once the budget is reached,
        // but keep walking so we can count what we left behind (`truncated`).
        let push_neighbor = |nb: NodeId,
                             q: &mut VecDeque<(NodeId, u32)>,
                             ord: &mut Vec<NodeId>,
                             vis: &mut HashSet<NodeId>,
                             hit: &mut bool| {
            if vis.contains(&nb) {
                return;
            }
            if ord.len() >= limit {
                *hit = true;
                return;
            }
            vis.insert(nb);
            ord.push(nb);
            q.push_back((nb, d + 1));
        };

        if matches!(dir, Dir::Out | Dir::Both) {
            for &e in g.out_edges(node) {
                let (_, b) = g.edge_endpoints(e);
                push_neighbor(b, &mut queue, &mut order, &mut visited, &mut budget_hit);
            }
        }
        if matches!(dir, Dir::In | Dir::Both) {
            for &e in g.in_edges(node) {
                let (a, _) = g.edge_endpoints(e);
                push_neighbor(a, &mut queue, &mut order, &mut visited, &mut budget_hit);
            }
        }
    }

    build_payload(g, &order, &visited, budget_hit)
}

/// Expand a single node's neighbours with offset/count paging — this backs the
/// `+N more` stub. Returns neighbours in adjacency order so paging is stable.
pub fn neighbors_page(
    g: &Graph,
    node: NodeId,
    dir: Dir,
    offset: usize,
    count: usize,
) -> SubGraphPayload {
    if node as usize >= g.node_count() {
        return SubGraphPayload {
            nodes: vec![],
            edges: vec![],
            truncated: false,
        };
    }
    let mut neigh: Vec<NodeId> = Vec::new();
    if matches!(dir, Dir::Out | Dir::Both) {
        for &e in g.out_edges(node) {
            neigh.push(g.edge_endpoints(e).1);
        }
    }
    if matches!(dir, Dir::In | Dir::Both) {
        for &e in g.in_edges(node) {
            neigh.push(g.edge_endpoints(e).0);
        }
    }
    let total = neigh.len();
    let page: Vec<NodeId> = neigh.into_iter().skip(offset).take(count).collect();

    let mut keep: HashSet<NodeId> = page.iter().copied().collect();
    keep.insert(node);
    let mut order = vec![node];
    order.extend(page.iter().copied());
    let mut payload = build_payload(g, &order, &keep, false);
    payload.truncated = offset + count < total;
    payload
}

/// Build a serialisable payload of nodes/edges restricted to `keep`.
fn build_payload(
    g: &Graph,
    order: &[NodeId],
    keep: &HashSet<NodeId>,
    budget_hit: bool,
) -> SubGraphPayload {
    let mut nodes = Vec::with_capacity(order.len());
    for &id in order {
        // Count neighbours that did NOT make it into this view.
        let mut truncated = 0u32;
        for &e in g.out_edges(id) {
            if !keep.contains(&g.edge_endpoints(e).1) {
                truncated += 1;
            }
        }
        for &e in g.in_edges(id) {
            if !keep.contains(&g.edge_endpoints(e).0) {
                truncated += 1;
            }
        }
        nodes.push(SubNode {
            id,
            name: g.name(id).to_string(),
            label: g.display(id).to_string(),
            shape: g.shape(id),
            out_degree: g.out_degree(id) as u32,
            in_degree: g.in_degree(id) as u32,
            truncated,
            x: 0.0,
            y: 0.0,
        });
    }

    // Include every edge whose endpoints are both in the kept set (dedup by id).
    let mut edges = Vec::new();
    let mut seen_edges: HashSet<u32> = HashSet::new();
    for &id in order {
        for &e in g.out_edges(id) {
            let (a, b) = g.edge_endpoints(e);
            if keep.contains(&b) && seen_edges.insert(e) {
                edges.push(SubEdge {
                    source: a,
                    target: b,
                    label: g.edge_label(e).map(|s| s.to_string()),
                    style: g.edge_style(e),
                });
            }
        }
        for &e in g.in_edges(id) {
            let (a, b) = g.edge_endpoints(e);
            if keep.contains(&a) && seen_edges.insert(e) {
                edges.push(SubEdge {
                    source: a,
                    target: b,
                    label: g.edge_label(e).map(|s| s.to_string()),
                    style: g.edge_style(e),
                });
            }
        }
    }

    SubGraphPayload {
        nodes,
        edges,
        truncated: budget_hit,
    }
}

/// Case-insensitive substring search over node names and labels.
pub fn search(g: &Graph, query: &str, limit: usize) -> Vec<SubNode> {
    let q = query.to_ascii_lowercase();
    let mut out = Vec::new();
    for id in 0..g.node_count() as NodeId {
        if g.name(id).to_ascii_lowercase().contains(&q)
            || g.display(id).to_ascii_lowercase().contains(&q)
        {
            out.push(SubNode {
                id,
                name: g.name(id).to_string(),
                label: g.display(id).to_string(),
                shape: g.shape(id),
                out_degree: g.out_degree(id) as u32,
                in_degree: g.in_degree(id) as u32,
                truncated: 0,
                x: 0.0,
                y: 0.0,
            });
            if out.len() >= limit {
                break;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::Parser;

    fn graph(s: &str) -> Graph {
        Parser::new().parse_str(s).build()
    }

    #[test]
    fn roots_are_sources() {
        let g = graph("flowchart TD\nA --> B\nA --> C\nB --> D\n");
        let r = roots(&g, 10);
        assert_eq!(r.len(), 1);
        assert_eq!(g.name(r[0]), "A");
    }

    #[test]
    fn expand_respects_depth() {
        let g = graph("flowchart TD\nA --> B\nB --> C\nC --> D\n");
        let a = g.id_of("A").unwrap();
        let p = expand(&g, &[a], 1, 100, Dir::Out);
        // depth 1 from A reaches B only
        let names: Vec<&str> = p.nodes.iter().map(|n| n.name.as_str()).collect();
        assert!(names.contains(&"A") && names.contains(&"B"));
        assert!(!names.contains(&"C"));
    }

    #[test]
    fn expand_respects_limit_and_reports_truncation() {
        // star: hub with 50 children
        let mut s = String::from("flowchart TD\n");
        for i in 0..50 {
            s.push_str(&format!("hub --> c{i}\n"));
        }
        let g = graph(&s);
        let hub = g.id_of("hub").unwrap();
        let p = expand(&g, &[hub], 1, 10, Dir::Out);
        assert!(p.nodes.len() <= 10);
        assert!(p.truncated);
        let hub_node = p.nodes.iter().find(|n| n.name == "hub").unwrap();
        assert!(hub_node.truncated > 0);
    }

    #[test]
    fn neighbors_paging() {
        let mut s = String::from("flowchart TD\n");
        for i in 0..20 {
            s.push_str(&format!("hub --> c{i}\n"));
        }
        let g = graph(&s);
        let hub = g.id_of("hub").unwrap();
        let p0 = neighbors_page(&g, hub, Dir::Out, 0, 5);
        // 5 neighbours + hub itself
        assert_eq!(p0.nodes.len(), 6);
        assert!(p0.truncated);
        let p1 = neighbors_page(&g, hub, Dir::Out, 15, 5);
        assert!(!p1.truncated);
    }
}

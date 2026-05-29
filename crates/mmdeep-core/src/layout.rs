//! Layout engines.
//!
//! * [`layered`] positions a small discovery sub-graph in readable top-down
//!   layers (a light Sugiyama: rank by longest path, order within rank). This
//!   runs on the handful-to-few-thousand nodes a discovery view ever holds.
//! * [`global_force`] computes a whole-graph force-directed layout for the
//!   overview using a Barnes-Hut quadtree so it scales to ~1M nodes. Results
//!   are meant to be cached to disk (see `cache` in lib.rs).

use crate::discovery::SubGraphPayload;
use crate::store::{Graph, NodeId};
use rayon::prelude::*;
use std::collections::HashMap;

const LAYER_GAP: f32 = 120.0;
const NODE_GAP: f32 = 80.0;

/// Assign layered coordinates in-place to a discovery payload.
pub fn layered(payload: &mut SubGraphPayload) {
    let n = payload.nodes.len();
    if n == 0 {
        return;
    }
    // local index: global NodeId -> position in payload.nodes
    let mut idx: HashMap<NodeId, usize> = HashMap::with_capacity(n);
    for (i, node) in payload.nodes.iter().enumerate() {
        idx.insert(node.id, i);
    }

    // local adjacency restricted to the payload
    let mut succ: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut indeg: Vec<u32> = vec![0; n];
    for e in &payload.edges {
        if let (Some(&a), Some(&b)) = (idx.get(&e.source), idx.get(&e.target)) {
            if a != b {
                succ[a].push(b);
                indeg[b] += 1;
            }
        }
    }

    // rank = longest path from a local source; cycle-safe via Kahn-style BFS
    // with a fallback that still ranks nodes left in cycles.
    let mut rank = vec![0u32; n];
    let mut remaining = indeg.clone();
    let mut queue: Vec<usize> = (0..n).filter(|&i| remaining[i] == 0).collect();
    let mut processed = vec![false; n];
    let mut head = 0;
    while head < queue.len() {
        let u = queue[head];
        head += 1;
        processed[u] = true;
        for &v in &succ[u] {
            rank[v] = rank[v].max(rank[u] + 1);
            if remaining[v] > 0 {
                remaining[v] -= 1;
                if remaining[v] == 0 {
                    queue.push(v);
                }
            }
        }
    }
    // Any nodes left (inside cycles) get ranked by BFS distance from processed.
    for i in 0..n {
        if !processed[i] {
            // place after the max rank of its already-ranked predecessors
            rank[i] = rank[i].max(1);
        }
    }

    // group by rank and spread within each layer, centred on x=0
    let max_rank = *rank.iter().max().unwrap_or(&0);
    let mut by_rank: Vec<Vec<usize>> = vec![Vec::new(); (max_rank + 1) as usize];
    for i in 0..n {
        by_rank[rank[i] as usize].push(i);
    }
    for (r, members) in by_rank.iter().enumerate() {
        let count = members.len();
        let width = (count.saturating_sub(1)) as f32 * NODE_GAP;
        for (k, &i) in members.iter().enumerate() {
            payload.nodes[i].x = k as f32 * NODE_GAP - width / 2.0;
            payload.nodes[i].y = r as f32 * LAYER_GAP;
        }
    }
}

/// Compact quadtree node. `first_child == 0` means "no children" (index 0 is
/// always the root, so it can never legitimately be a child slot). Children are
/// allocated four-at-a-time and contiguously, at `first_child .. first_child+4`.
#[derive(Clone, Copy)]
struct QuadNode {
    cx: f32,
    cy: f32,
    half: f32,
    mass: f32,
    mass_x: f32,
    mass_y: f32,
    body: i32,
    first_child: u32,
}

impl QuadNode {
    #[inline]
    fn empty(cx: f32, cy: f32, half: f32) -> Self {
        QuadNode {
            cx,
            cy,
            half,
            mass: 0.0,
            mass_x: 0.0,
            mass_y: 0.0,
            body: -1,
            first_child: 0,
        }
    }
}

/// Barnes-Hut quadtree backed by a single reusable arena (no per-iteration or
/// per-node heap allocation in the steady state).
struct QuadTree {
    nodes: Vec<QuadNode>,
}

impl QuadTree {
    fn with_capacity(cap: usize) -> Self {
        QuadTree {
            nodes: Vec::with_capacity(cap),
        }
    }

    fn reset(&mut self, cx: f32, cy: f32, half: f32) {
        self.nodes.clear();
        self.nodes.push(QuadNode::empty(cx, cy, half));
    }

    #[inline]
    fn quadrant(cx: f32, cy: f32, x: f32, y: f32) -> usize {
        let east = (x >= cx) as usize;
        let south = (y >= cy) as usize;
        south * 2 + east
    }

    fn insert(&mut self, body: usize, pos: &[[f32; 2]]) {
        let (x, y) = (pos[body][0], pos[body][1]);
        let mut cur = 0usize;
        loop {
            let n = &mut self.nodes[cur];
            n.mass += 1.0;
            n.mass_x += x;
            n.mass_y += y;

            if n.first_child == 0 {
                if n.body == -1 {
                    n.body = body as i32;
                    return;
                }
                // leaf already holds a body — subdivide and re-seat it
                let old = n.body;
                let (pcx, pcy, phalf) = (n.cx, n.cy, n.half);
                let first = self.make_children(pcx, pcy, phalf);
                self.nodes[cur].body = -1;
                self.nodes[cur].first_child = first as u32;
                let oq = Self::quadrant(pcx, pcy, pos[old as usize][0], pos[old as usize][1]);
                let child = first + oq;
                let cn = &mut self.nodes[child];
                cn.mass += 1.0;
                cn.mass_x += pos[old as usize][0];
                cn.mass_y += pos[old as usize][1];
                cn.body = old;
            }
            let first = self.nodes[cur].first_child as usize;
            let q = Self::quadrant(self.nodes[cur].cx, self.nodes[cur].cy, x, y);
            cur = first + q;
        }
    }

    fn make_children(&mut self, cx: f32, cy: f32, half: f32) -> usize {
        let h = half / 2.0;
        let first = self.nodes.len();
        self.nodes.push(QuadNode::empty(cx - h, cy - h, h));
        self.nodes.push(QuadNode::empty(cx + h, cy - h, h));
        self.nodes.push(QuadNode::empty(cx - h, cy + h, h));
        self.nodes.push(QuadNode::empty(cx + h, cy + h, h));
        first
    }

    /// Accumulate repulsive force on a body at (x,y). `stack` is a caller-owned
    /// scratch buffer reused across calls to avoid per-node allocation.
    #[inline]
    fn force(
        &self,
        x: f32,
        y: f32,
        theta2: f32,
        k2: f32,
        stack: &mut Vec<u32>,
    ) -> (f32, f32) {
        let (mut fx, mut fy) = (0.0f32, 0.0f32);
        stack.clear();
        stack.push(0);
        while let Some(ni) = stack.pop() {
            let node = &self.nodes[ni as usize];
            if node.mass == 0.0 {
                continue;
            }
            let inv = 1.0 / node.mass;
            let dx = x - node.mass_x * inv;
            let dy = y - node.mass_y * inv;
            let dist2 = dx * dx + dy * dy + 0.01;
            let size = node.half * 2.0;
            if node.first_child == 0 || (size * size) < theta2 * dist2 {
                let force = k2 * node.mass / dist2;
                let dist = dist2.sqrt();
                fx += force * dx / dist;
                fy += force * dy / dist;
            } else {
                let f = node.first_child;
                stack.push(f);
                stack.push(f + 1);
                stack.push(f + 2);
                stack.push(f + 3);
            }
        }
        (fx, fy)
    }
}

/// Compute a whole-graph force-directed layout. Returns `[x,y]` per node id.
/// `iterations` trades quality for speed; 60-150 is reasonable.
pub fn global_force(g: &Graph, iterations: usize) -> Vec<[f32; 2]> {
    let n = g.node_count();
    let mut pos = vec![[0.0f32; 2]; n];
    if n == 0 {
        return pos;
    }
    // Deterministic spiral/grid seed so runs are reproducible (cacheable).
    let side = (n as f32).sqrt().ceil() as usize;
    let spread = (n as f32).sqrt() * 30.0;
    for i in 0..n {
        let gx = (i % side) as f32 / side as f32 - 0.5;
        let gy = (i / side) as f32 / side as f32 - 0.5;
        // jitter from a hash so coincident grid points separate
        let h = (i.wrapping_mul(2654435761)) as f32;
        pos[i][0] = gx * spread + (h % 17.0 - 8.0);
        pos[i][1] = gy * spread + ((h / 17.0) % 17.0 - 8.0);
    }

    let k = spread / (n as f32).sqrt().max(1.0); // ideal edge length
    let k2 = k * k;
    let theta2 = 0.81; // theta^2, theta = 0.9
    let mut temp = spread * 0.1;
    let cooling = temp / (iterations.max(1) as f32);

    let mut disp = vec![[0.0f32; 2]; n];
    // ~1.4 internal nodes per body is a safe arena reservation.
    let mut qt = QuadTree::with_capacity(n * 2);
    for _ in 0..iterations {
        // bounding box
        let (mut minx, mut miny, mut maxx, mut maxy) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for p in &pos {
            minx = minx.min(p[0]);
            miny = miny.min(p[1]);
            maxx = maxx.max(p[0]);
            maxy = maxy.max(p[1]);
        }
        let cx = (minx + maxx) / 2.0;
        let cy = (miny + maxy) / 2.0;
        let half = ((maxx - minx).max(maxy - miny) / 2.0).max(1.0) + 1.0;

        // build quadtree (reusing the arena)
        qt.reset(cx, cy, half);
        for i in 0..n {
            qt.insert(i, &pos);
        }

        // repulsion — parallel, each worker keeps a reusable traversal stack
        let qt_ref = &qt;
        let pos_ref = &pos;
        disp.par_iter_mut().enumerate().for_each_init(
            || Vec::<u32>::with_capacity(128),
            |stack, (i, d)| {
                let (fx, fy) = qt_ref.force(pos_ref[i][0], pos_ref[i][1], theta2, k2, stack);
                *d = [fx, fy];
            },
        );
        // attraction along edges
        for e in 0..g.edge_count() {
            let (a, b) = g.edge_endpoints(e as u32);
            let (a, b) = (a as usize, b as usize);
            let dx = pos[a][0] - pos[b][0];
            let dy = pos[a][1] - pos[b][1];
            let dist = (dx * dx + dy * dy).sqrt() + 0.01;
            let force = dist * dist / k;
            let fx = force * dx / dist;
            let fy = force * dy / dist;
            disp[a][0] -= fx;
            disp[a][1] -= fy;
            disp[b][0] += fx;
            disp[b][1] += fy;
        }
        // integrate with temperature cap
        for i in 0..n {
            let d = (disp[i][0] * disp[i][0] + disp[i][1] * disp[i][1]).sqrt() + 1e-6;
            let limited = d.min(temp);
            pos[i][0] += disp[i][0] / d * limited;
            pos[i][1] += disp[i][1] / d * limited;
        }
        temp = (temp - cooling).max(spread * 0.001);
    }

    // Rescale to a large, centred world so the map has plenty of zoom range:
    // node glyphs keep a roughly constant screen size, so a bigger world means
    // zooming in separates nodes that the force pass packed tightly together.
    //
    // Center on the centroid (not the bbox centre) and size the world by a
    // percentile radius, then clamp rare outliers to the edge. This keeps the
    // dense mass centred and filling the viewport instead of being pushed off to
    // one side by a handful of far-flung nodes.
    let (mut cx, mut cy) = (0.0f64, 0.0f64);
    for p in &pos {
        cx += p[0] as f64;
        cy += p[1] as f64;
    }
    cx /= n as f64;
    cy /= n as f64;
    let (cx, cy) = (cx as f32, cy as f32);

    let mut sq: Vec<f32> = pos
        .iter()
        .map(|p| {
            let dx = p[0] - cx;
            let dy = p[1] - cy;
            dx * dx + dy * dy
        })
        .collect();
    // 98th-percentile radius via quickselect (O(n))
    let k = ((n as f32 * 0.98) as usize).min(n - 1);
    sq.select_nth_unstable_by(k, |a, b| a.partial_cmp(b).unwrap());
    let r98 = sq[k].max(1e-6).sqrt();

    let world = (n as f32).sqrt() * 50.0;
    let half = world / 2.0;
    let scale = (half * 0.9) / r98;
    for p in &mut pos {
        p[0] = ((p[0] - cx) * scale).clamp(-half, half);
        p[1] = ((p[1] - cy) * scale).clamp(-half, half);
    }
    pos
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discovery::{expand, Dir};
    use crate::parser::Parser;

    #[test]
    fn layered_assigns_ranks() {
        let g = Parser::new()
            .parse_str("flowchart TD\nA --> B\nA --> C\nB --> D\nC --> D\n")
            .build();
        let a = g.id_of("A").unwrap();
        let mut p = expand(&g, &[a], 5, 100, Dir::Out);
        layered(&mut p);
        let y = |name: &str| p.nodes.iter().find(|n| n.name == name).unwrap().y;
        assert!(y("A") < y("B"));
        assert!(y("B") < y("D"));
    }

    #[test]
    fn force_layout_runs() {
        let g = Parser::new()
            .parse_str("flowchart TD\nA --> B\nB --> C\nC --> A\nA --> D\n")
            .build();
        let pos = global_force(&g, 30);
        assert_eq!(pos.len(), 4);
        assert!(pos.iter().all(|p| p[0].is_finite() && p[1].is_finite()));
    }
}

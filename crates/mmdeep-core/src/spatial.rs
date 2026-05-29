//! Level-of-detail spatial index for the "map" view.
//!
//! Nodes are bucketed into a uniform grid over the global layout bounds; within
//! each cell they're sorted by importance (degree). A viewport query does a
//! k-way merge across the overlapping cells to return the **most important**
//! nodes in view, up to a budget — so zoomed-out views show major hubs (few,
//! labelled) and zoomed-in views show local detail, exactly like map tiles
//! revealing streets as you zoom.

use crate::store::NodeId;
use std::cmp::Ordering;
use std::collections::BinaryHeap;

pub struct SpatialIndex {
    positions: Vec<[f32; 2]>,
    importance: Vec<u32>,
    minx: f32,
    miny: f32,
    maxx: f32,
    maxy: f32,
    cols: usize,
    rows: usize,
    cell_w: f32,
    cell_h: f32,
    /// node ids per cell, each sorted by importance descending
    cells: Vec<Vec<NodeId>>,
}

/// Heap entry for the k-way merge: order by importance, then node id.
struct HeapItem {
    importance: u32,
    cell: usize,
    cursor: usize,
    node: NodeId,
}
impl PartialEq for HeapItem {
    fn eq(&self, other: &Self) -> bool {
        self.importance == other.importance && self.node == other.node
    }
}
impl Eq for HeapItem {}
impl Ord for HeapItem {
    fn cmp(&self, other: &Self) -> Ordering {
        self.importance
            .cmp(&other.importance)
            .then(other.node.cmp(&self.node))
    }
}
impl PartialOrd for HeapItem {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl SpatialIndex {
    pub fn build(positions: Vec<[f32; 2]>, importance: Vec<u32>) -> Self {
        let n = positions.len();
        let (mut minx, mut miny, mut maxx, mut maxy) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for p in &positions {
            minx = minx.min(p[0]);
            miny = miny.min(p[1]);
            maxx = maxx.max(p[0]);
            maxy = maxy.max(p[1]);
        }
        if n == 0 {
            minx = 0.0;
            miny = 0.0;
            maxx = 0.0;
            maxy = 0.0;
        }
        // pad so points on the max edge land inside the grid
        let pad_x = ((maxx - minx) * 1e-4).max(1e-3);
        let pad_y = ((maxy - miny) * 1e-4).max(1e-3);
        maxx += pad_x;
        maxy += pad_y;

        // ~12 nodes per cell on average, grid proportioned to the layout aspect
        let target_per_cell = 12.0f32;
        let ncells = (n as f32 / target_per_cell).ceil().max(1.0);
        let w = (maxx - minx).max(1e-3);
        let h = (maxy - miny).max(1e-3);
        let aspect = w / h;
        let cols = ((ncells * aspect).sqrt().round() as usize).max(1);
        let rows = ((ncells / cols as f32).ceil() as usize).max(1);
        let cell_w = w / cols as f32;
        let cell_h = h / rows as f32;

        let mut cells: Vec<Vec<NodeId>> = vec![Vec::new(); cols * rows];
        for (i, p) in positions.iter().enumerate() {
            let cx = (((p[0] - minx) / cell_w) as usize).min(cols - 1);
            let cy = (((p[1] - miny) / cell_h) as usize).min(rows - 1);
            cells[cy * cols + cx].push(i as NodeId);
        }
        for cell in &mut cells {
            cell.sort_unstable_by(|&a, &b| {
                importance[b as usize]
                    .cmp(&importance[a as usize])
                    .then(a.cmp(&b))
            });
        }

        SpatialIndex {
            positions,
            importance,
            minx,
            miny,
            maxx,
            maxy,
            cols,
            rows,
            cell_w,
            cell_h,
            cells,
        }
    }

    pub fn position(&self, id: NodeId) -> Option<[f32; 2]> {
        self.positions.get(id as usize).copied()
    }

    pub fn bounds(&self) -> (f32, f32, f32, f32) {
        (self.minx, self.miny, self.maxx, self.maxy)
    }

    fn col_of(&self, x: f32) -> usize {
        (((x - self.minx) / self.cell_w) as isize)
            .clamp(0, self.cols as isize - 1) as usize
    }
    fn row_of(&self, y: f32) -> usize {
        (((y - self.miny) / self.cell_h) as isize)
            .clamp(0, self.rows as isize - 1) as usize
    }

    /// Return up to `budget` node ids inside the bounds, highest-importance
    /// first, merged across the overlapping grid cells.
    pub fn query(&self, minx: f32, miny: f32, maxx: f32, maxy: f32, budget: usize) -> Vec<NodeId> {
        if self.positions.is_empty() || budget == 0 {
            return Vec::new();
        }
        let c0 = self.col_of(minx);
        let c1 = self.col_of(maxx);
        let r0 = self.row_of(miny);
        let r1 = self.row_of(maxy);

        let mut heap: BinaryHeap<HeapItem> = BinaryHeap::new();
        for r in r0..=r1 {
            for c in c0..=c1 {
                let cell = &self.cells[r * self.cols + c];
                if let Some(&node) = cell.first() {
                    heap.push(HeapItem {
                        importance: self.importance[node as usize],
                        cell: r * self.cols + c,
                        cursor: 0,
                        node,
                    });
                }
            }
        }

        let mut out = Vec::with_capacity(budget.min(1024));
        while let Some(item) = heap.pop() {
            let p = self.positions[item.node as usize];
            if p[0] >= minx && p[0] <= maxx && p[1] >= miny && p[1] <= maxy {
                out.push(item.node);
                if out.len() >= budget {
                    break;
                }
            }
            // advance this cell's cursor
            let cell = &self.cells[item.cell];
            let next = item.cursor + 1;
            if let Some(&node) = cell.get(next) {
                heap.push(HeapItem {
                    importance: self.importance[node as usize],
                    cell: item.cell,
                    cursor: next,
                    node,
                });
            }
        }
        out
    }
}

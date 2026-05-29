//! Spatial index over the overview layout so the frontend only ever receives
//! the nodes/edges inside the current viewport (with a per-request budget).

use crate::store::NodeId;
use rstar::primitives::GeomWithData;
use rstar::{RTree, AABB};

type IndexedPoint = GeomWithData<[f32; 2], NodeId>;

pub struct SpatialIndex {
    tree: RTree<IndexedPoint>,
    positions: Vec<[f32; 2]>,
}

impl SpatialIndex {
    pub fn build(positions: Vec<[f32; 2]>) -> Self {
        let pts: Vec<IndexedPoint> = positions
            .iter()
            .enumerate()
            .map(|(i, p)| GeomWithData::new(*p, i as NodeId))
            .collect();
        let tree = RTree::bulk_load(pts);
        SpatialIndex { tree, positions }
    }

    pub fn position(&self, id: NodeId) -> Option<[f32; 2]> {
        self.positions.get(id as usize).copied()
    }

    pub fn positions(&self) -> &[[f32; 2]] {
        &self.positions
    }

    /// Node ids whose layout point falls inside the given bounds. If more than
    /// `max_nodes` match, the densest region is still bounded by returning the
    /// first `max_nodes` encountered (callers pass higher budgets when zoomed
    /// in, lower when zoomed out for level-of-detail).
    pub fn query(&self, minx: f32, miny: f32, maxx: f32, maxy: f32, max_nodes: usize) -> Vec<NodeId> {
        let env = AABB::from_corners([minx, miny], [maxx, maxy]);
        let mut out = Vec::new();
        for p in self.tree.locate_in_envelope(&env) {
            out.push(p.data);
            if out.len() >= max_nodes {
                break;
            }
        }
        out
    }

    pub fn bounds(&self) -> (f32, f32, f32, f32) {
        let (mut minx, mut miny, mut maxx, mut maxy) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for p in &self.positions {
            minx = minx.min(p[0]);
            miny = miny.min(p[1]);
            maxx = maxx.max(p[0]);
            maxy = maxy.max(p[1]);
        }
        if self.positions.is_empty() {
            (0.0, 0.0, 0.0, 0.0)
        } else {
            (minx, miny, maxx, maxy)
        }
    }
}

//! mmdeep-core: parse, index, discover and lay out very large Mermaid
//! flowcharts. This crate is UI-agnostic — both the Tauri app and the headless
//! `mmdeep-core` CLI build on the [`Document`] API.

pub mod discovery;
pub mod layout;
pub mod parser;
pub mod spatial;
pub mod store;

use discovery::{Dir, SubGraphPayload, SubNode};
use serde::Serialize;
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;
use store::{Graph, NodeId};

/// Magic + version for the layout cache sidecar file.
const CACHE_MAGIC: u32 = 0x4D_4D_44_4C; // "MMDL"
const CACHE_VERSION: u32 = 1;

#[derive(Serialize, Clone)]
pub struct Stats {
    pub path: String,
    pub node_count: usize,
    pub edge_count: usize,
    pub direction: String,
    pub root_count: usize,
    pub parse_ms: f64,
    pub approx_bytes: usize,
    pub has_overview: bool,
}

/// An opened document: the parsed graph plus an optional overview layout.
pub struct Document {
    pub path: PathBuf,
    pub graph: Graph,
    pub parse_ms: f64,
    content_hash: u64,
    pub overview: Option<spatial::SpatialIndex>,
}

impl Document {
    /// Parse a `.mmd` file from disk (streaming, line by line).
    pub fn open<P: AsRef<Path>>(path: P) -> std::io::Result<Document> {
        let path = path.as_ref().to_path_buf();
        let t0 = Instant::now();
        let file = File::open(&path)?;
        let content_hash = hash_file(&path)?;
        let builder = parser::Parser::new().parse_reader(BufReader::new(file))?;
        let graph = builder.build();
        let parse_ms = t0.elapsed().as_secs_f64() * 1000.0;
        Ok(Document {
            path,
            graph,
            parse_ms,
            content_hash,
            overview: None,
        })
    }

    /// Parse from an in-memory string (used in tests and small inputs).
    pub fn from_str(text: &str) -> Document {
        let t0 = Instant::now();
        let graph = parser::Parser::new().parse_str(text).build();
        let parse_ms = t0.elapsed().as_secs_f64() * 1000.0;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut hasher);
        Document {
            path: PathBuf::from("<memory>"),
            graph,
            parse_ms,
            content_hash: hasher.finish(),
            overview: None,
        }
    }

    pub fn stats(&self) -> Stats {
        Stats {
            path: self.path.display().to_string(),
            node_count: self.graph.node_count(),
            edge_count: self.graph.edge_count(),
            direction: self.graph.direction.clone(),
            root_count: self.graph.names.iter().enumerate().filter(|(i, _)| self.graph.in_degree(*i as NodeId) == 0).count(),
            parse_ms: self.parse_ms,
            approx_bytes: self.graph.approx_bytes(),
            has_overview: self.overview.is_some(),
        }
    }

    pub fn roots(&self, limit: usize) -> SubGraphPayload {
        let seeds = discovery::roots(&self.graph, limit);
        let mut payload = discovery::expand(&self.graph, &seeds, 1, limit.max(seeds.len()), Dir::Out);
        layout::layered(&mut payload);
        payload
    }

    pub fn expand(&self, seeds: &[NodeId], depth: u32, limit: usize, dir: &str) -> SubGraphPayload {
        let mut payload = discovery::expand(&self.graph, seeds, depth, limit, Dir::from_str(dir));
        layout::layered(&mut payload);
        payload
    }

    pub fn neighbors_page(
        &self,
        node: NodeId,
        dir: &str,
        offset: usize,
        count: usize,
    ) -> SubGraphPayload {
        let mut payload =
            discovery::neighbors_page(&self.graph, node, Dir::from_str(dir), offset, count);
        layout::layered(&mut payload);
        payload
    }

    pub fn search(&self, query: &str, limit: usize) -> Vec<SubNode> {
        discovery::search(&self.graph, query, limit)
    }

    /// Ensure the overview layout exists: load from cache if valid, else compute
    /// and cache. `iterations` controls force-layout quality when computing.
    pub fn ensure_overview(&mut self, iterations: usize) -> std::io::Result<()> {
        if self.overview.is_some() {
            return Ok(());
        }
        if let Some(pos) = self.load_cache() {
            self.overview = Some(spatial::SpatialIndex::build(pos));
            return Ok(());
        }
        let pos = layout::global_force(&self.graph, iterations);
        let _ = self.store_cache(&pos); // best-effort cache write
        self.overview = Some(spatial::SpatialIndex::build(pos));
        Ok(())
    }

    /// Query the overview viewport. Returns nodes inside the bounds (capped at
    /// `max_nodes`) plus edges among them. Requires `ensure_overview` first.
    pub fn viewport(
        &self,
        minx: f32,
        miny: f32,
        maxx: f32,
        maxy: f32,
        max_nodes: usize,
    ) -> SubGraphPayload {
        let Some(idx) = &self.overview else {
            return SubGraphPayload {
                nodes: vec![],
                edges: vec![],
                truncated: false,
            };
        };
        let ids = idx.query(minx, miny, maxx, maxy, max_nodes);
        let keep: std::collections::HashSet<NodeId> = ids.iter().copied().collect();
        let mut payload = build_viewport_payload(&self.graph, &ids, &keep);
        // attach precomputed coordinates
        for node in &mut payload.nodes {
            if let Some(p) = idx.position(node.id) {
                node.x = p[0];
                node.y = p[1];
            }
        }
        payload
    }

    pub fn overview_bounds(&self) -> Option<(f32, f32, f32, f32)> {
        self.overview.as_ref().map(|i| i.bounds())
    }

    fn cache_path(&self) -> PathBuf {
        let mut p = self.path.clone().into_os_string();
        p.push(".mmdlayout");
        PathBuf::from(p)
    }

    fn load_cache(&self) -> Option<Vec<[f32; 2]>> {
        let mut f = File::open(self.cache_path()).ok()?;
        let mut header = [0u8; 24];
        f.read_exact(&mut header).ok()?;
        let magic = u32::from_le_bytes(header[0..4].try_into().unwrap());
        let version = u32::from_le_bytes(header[4..8].try_into().unwrap());
        let hash = u64::from_le_bytes(header[8..16].try_into().unwrap());
        let count = u64::from_le_bytes(header[16..24].try_into().unwrap()) as usize;
        if magic != CACHE_MAGIC
            || version != CACHE_VERSION
            || hash != self.content_hash
            || count != self.graph.node_count()
        {
            return None;
        }
        let mut bytes = vec![0u8; count * 8];
        f.read_exact(&mut bytes).ok()?;
        let mut pos = Vec::with_capacity(count);
        for chunk in bytes.chunks_exact(8) {
            let x = f32::from_le_bytes(chunk[0..4].try_into().unwrap());
            let y = f32::from_le_bytes(chunk[4..8].try_into().unwrap());
            pos.push([x, y]);
        }
        Some(pos)
    }

    fn store_cache(&self, pos: &[[f32; 2]]) -> std::io::Result<()> {
        let mut f = File::create(self.cache_path())?;
        f.write_all(&CACHE_MAGIC.to_le_bytes())?;
        f.write_all(&CACHE_VERSION.to_le_bytes())?;
        f.write_all(&self.content_hash.to_le_bytes())?;
        f.write_all(&(pos.len() as u64).to_le_bytes())?;
        let mut buf = Vec::with_capacity(pos.len() * 8);
        for p in pos {
            buf.extend_from_slice(&p[0].to_le_bytes());
            buf.extend_from_slice(&p[1].to_le_bytes());
        }
        f.write_all(&buf)?;
        Ok(())
    }
}

/// Build a payload for an explicit node set (overview viewport): include every
/// edge with both endpoints in the kept set.
fn build_viewport_payload(
    g: &Graph,
    order: &[NodeId],
    keep: &std::collections::HashSet<NodeId>,
) -> SubGraphPayload {
    let mut nodes = Vec::with_capacity(order.len());
    for &id in order {
        nodes.push(SubNode {
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
    }
    let mut edges = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for &id in order {
        for &e in g.out_edges(id) {
            let (a, b) = g.edge_endpoints(e);
            if keep.contains(&b) && seen.insert(e) {
                edges.push(discovery::SubEdge {
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
        truncated: false,
    }
}

/// Hash file contents for the layout cache key.
fn hash_file(path: &Path) -> std::io::Result<u64> {
    let mut f = File::open(path)?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        buf[..n].hash(&mut hasher);
    }
    Ok(hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_roundtrip() {
        let doc = Document::from_str("flowchart TD\nA --> B\nB --> C\n");
        let s = doc.stats();
        assert_eq!(s.node_count, 3);
        assert_eq!(s.edge_count, 2);
        assert_eq!(s.root_count, 1);
    }

    #[test]
    fn overview_and_viewport() {
        let mut doc = Document::from_str("flowchart TD\nA --> B\nB --> C\nC --> D\nA --> D\n");
        doc.ensure_overview(20).unwrap();
        let (minx, miny, maxx, maxy) = doc.overview_bounds().unwrap();
        let p = doc.viewport(minx - 1.0, miny - 1.0, maxx + 1.0, maxy + 1.0, 100);
        assert_eq!(p.nodes.len(), 4);
    }
}

//! Tauri backend: thin command layer over `mmdeep_core::Document`.
//!
//! The open document lives in shared state behind a mutex. Quick queries
//! (stats, roots, expand, viewport, search) run inline — they are sub-10ms even
//! at 1M+ edges. The one heavy operation, computing the overview force layout,
//! runs on a blocking worker thread so the UI never freezes.

use mmdeep_core::discovery::{SubGraphPayload, SubNode};
use mmdeep_core::{Document, Stats};
use serde::Serialize;
use std::sync::{Arc, Mutex};
use tauri::State;

type SharedDoc = Arc<Mutex<Option<Document>>>;

#[derive(Serialize)]
struct OverviewInfo {
    minx: f32,
    miny: f32,
    maxx: f32,
    maxy: f32,
    node_count: usize,
    edge_count: usize,
    iterations: usize,
    compute_ms: f64,
}

fn with_doc<T, F>(state: &SharedDoc, f: F) -> Result<T, String>
where
    F: FnOnce(&Document) -> T,
{
    let guard = state.lock().map_err(|_| "state poisoned")?;
    let doc = guard.as_ref().ok_or("no document open")?;
    Ok(f(doc))
}

#[tauri::command]
fn open_file(path: String, state: State<SharedDoc>) -> Result<Stats, String> {
    let doc = Document::open(&path).map_err(|e| format!("failed to open {path}: {e}"))?;
    let stats = doc.stats();
    *state.lock().map_err(|_| "state poisoned")? = Some(doc);
    Ok(stats)
}

#[tauri::command]
fn stats(state: State<SharedDoc>) -> Result<Stats, String> {
    with_doc(&state, |d| d.stats())
}

#[tauri::command]
fn get_roots(limit: usize, state: State<SharedDoc>) -> Result<SubGraphPayload, String> {
    with_doc(&state, |d| d.roots(limit))
}

#[tauri::command]
fn expand(
    seeds: Vec<u32>,
    depth: u32,
    limit: usize,
    dir: String,
    state: State<SharedDoc>,
) -> Result<SubGraphPayload, String> {
    with_doc(&state, |d| d.expand(&seeds, depth, limit, &dir))
}

#[tauri::command]
fn neighbors_page(
    node: u32,
    dir: String,
    offset: usize,
    count: usize,
    state: State<SharedDoc>,
) -> Result<SubGraphPayload, String> {
    with_doc(&state, |d| d.neighbors_page(node, &dir, offset, count))
}

#[tauri::command]
fn search(query: String, limit: usize, state: State<SharedDoc>) -> Result<Vec<SubNode>, String> {
    with_doc(&state, |d| d.search(&query, limit))
}

/// Compute (or load from cache) the overview layout on a blocking worker thread.
#[tauri::command]
async fn ensure_overview(
    iterations: Option<usize>,
    state: State<'_, SharedDoc>,
) -> Result<OverviewInfo, String> {
    let shared = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut guard = shared.lock().map_err(|_| "state poisoned")?;
        let doc = guard.as_mut().ok_or("no document open")?;
        let n = doc.graph.node_count();
        // scale iterations down for very large graphs (overview is cached)
        let iters = iterations.unwrap_or(if n > 500_000 {
            40
        } else if n > 100_000 {
            60
        } else {
            120
        });
        let t0 = std::time::Instant::now();
        doc.ensure_overview(iters).map_err(|e| e.to_string())?;
        let compute_ms = t0.elapsed().as_secs_f64() * 1000.0;
        let (minx, miny, maxx, maxy) = doc.overview_bounds().unwrap_or((0.0, 0.0, 0.0, 0.0));
        Ok(OverviewInfo {
            minx,
            miny,
            maxx,
            maxy,
            node_count: n,
            edge_count: doc.graph.edge_count(),
            iterations: iters,
            compute_ms,
        })
    })
    .await
    .map_err(|e| format!("overview task failed: {e}"))?
}

#[tauri::command]
fn viewport(
    minx: f32,
    miny: f32,
    maxx: f32,
    maxy: f32,
    max_nodes: usize,
    state: State<SharedDoc>,
) -> Result<SubGraphPayload, String> {
    with_doc(&state, |d| d.viewport(minx, miny, maxx, maxy, max_nodes))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(SharedDoc::default())
        .invoke_handler(tauri::generate_handler![
            open_file,
            stats,
            get_roots,
            expand,
            neighbors_page,
            search,
            ensure_overview,
            viewport
        ])
        .run(tauri::generate_context!())
        .expect("error while running mmdeep");
}

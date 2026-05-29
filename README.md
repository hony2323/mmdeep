# mmdeep

A cross-platform desktop viewer for **very large Mermaid (`.mmd`) flowcharts** —
built to open files with **up to ~1,000,000 edges** and stay interactive, where
existing Mermaid tooling falls over around ~1,000 edges.

## Why it scales

Standard Mermaid tooling renders **mermaid.js → dagre layout → SVG/DOM**. Both
the layout (≈O(n²)) and the per-element DOM cost collapse well before 100k
edges. mmdeep takes a different approach:

- **Native parse + index** (Rust): a streaming Mermaid-flowchart parser builds a
  compact CSR (compressed-sparse-row) graph. 1.4M edges parses in ~2.5s and
  holds in ~70 MB RAM.
- **Never render the whole graph.** The entire graph is laid out once into a
  fixed "world" (a whole-graph Barnes-Hut force layout, computed once and cached
  to disk), then explored **like a map — pan and zoom**:
  - The renderer pins Sigma.js's coordinate box to the full-graph bounds, so the
    world is stable while you drag/zoom.
  - Only the nodes/edges inside the current viewport are sent to the frontend,
    **ranked by importance (degree)** up to a budget: zoomed out shows the major
    hubs (like cities on a map), zooming in reveals local detail (like streets).
    A grid level-of-detail index answers each viewport query in a few ms.
  - Hover a node to highlight it and its neighbours; search or click to fly to a
    node anywhere in the world.

## Architecture

```
.mmd ─▶ Rust core (crates/mmdeep-core)                  ─▶ Tauri commands ─▶ React + Sigma.js
        parser → CSR store → discovery (BFS/paging)         (src-tauri)        (src/)  WebGL render
                            → layered + force layout                            culling + LOD
                            → R-tree viewport index
```

- `crates/mmdeep-core` — UI-agnostic engine. Also ships a headless CLI
  (`mmdeep-core`) used for testing and benchmarking.
- `src-tauri` — Tauri v2 app; thin command layer over the core.
- `src` — Vite + React + TypeScript frontend; Sigma.js/graphology map renderer
  (fixed-world `setCustomBBox`, viewport diff-loading, importance LOD).
- `scripts` — Python generators and the benchmark harness.

## Prerequisites

- **Rust** (stable) with the platform's C toolchain
  (Windows: *Visual Studio Build Tools* with the C++ workload; macOS: Xcode CLT;
  Linux: `build-essential` + webkit2gtk — see the Tauri docs).
- **Node.js 18+** and npm.
- **Python 3.9+** (only for the example/benchmark scripts).
- WebView2 (preinstalled on Windows 11; Tauri installs it otherwise).

## Develop

```bash
npm install
npm run tauri dev          # launches the desktop app with hot reload
```

## Build installers

```bash
npm run tauri build        # → .msi/.exe (Win), .dmg (macOS), .AppImage/.deb (Linux)
```

## Generate test data

```bash
python scripts/generate.py --preset small  --out examples/small.mmd
python scripts/generate.py --preset large  --out examples/large.mmd      # ~1M edges
python scripts/generate.py --topology tree --nodes 50000 --out examples/tree.mmd
```

Topologies: `tree`, `dag`, `scale-free`, `random`, `grid`, `clustered`.
Presets: `tiny` (1k), `small` (10k), `medium` (100k), `large` (1M), `huge` (5M).

## Benchmark

```bash
cargo build --release -p mmdeep-core
python scripts/bench.py            # full matrix → benchmarks/results.json
python scripts/bench.py --quick    # skip the 1M preset
```

The harness drives the headless `mmdeep-core` binary (no GUI in the timing path)
and reports parse time, memory, first-render (roots), expand latency, overview
layout time, and viewport-query latency.

### Indicative numbers (1.4M edges, scale-free)

| metric                 | value      |
| ---------------------- | ---------- |
| parse + index            | ~2.5 s     |
| memory                   | ~70 MB     |
| viewport query (pan/zoom)| ~5–8 ms    |
| search                   | a few ms   |
| map layout (once)        | ~45 s, then cached (~2.7 s reopen) |

## License

MIT

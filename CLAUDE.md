# mmdeep — notes for Claude

Desktop viewer for very large Mermaid `.mmd` flowcharts (target: ~1M edges).
The whole design hinges on **never rendering the full graph**: parse/index in
Rust, then only ever draw the visible subgraph.

## Layout

- `crates/mmdeep-core/` — UI-agnostic engine (the important code lives here):
  - `parser.rs` — streaming Mermaid flowchart parser (line-based, bracket-masked
    link detection via regex). Supports edges/labels/shapes/chains/groups.
  - `store.rs` — interned node ids + CSR forward/reverse adjacency (`Graph`).
  - `discovery.rs` — `roots`, budgeted BFS `expand`, and `neighbors_page`
    (the "+N more" paging). Returns serde `SubGraphPayload`.
  - `layout.rs` — `layered` (Sugiyama-ish, for small discovery subgraphs) and
    `global_force` (parallel Barnes-Hut, for the overview; cached to disk).
  - `spatial.rs` — `rstar` R-tree for overview viewport queries.
  - `lib.rs` — `Document` ties it together + layout cache (`*.mmdlayout`).
  - `src/bin/cli.rs` — headless `mmdeep-core` CLI (used by `scripts/bench.py`).
- `src-tauri/` — Tauri v2 app; `src/lib.rs` is a thin command layer. Heavy
  `ensure_overview` runs on a blocking worker thread.
- `src/` — React + Sigma.js frontend. `graphController.ts` owns the
  graphology graph + Sigma instance and implements both modes.
- `scripts/` — `generate.py` (topologies/presets), `bench.py`, `make_icon.py`.

## Build / run

- Toolchain: **MSVC** toolchain is required for the Tauri app on Windows
  (installed: VS 2022 Build Tools, VC 14.44). cargo finds `link.exe` itself.
  Add cargo to PATH in a shell: `$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"`.
- App: `npm install` then `npm run tauri dev` / `npm run tauri build`.
- Core only (fast, no webview): `cargo build -p mmdeep-core`,
  `cargo test -p mmdeep-core`.
- Benchmarks are CPU-sensitive — don't trust numbers taken while a compile runs.

## Conventions

- Backend serializes structs with serde defaults → **snake_case** JSON; the TS
  types in `src/api.ts` mirror that. Enums use kebab-case (`round-rect`).
- Discovery layout flips Y (`-y`) for a top-down look; overview does not.
- Parser is single-statement-per-line; inline `A -- text --> B` edge text is not
  parsed (use the `A -->|text| B` form). Add tests in the relevant `mod tests`.

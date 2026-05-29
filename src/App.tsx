import { useCallback, useEffect, useRef, useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { api, type Stats, type SubNode } from "./api";
import { GraphController, type HudInfo } from "./graphController";

export default function App() {
  const containerRef = useRef<HTMLDivElement>(null);
  const ctrlRef = useRef<GraphController | null>(null);

  const [stats, setStats] = useState<Stats | null>(null);
  const [selected, setSelected] = useState<SubNode | null>(null);
  const [hud, setHud] = useState<HudInfo | null>(null);
  const [status, setStatus] = useState("Open a .mmd file to begin.");
  const [busy, setBusy] = useState(false);

  const [query, setQuery] = useState("");
  const [results, setResults] = useState<SubNode[]>([]);

  useEffect(() => {
    if (!containerRef.current) return;
    const ctrl = new GraphController(containerRef.current, {
      onSelect: setSelected,
      onHud: setHud,
      onStatus: setStatus,
    });
    ctrlRef.current = ctrl;
    return () => ctrl.destroy();
  }, []);

  const openPath = useCallback(async (path: string) => {
    setBusy(true);
    setStatus(`Parsing ${path}…`);
    try {
      const s = await api.openFile(path);
      setStats(s);
      setSelected(null);
      setResults([]);
      await ctrlRef.current?.openMap();
    } catch (e) {
      setStatus(`Error: ${e}`);
    } finally {
      setBusy(false);
    }
  }, []);

  const handleOpen = useCallback(async () => {
    const path = await openDialog({
      multiple: false,
      filters: [{ name: "Mermaid", extensions: ["mmd", "mermaid", "txt"] }],
    });
    if (typeof path === "string") await openPath(path);
  }, [openPath]);

  // auto-open a file passed on the command line (file association / CLI arg)
  useEffect(() => {
    api.startupFile().then((p) => {
      if (p) void openPath(p);
    });
  }, [openPath]);

  const runSearch = useCallback(async () => {
    if (!query.trim() || !stats) return;
    const res = await api.search(query.trim(), 50);
    setResults(res);
    setStatus(`${res.length} match(es) for "${query.trim()}" — click one to fly there.`);
  }, [query, stats]);

  const zoomPct = hud ? Math.round((1 / Math.max(hud.ratio, 1e-3)) * 100) : 100;

  return (
    <div className="app">
      <header className="toolbar">
        <button className="primary" onClick={handleOpen} disabled={busy}>
          Open .mmd
        </button>

        <div className="search">
          <input
            placeholder="Search nodes…"
            value={query}
            disabled={!stats}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && runSearch()}
          />
          <button onClick={runSearch} disabled={!stats}>
            Search
          </button>
        </div>

        <div className="cam">
          <button title="Zoom out" onClick={() => ctrlRef.current?.zoomOut()}>
            −
          </button>
          <button title="Fit whole graph" onClick={() => ctrlRef.current?.resetCamera()}>
            ⤢
          </button>
          <button title="Zoom in" onClick={() => ctrlRef.current?.zoomIn()}>
            +
          </button>
        </div>
      </header>

      <div className="body">
        <aside className="sidebar">
          {stats && (
            <section>
              <h3>File</h3>
              <div className="kv">
                <span>nodes</span>
                <b>{stats.node_count.toLocaleString()}</b>
              </div>
              <div className="kv">
                <span>edges</span>
                <b>{stats.edge_count.toLocaleString()}</b>
              </div>
              <div className="kv">
                <span>roots</span>
                <b>{stats.root_count.toLocaleString()}</b>
              </div>
              <div className="kv">
                <span>parse</span>
                <b>{stats.parse_ms.toFixed(0)} ms</b>
              </div>
              <div className="kv">
                <span>memory</span>
                <b>{(stats.approx_bytes / 1e6).toFixed(1)} MB</b>
              </div>
            </section>
          )}

          {selected && (
            <section>
              <h3>Selected</h3>
              <div className="kv">
                <span>id</span>
                <b>{selected.name}</b>
              </div>
              <div className="kv">
                <span>label</span>
                <b title={selected.label}>{trunc(selected.label, 22)}</b>
              </div>
              <div className="kv">
                <span>shape</span>
                <b>{selected.shape}</b>
              </div>
              <div className="kv">
                <span>out / in</span>
                <b>
                  {selected.out_degree} / {selected.in_degree}
                </b>
              </div>
              <button className="full" onClick={() => ctrlRef.current?.focusNode(selected.id)}>
                Fly to node
              </button>
            </section>
          )}

          {results.length > 0 && (
            <section>
              <h3>Search results</h3>
              <ul className="results">
                {results.map((r) => (
                  <li key={r.id}>
                    <button onClick={() => ctrlRef.current?.focusNode(r.id)}>
                      {trunc(r.label, 26)}
                      <em>{r.out_degree + r.in_degree} deg</em>
                    </button>
                  </li>
                ))}
              </ul>
            </section>
          )}
        </aside>

        <main className="canvas-wrap">
          <div ref={containerRef} className="canvas" />
          {hud && stats && (
            <div className="hud">
              <span className="pill map">map</span>
              <span>
                {hud.visibleNodes.toLocaleString()} / {hud.totalNodes.toLocaleString()} nodes
              </span>
              <span>{hud.visibleEdges.toLocaleString()} edges</span>
              <span>{zoomPct}% zoom</span>
              <span>{hud.fps} fps</span>
              <span>{hud.lastQueryMs.toFixed(1)} ms</span>
            </div>
          )}
          {busy && <div className="overlay">Computing map layout…</div>}
          {!stats && (
            <div className="empty">
              <h1>mmdeep</h1>
              <p>
                Open a Mermaid <code>.mmd</code> flowchart — even one with a million edges — and
                explore the whole graph as a map: scroll to zoom, drag to pan.
              </p>
            </div>
          )}
        </main>
      </div>

      <footer className="statusbar">{status}</footer>
    </div>
  );
}

function trunc(s: string, n: number) {
  return s.length > n ? s.slice(0, n - 1) + "…" : s;
}

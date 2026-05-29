import { useCallback, useEffect, useRef, useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { api, type Dir, type Stats, type SubNode } from "./api";
import { GraphController, type HudInfo } from "./graphController";

export default function App() {
  const containerRef = useRef<HTMLDivElement>(null);
  const ctrlRef = useRef<GraphController | null>(null);

  const [stats, setStats] = useState<Stats | null>(null);
  const [selected, setSelected] = useState<SubNode | null>(null);
  const [hud, setHud] = useState<HudInfo | null>(null);
  const [status, setStatus] = useState("Open a .mmd file to begin.");
  const [mode, setMode] = useState<"discovery" | "overview">("discovery");
  const [dir, setDir] = useState<Dir>("both");
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

  const handleOpen = useCallback(async () => {
    const path = await openDialog({
      multiple: false,
      filters: [{ name: "Mermaid", extensions: ["mmd", "mermaid", "txt"] }],
    });
    if (typeof path !== "string") return;
    setBusy(true);
    setStatus(`Parsing ${path}…`);
    try {
      const s = await api.openFile(path);
      setStats(s);
      setSelected(null);
      setResults([]);
      setMode("discovery");
      await ctrlRef.current?.loadRoots();
    } catch (e) {
      setStatus(`Error: ${e}`);
    } finally {
      setBusy(false);
    }
  }, []);

  const switchMode = useCallback(
    async (next: "discovery" | "overview") => {
      const ctrl = ctrlRef.current;
      if (!ctrl || !stats) return;
      setBusy(true);
      try {
        if (next === "overview") {
          await ctrl.enterOverview();
        } else {
          await ctrl.loadRoots();
        }
        setMode(next);
      } catch (e) {
        setStatus(`Error: ${e}`);
      } finally {
        setBusy(false);
      }
    },
    [stats],
  );

  const runSearch = useCallback(async () => {
    if (!query.trim() || !stats) return;
    const res = await api.search(query.trim(), 50);
    setResults(res);
    setStatus(`${res.length} match(es) for "${query.trim()}"`);
  }, [query, stats]);

  const onDirChange = (d: Dir) => {
    setDir(d);
    ctrlRef.current?.setDir(d);
  };

  return (
    <div className="app">
      <header className="toolbar">
        <button className="primary" onClick={handleOpen} disabled={busy}>
          Open .mmd
        </button>

        <div className="seg">
          <button
            className={mode === "discovery" ? "active" : ""}
            onClick={() => switchMode("discovery")}
            disabled={busy || !stats}
          >
            Discovery
          </button>
          <button
            className={mode === "overview" ? "active" : ""}
            onClick={() => switchMode("overview")}
            disabled={busy || !stats}
          >
            Overview
          </button>
        </div>

        <label className="dir">
          Expand:
          <select value={dir} onChange={(e) => onDirChange(e.target.value as Dir)}>
            <option value="both">both</option>
            <option value="out">outgoing</option>
            <option value="in">incoming</option>
          </select>
        </label>

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
          <button onClick={() => ctrlRef.current?.zoomOut()}>−</button>
          <button onClick={() => ctrlRef.current?.resetCamera()}>⤢</button>
          <button onClick={() => ctrlRef.current?.zoomIn()}>+</button>
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
              {selected.truncated > 0 && (
                <div className="more">+{selected.truncated} more — click node to load</div>
              )}
              <button
                className="full"
                onClick={() => ctrlRef.current?.pageNeighbors(selected.id)}
                disabled={mode !== "discovery"}
              >
                Expand neighbours
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
                      <em>
                        {r.out_degree + r.in_degree} deg
                      </em>
                    </button>
                  </li>
                ))}
              </ul>
            </section>
          )}
        </aside>

        <main className="canvas-wrap">
          <div ref={containerRef} className="canvas" />
          {hud && (
            <div className="hud">
              <span className={`pill ${hud.mode}`}>{hud.mode}</span>
              <span>{hud.visibleNodes.toLocaleString()} nodes</span>
              <span>{hud.visibleEdges.toLocaleString()} edges</span>
              <span>{hud.fps} fps</span>
              <span>{hud.lastQueryMs.toFixed(1)} ms query</span>
            </div>
          )}
          {busy && <div className="overlay">Working…</div>}
          {!stats && (
            <div className="empty">
              <h1>mmdeep</h1>
              <p>Open a Mermaid <code>.mmd</code> flowchart — even one with a million edges.</p>
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

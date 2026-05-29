// Owns the graphology graph + Sigma renderer for the MAP view.
//
// The whole graph has one fixed global layout (computed/cached in the backend).
// We pin Sigma's coordinate box to the full-graph bounds (`setCustomBBox`) so
// the world is stable: panning/zooming behaves like a map and loaded subsets
// always render at the same place. Only the nodes/edges inside the current
// viewport are sent over IPC, ranked by importance (degree) so zoomed-out views
// show major hubs and zooming in reveals local detail — like map tiles.
import Graph from "graphology";
import Sigma from "sigma";
import { api, type SubGraphPayload, type SubNode, type Shape } from "./api";

const SHAPE_COLORS: Record<Shape, string> = {
  default: "#6ea8fe",
  rect: "#6ea8fe",
  "round-rect": "#63e6be",
  stadium: "#63e6be",
  subroutine: "#74c0fc",
  cylinder: "#ffd43b",
  circle: "#ff8787",
  diamond: "#faa2c1",
  hexagon: "#b197fc",
  parallelogram: "#ffa94d",
  trapezoid: "#ffa94d",
  asymmetric: "#ffe066",
};

const DIM_NODE = "#262d3c";
const DIM_EDGE = "#1b2230";
const HILITE_EDGE = "#6ea8fe";

export interface HudInfo {
  visibleNodes: number;
  visibleEdges: number;
  totalNodes: number;
  totalEdges: number;
  fps: number;
  lastQueryMs: number;
  ratio: number;
}

export interface Callbacks {
  onSelect: (node: SubNode | null) => void;
  onHud: (hud: HudInfo) => void;
  onStatus: (msg: string) => void;
}

export class GraphController {
  private graph: Graph;
  private sigma: Sigma;
  private cb: Callbacks;

  private world = { minx: 0, miny: 0, maxx: 0, maxy: 0 };
  private totalNodes = 0;
  private totalEdges = 0;
  private ready = false;

  // viewport refresh scheduling / reentrancy
  private refreshTimer: number | null = null;
  private refreshing = false;
  private refreshQueued = false;

  // hover highlight
  private hovered: string | null = null;
  private hoverSet = new Set<string>();

  // perf HUD
  private lastQueryMs = 0;
  private frames = 0;
  private fps = 0;
  private fpsTimer: number;

  constructor(container: HTMLElement, cb: Callbacks) {
    this.cb = cb;
    this.graph = new Graph({ multi: true, type: "directed" });
    this.sigma = new Sigma(this.graph, container, {
      renderLabels: true,
      labelRenderedSizeThreshold: 7,
      labelDensity: 0.7,
      labelGridCellSize: 140,
      labelColor: { color: "#e7ecf5" },
      labelFont: "Inter, system-ui, sans-serif",
      labelSize: 12,
      labelWeight: "500",
      defaultNodeColor: "#6ea8fe",
      defaultEdgeColor: "#2b3346",
      defaultEdgeType: "arrow",
      zIndex: true,
    });

    this.sigma.on("clickNode", ({ node }) => this.flyToNode(node));
    this.sigma.on("enterNode", ({ node }) => this.setHover(node));
    this.sigma.on("leaveNode", () => this.setHover(null));
    this.sigma.on("clickStage", () => this.setHover(null));

    this.sigma.setSetting("nodeReducer", (node, data) => this.nodeReducer(node, data));
    this.sigma.setSetting("edgeReducer", (edge, data) => this.edgeReducer(edge, data));

    this.sigma.getCamera().on("updated", () => this.scheduleRefresh());

    this.sigma.on("afterRender", () => {
      this.frames++;
    });
    this.fpsTimer = window.setInterval(() => {
      this.fps = this.frames;
      this.frames = 0;
      this.emitHud();
    }, 1000);
  }

  destroy() {
    window.clearInterval(this.fpsTimer);
    if (this.refreshTimer) window.clearTimeout(this.refreshTimer);
    this.sigma.kill();
  }

  // -- reducers (hover highlight + label LOD) -------------------------------

  private nodeReducer(node: string, data: any) {
    const res: any = { ...data };
    if (this.hovered) {
      if (!this.hoverSet.has(node)) {
        res.color = DIM_NODE;
        res.label = "";
        res.zIndex = 0;
      } else {
        res.zIndex = node === this.hovered ? 3 : 2;
        res.forceLabel = true;
        if (node === this.hovered) res.highlighted = true;
      }
    }
    return res;
  }

  private edgeReducer(edge: string, data: any) {
    const res: any = { ...data };
    if (this.hovered) {
      const [s, t] = this.graph.extremities(edge);
      if (s === this.hovered || t === this.hovered) {
        res.color = HILITE_EDGE;
        res.zIndex = 2;
        res.size = (res.size ?? 1) + 0.6;
      } else {
        res.color = DIM_EDGE;
        res.zIndex = 0;
      }
    }
    return res;
  }

  private setHover(node: string | null) {
    if (node && this.graph.hasNode(node)) {
      this.hovered = node;
      this.hoverSet = new Set<string>(this.graph.neighbors(node));
      this.hoverSet.add(node);
      this.emitSelect(node);
    } else {
      this.hovered = null;
      this.hoverSet.clear();
    }
    this.sigma.refresh({ skipIndexation: true });
  }

  // -- node styling ----------------------------------------------------------

  private nodeSize(deg: number) {
    return Math.max(3, Math.min(22, 3 + Math.sqrt(deg) * 1.4));
  }

  private styleAttrs(n: SubNode) {
    const deg = n.out_degree + n.in_degree;
    return {
      x: n.x,
      y: n.y,
      size: this.nodeSize(deg),
      color: SHAPE_COLORS[n.shape] ?? SHAPE_COLORS.default,
      label: n.label,
      name: n.name,
      shape: n.shape,
      out_degree: n.out_degree,
      in_degree: n.in_degree,
      truncated: n.truncated,
      zIndex: deg,
    };
  }

  private emitSelect(nodeKey: string) {
    const a = this.graph.getNodeAttributes(nodeKey) as any;
    this.cb.onSelect({
      id: Number(nodeKey),
      name: a.name,
      label: a.label,
      shape: a.shape,
      out_degree: a.out_degree,
      in_degree: a.in_degree,
      truncated: a.truncated ?? 0,
      x: a.x,
      y: a.y,
    });
  }

  private emitHud() {
    this.cb.onHud({
      visibleNodes: this.graph.order,
      visibleEdges: this.graph.size,
      totalNodes: this.totalNodes,
      totalEdges: this.totalEdges,
      fps: this.fps,
      lastQueryMs: this.lastQueryMs,
      ratio: this.sigma.getCamera().getState().ratio,
    });
  }

  // -- open the map ----------------------------------------------------------

  async openMap() {
    this.ready = false;
    this.graph.clear();
    this.setHover(null);
    this.cb.onStatus("Computing map layout… (one-time, cached afterwards)");
    const info = await api.ensureOverview();
    this.world = { minx: info.minx, miny: info.miny, maxx: info.maxx, maxy: info.maxy };
    this.totalNodes = info.node_count;
    this.totalEdges = info.edge_count;

    // pin the coordinate system to the whole-graph bounds -> stable world
    this.sigma.setCustomBBox({
      x: [this.world.minx, this.world.maxx],
      y: [this.world.miny, this.world.maxy],
    });
    this.sigma.getCamera().setState({ x: 0.5, y: 0.5, ratio: 1, angle: 0 });
    this.sigma.refresh();
    this.ready = true;

    this.cb.onStatus(
      `Map ready: ${info.node_count.toLocaleString()} nodes, ` +
        `${info.edge_count.toLocaleString()} edges (layout ${info.compute_ms.toFixed(0)} ms). ` +
        `Scroll to zoom, drag to pan.`,
    );
    await this.refreshViewport();
  }

  // -- viewport streaming ----------------------------------------------------

  private scheduleRefresh() {
    if (!this.ready) return;
    if (this.refreshTimer) window.clearTimeout(this.refreshTimer);
    this.refreshTimer = window.setTimeout(() => this.refreshViewport(), 120);
  }

  private async refreshViewport() {
    if (!this.ready) return;
    if (this.refreshing) {
      this.refreshQueued = true;
      return;
    }
    this.refreshing = true;
    try {
      const { width, height } = this.sigma.getDimensions();
      const tl = this.sigma.viewportToGraph({ x: 0, y: 0 });
      const br = this.sigma.viewportToGraph({ x: width, y: height });
      // query a margin around the visible area so panning has a buffer
      const mx = Math.abs(br.x - tl.x) * 0.25;
      const my = Math.abs(br.y - tl.y) * 0.25;
      const minx = Math.min(tl.x, br.x) - mx;
      const maxx = Math.max(tl.x, br.x) + mx;
      const miny = Math.min(tl.y, br.y) - my;
      const maxy = Math.max(tl.y, br.y) + my;

      const ratio = this.sigma.getCamera().getState().ratio;
      const budget = ratio > 0.55 ? 1500 : ratio > 0.2 ? 4500 : 9000;

      const t = performance.now();
      const payload = await api.viewport(minx, miny, maxx, maxy, budget);
      this.lastQueryMs = performance.now() - t;
      this.applyDiff(payload);
      this.emitHud();
    } finally {
      this.refreshing = false;
      if (this.refreshQueued) {
        this.refreshQueued = false;
        void this.refreshViewport();
      }
    }
  }

  /** Reconcile the loaded graph with the viewport payload (add/remove diff so
   * stable nodes don't flicker). */
  private applyDiff(payload: SubGraphPayload) {
    const wantNodes = new Set<string>();
    for (const n of payload.nodes) wantNodes.add(String(n.id));

    // drop nodes no longer in view (also drops their edges)
    for (const node of this.graph.nodes()) {
      if (!wantNodes.has(node)) this.graph.dropNode(node);
    }
    // add new nodes
    for (const n of payload.nodes) {
      const key = String(n.id);
      if (!this.graph.hasNode(key)) this.graph.addNode(key, this.styleAttrs(n));
    }
    // reconcile edges
    const wantEdges = new Set<string>();
    for (const e of payload.edges) {
      const s = String(e.source);
      const t = String(e.target);
      if (!this.graph.hasNode(s) || !this.graph.hasNode(t)) continue;
      const ek = `${e.source}->${e.target}`;
      wantEdges.add(ek);
      if (!this.graph.hasEdge(ek)) {
        this.graph.addEdgeWithKey(ek, s, t, {
          size: e.style === "thick" ? 2 : 0.8,
          color: e.style === "dotted" ? "#3a4356" : "#2b3346",
          type: "arrow",
          label: e.label ?? undefined,
        });
      }
    }
    for (const ek of this.graph.edges()) {
      if (!wantEdges.has(ek)) this.graph.dropEdge(ek);
    }
  }

  // -- navigation ------------------------------------------------------------

  private flyToNode(nodeKey: string) {
    if (!this.graph.hasNode(nodeKey)) return;
    this.emitSelect(nodeKey);
    const d = this.sigma.getNodeDisplayData(nodeKey);
    if (!d) return;
    const ratio = this.sigma.getCamera().getState().ratio;
    this.sigma.getCamera().animate(
      { x: d.x, y: d.y, ratio: Math.min(ratio, 0.18) },
      { duration: 500 },
    );
  }

  /** Fly to a node found via search, even if it isn't currently loaded. */
  async focusNode(id: number) {
    const pos = await api.locate(id);
    if (!pos) {
      this.cb.onStatus("That node has no map position yet — open the map first.");
      return;
    }
    const key = String(id);
    if (!this.graph.hasNode(key)) {
      // add a temporary node at its world position so the camera can target it
      this.graph.addNode(key, { x: pos[0], y: pos[1], size: 8, color: "#ffe066", label: id });
    }
    this.sigma.refresh({ skipIndexation: true });
    const d = this.sigma.getNodeDisplayData(key);
    if (d) {
      this.sigma.getCamera().animate({ x: d.x, y: d.y, ratio: 0.12 }, { duration: 600 });
    }
    // surrounding detail (and the real node attributes) load on the next refresh
    window.setTimeout(() => this.refreshViewport(), 650);
  }

  zoomIn() {
    this.sigma.getCamera().animatedZoom(1.6);
  }
  zoomOut() {
    this.sigma.getCamera().animatedUnzoom(1.6);
  }
  resetCamera() {
    this.sigma.getCamera().animate({ x: 0.5, y: 0.5, ratio: 1, angle: 0 }, { duration: 400 });
  }
}

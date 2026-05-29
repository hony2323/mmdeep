// Owns the graphology graph + Sigma renderer and implements both navigation
// modes. Kept outside React so Sigma's imperative lifecycle isn't fighting the
// render loop; React just calls methods and listens for callbacks.
import Graph from "graphology";
import Sigma from "sigma";
import { api, type Dir, type SubGraphPayload, type SubNode, type Shape } from "./api";

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

const MORE_COLOR = "#f783ac";

export interface HudInfo {
  mode: "discovery" | "overview";
  visibleNodes: number;
  visibleEdges: number;
  fps: number;
  lastQueryMs: number;
}

export interface Callbacks {
  onSelect: (node: SubNode | null) => void;
  onHud: (hud: HudInfo) => void;
  onStatus: (msg: string) => void;
}

const DISCOVERY_PAGE = 50;

export class GraphController {
  private graph: Graph;
  private sigma: Sigma;
  private cb: Callbacks;

  mode: "discovery" | "overview" = "discovery";
  dir: Dir = "both";
  private flipY = true;

  // discovery paging state
  private degree = new Map<number, number>();
  private loaded = new Map<number, number>();

  // overview state
  private overviewBounds = { minx: 0, miny: 0, maxx: 0, maxy: 0 };
  private refreshTimer: number | null = null;

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
      labelRenderedSizeThreshold: 8,
      labelDensity: 0.6,
      labelGridCellSize: 80,
      defaultEdgeColor: "#3a4254",
      defaultNodeColor: "#6ea8fe",
      labelColor: { color: "#dfe5f0" },
      labelFont: "Inter, system-ui, sans-serif",
      labelSize: 12,
      zIndex: true,
    });

    this.sigma.on("clickNode", ({ node }) => this.handleClick(node));
    this.sigma.on("enterNode", ({ node }) => this.emitSelect(node));

    // LOD: hide labels when zoomed out; fade distant edges.
    this.sigma.setSetting("nodeReducer", (_key, data) => {
      const ratio = this.sigma.getCamera().getState().ratio;
      const res: any = { ...data };
      if (this.mode === "overview" && ratio > 1.2) res.label = "";
      return res;
    });
    this.sigma.setSetting("edgeReducer", (_key, data) => {
      const ratio = this.sigma.getCamera().getState().ratio;
      const res: any = { ...data };
      if (this.mode === "overview" && ratio > 2.0) res.hidden = true;
      return res;
    });

    this.sigma.getCamera().on("updated", () => {
      if (this.mode === "overview") this.scheduleViewportRefresh();
    });

    // fps + hud ticker
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

  // -- shared helpers --------------------------------------------------------

  private gx(n: SubNode) {
    return n.x;
  }
  private gy(n: SubNode) {
    return this.flipY ? -n.y : n.y;
  }

  private styleNode(n: SubNode, x: number, y: number) {
    const hasMore = n.truncated > 0;
    const deg = n.out_degree + n.in_degree;
    return {
      x,
      y,
      size: Math.max(3, Math.min(22, 3 + Math.sqrt(deg) * 1.6)),
      color: SHAPE_COLORS[n.shape] ?? SHAPE_COLORS.default,
      borderColor: hasMore ? MORE_COLOR : undefined,
      label: n.label,
      name: n.name,
      shape: n.shape,
      out_degree: n.out_degree,
      in_degree: n.in_degree,
      truncated: n.truncated,
      zIndex: deg,
    };
  }

  private mergePayload(payload: SubGraphPayload, anchorId?: number) {
    let dx = 0;
    let dy = 0;
    if (anchorId != null && this.graph.hasNode(String(anchorId))) {
      const a = payload.nodes.find((n) => n.id === anchorId);
      if (a) {
        dx = (this.graph.getNodeAttribute(String(anchorId), "x") as number) - this.gx(a);
        dy = (this.graph.getNodeAttribute(String(anchorId), "y") as number) - this.gy(a);
      }
    }
    for (const n of payload.nodes) {
      const key = String(n.id);
      const x = this.gx(n) + dx;
      const y = this.gy(n) + dy;
      if (!this.graph.hasNode(key)) {
        this.graph.addNode(key, this.styleNode(n, x, y));
        this.degree.set(n.id, n.out_degree + n.in_degree);
        if (!this.loaded.has(n.id)) this.loaded.set(n.id, 0);
      } else {
        // refresh the "+N more" badge as more neighbours come in
        this.graph.setNodeAttribute(key, "truncated", n.truncated);
        this.graph.setNodeAttribute(
          key,
          "borderColor",
          n.truncated > 0 ? MORE_COLOR : undefined,
        );
      }
    }
    for (const e of payload.edges) {
      const s = String(e.source);
      const t = String(e.target);
      if (this.graph.hasNode(s) && this.graph.hasNode(t)) {
        const ek = `${e.source}->${e.target}`;
        if (!this.graph.hasEdge(ek)) {
          this.graph.addEdgeWithKey(ek, s, t, {
            size: e.style === "thick" ? 2.5 : 1,
            color: e.style === "dotted" ? "#566076" : "#3a4254",
            type: "arrow",
            label: e.label ?? undefined,
          });
        }
      }
    }
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
      truncated: a.truncated,
      x: a.x,
      y: a.y,
    });
  }

  private emitHud() {
    this.cb.onHud({
      mode: this.mode,
      visibleNodes: this.graph.order,
      visibleEdges: this.graph.size,
      fps: this.fps,
      lastQueryMs: this.lastQueryMs,
    });
  }

  setDir(dir: Dir) {
    this.dir = dir;
  }

  // -- discovery mode --------------------------------------------------------

  async loadRoots(limit = 60) {
    this.mode = "discovery";
    this.flipY = true;
    this.graph.clear();
    this.degree.clear();
    this.loaded.clear();
    const t = performance.now();
    const payload = await api.getRoots(limit);
    this.lastQueryMs = performance.now() - t;
    this.mergePayload(payload);
    // initialise loaded counts from what's already shown
    for (const n of payload.nodes) {
      this.loaded.set(n.id, (this.loaded.get(n.id) ?? 0));
    }
    this.fit();
    this.cb.onStatus(
      `Loaded ${payload.nodes.length} seed nodes. Click a node to expand its neighbours.`,
    );
    this.emitHud();
  }

  private async handleClick(nodeKey: string) {
    this.emitSelect(nodeKey);
    if (this.mode === "discovery") {
      await this.pageNeighbors(Number(nodeKey));
    } else {
      // overview: focus + page neighbours into a fresh discovery view
      await this.focusInDiscovery(Number(nodeKey));
    }
  }

  /** Page in the next chunk of a node's neighbours — "pagination for graphs". */
  async pageNeighbors(id: number) {
    const offset = this.loaded.get(id) ?? 0;
    const t = performance.now();
    const payload = await api.neighborsPage(id, this.dir, offset, DISCOVERY_PAGE);
    this.lastQueryMs = performance.now() - t;
    const before = this.graph.order;
    this.mergePayload(payload, id);
    const added = this.graph.order - before;
    // advance the page cursor by the neighbours actually returned (minus self)
    const returnedNeighbors = Math.max(0, payload.nodes.length - 1);
    this.loaded.set(id, offset + returnedNeighbors);
    this.cb.onStatus(
      `+${added} node(s) from ${this.graph.getNodeAttribute(String(id), "name")}` +
        (payload.truncated ? " (more available — click again)" : ""),
    );
    this.emitHud();
  }

  private async focusInDiscovery(id: number) {
    this.mode = "discovery";
    this.flipY = true;
    this.graph.clear();
    this.degree.clear();
    this.loaded.clear();
    const t = performance.now();
    const payload = await api.expand([id], 1, 200, this.dir);
    this.lastQueryMs = performance.now() - t;
    this.mergePayload(payload);
    this.loaded.set(id, Math.max(0, payload.nodes.length - 1));
    this.fit();
    this.emitHud();
  }

  /** Center the view on a searched node and expand it. */
  async focusNode(id: number) {
    await this.focusInDiscovery(id);
    if (this.graph.hasNode(String(id))) {
      this.emitSelect(String(id));
      this.fit();
    }
  }

  // -- overview mode ---------------------------------------------------------

  async enterOverview() {
    this.cb.onStatus("Computing overview layout… (cached after first run)");
    const info = await api.ensureOverview();
    this.overviewBounds = {
      minx: info.minx,
      miny: info.miny,
      maxx: info.maxx,
      maxy: info.maxy,
    };
    this.mode = "overview";
    this.flipY = false;
    this.graph.clear();
    this.degree.clear();
    this.loaded.clear();
    this.cb.onStatus(
      `Overview ready: ${info.node_count.toLocaleString()} nodes, ` +
        `${info.edge_count.toLocaleString()} edges (layout ${info.compute_ms.toFixed(0)} ms).`,
    );
    this.fitBounds(this.overviewBounds);
    await this.refreshViewport();
  }

  private scheduleViewportRefresh() {
    if (this.refreshTimer) window.clearTimeout(this.refreshTimer);
    this.refreshTimer = window.setTimeout(() => this.refreshViewport(), 140);
  }

  private async refreshViewport() {
    if (this.mode !== "overview") return;
    const { width, height } = this.sigma.getDimensions();
    const tl = this.sigma.viewportToGraph({ x: 0, y: 0 });
    const br = this.sigma.viewportToGraph({ x: width, y: height });
    const minx = Math.min(tl.x, br.x);
    const maxx = Math.max(tl.x, br.x);
    const miny = Math.min(tl.y, br.y);
    const maxy = Math.max(tl.y, br.y);
    // LOD budget: fewer nodes when zoomed out (larger ratio).
    const ratio = this.sigma.getCamera().getState().ratio;
    const budget = ratio > 2 ? 1500 : ratio > 0.8 ? 4000 : 8000;

    const t = performance.now();
    const payload = await api.viewport(minx, miny, maxx, maxy, budget);
    this.lastQueryMs = performance.now() - t;

    // rebuild the visible set
    this.graph.clear();
    for (const n of payload.nodes) {
      this.graph.addNode(String(n.id), this.styleNode(n, this.gx(n), this.gy(n)));
    }
    for (const e of payload.edges) {
      const s = String(e.source);
      const t2 = String(e.target);
      if (this.graph.hasNode(s) && this.graph.hasNode(t2)) {
        const ek = `${e.source}->${e.target}`;
        if (!this.graph.hasEdge(ek))
          this.graph.addEdgeWithKey(ek, s, t2, { size: 0.8, type: "arrow" });
      }
    }
    this.emitHud();
  }

  // -- camera helpers --------------------------------------------------------

  private fit() {
    // let sigma autoscale to the current graph extent
    this.sigma.getCamera().animatedReset();
  }

  private fitBounds(_b: { minx: number; miny: number; maxx: number; maxy: number }) {
    this.sigma.getCamera().animatedReset();
  }

  zoomIn() {
    this.sigma.getCamera().animatedZoom(1.5);
  }
  zoomOut() {
    this.sigma.getCamera().animatedUnzoom(1.5);
  }
  resetCamera() {
    this.sigma.getCamera().animatedReset();
  }
}

// Typed wrappers over the Tauri backend commands. Field names are snake_case
// to match serde's default serialization of the Rust structs.
import { invoke } from "@tauri-apps/api/core";

export type Shape =
  | "default"
  | "rect"
  | "round-rect"
  | "stadium"
  | "subroutine"
  | "cylinder"
  | "circle"
  | "diamond"
  | "hexagon"
  | "parallelogram"
  | "trapezoid"
  | "asymmetric";

export type EdgeStyle = "normal" | "thick" | "dotted";

export interface Stats {
  path: string;
  node_count: number;
  edge_count: number;
  direction: string;
  root_count: number;
  parse_ms: number;
  approx_bytes: number;
  has_overview: boolean;
}

export interface SubNode {
  id: number;
  name: string;
  label: string;
  shape: Shape;
  out_degree: number;
  in_degree: number;
  truncated: number;
  x: number;
  y: number;
}

export interface SubEdge {
  source: number;
  target: number;
  label: string | null;
  style: EdgeStyle;
}

export interface SubGraphPayload {
  nodes: SubNode[];
  edges: SubEdge[];
  truncated: boolean;
}

export interface OverviewInfo {
  minx: number;
  miny: number;
  maxx: number;
  maxy: number;
  node_count: number;
  edge_count: number;
  iterations: number;
  compute_ms: number;
}

export type Dir = "out" | "in" | "both";

export const api = {
  startupFile: () => invoke<string | null>("startup_file"),
  openFile: (path: string) => invoke<Stats>("open_file", { path }),
  stats: () => invoke<Stats>("stats"),
  getRoots: (limit: number) => invoke<SubGraphPayload>("get_roots", { limit }),
  expand: (seeds: number[], depth: number, limit: number, dir: Dir) =>
    invoke<SubGraphPayload>("expand", { seeds, depth, limit, dir }),
  neighborsPage: (node: number, dir: Dir, offset: number, count: number) =>
    invoke<SubGraphPayload>("neighbors_page", { node, dir, offset, count }),
  search: (query: string, limit: number) =>
    invoke<SubNode[]>("search", { query, limit }),
  ensureOverview: (iterations?: number) =>
    invoke<OverviewInfo>("ensure_overview", { iterations: iterations ?? null }),
  locate: (node: number) => invoke<[number, number] | null>("locate", { node }),
  viewport: (
    minx: number,
    miny: number,
    maxx: number,
    maxy: number,
    maxNodes: number,
  ) => invoke<SubGraphPayload>("viewport", { minx, miny, maxx, maxy, maxNodes }),
};

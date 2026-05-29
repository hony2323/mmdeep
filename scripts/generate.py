#!/usr/bin/env python3
"""Generate large Mermaid (.mmd) flowchart files for testing mmdeep.

The generators stream edges straight to disk so even the 1,000,000-edge
preset stays at near-constant memory. Topologies are implemented directly in
pure Python; networkx is only used (optionally) for the scale-free generator
when available, otherwise a built-in Barabasi-Albert sampler is used.

Examples:
    python scripts/generate.py --preset small --out examples/small.mmd
    python scripts/generate.py --topology tree --nodes 50000 --out examples/tree.mmd
    python scripts/generate.py --preset large --topology dag --out examples/large.mmd
"""

from __future__ import annotations

import argparse
import random
import time
from pathlib import Path

# preset -> (nodes, edges). edges is a target; some topologies derive edges
# from structure (e.g. tree has nodes-1 edges).
PRESETS = {
    "tiny": (1_000, 1_000),
    "small": (8_000, 10_000),
    "medium": (80_000, 100_000),
    "large": (700_000, 1_000_000),
    "huge": (3_500_000, 5_000_000),
}

DIRECTIONS = ("TD", "LR", "TB", "RL", "BT")


def human(n: int) -> str:
    for unit in ("", "K", "M", "B"):
        if abs(n) < 1000:
            return f"{n}{unit}"
        n //= 1000
    return f"{n}T"


class MmdWriter:
    """Buffered Mermaid writer. Emits a `flowchart` header then edges/nodes."""

    def __init__(self, fh, direction: str, labels: bool, subgraphs: int):
        self.fh = fh
        self.labels = labels
        self.subgraphs = subgraphs
        self.edge_count = 0
        self.node_count = 0
        self._buf: list[str] = []
        self._buf_lines = 0
        fh.write(f"flowchart {direction}\n")

    def _flush(self):
        if self._buf:
            self.fh.write("".join(self._buf))
            self._buf.clear()
            self._buf_lines = 0

    def _emit(self, line: str):
        self._buf.append(line)
        self._buf_lines += 1
        if self._buf_lines >= 8192:
            self._flush()

    def node_decl(self, idx: int):
        """Optionally declare a node with a shape/label so files exercise the
        parser's shape handling. Most nodes are declared implicitly via edges."""
        self.node_count = max(self.node_count, idx + 1)
        if not self.labels:
            return
        shape = idx % 5
        text = f"Node {idx}"
        if shape == 0:
            self._emit(f'  n{idx}["{text}"]\n')
        elif shape == 1:
            self._emit(f"  n{idx}(({text}))\n")
        elif shape == 2:
            self._emit(f"  n{idx}{{{text}}}\n")
        elif shape == 3:
            self._emit(f"  n{idx}([{text}])\n")
        else:
            self._emit(f"  n{idx}[/{text}/]\n")

    def edge(self, a: int, b: int, label: str | None = None):
        self.node_count = max(self.node_count, a + 1, b + 1)
        if label is not None and self.labels:
            self._emit(f"  n{a} -->|{label}| n{b}\n")
        else:
            self._emit(f"  n{a} --> n{b}\n")
        self.edge_count += 1

    def subgraph_open(self, name: str):
        self._emit(f"  subgraph {name}\n")

    def subgraph_close(self):
        self._emit("  end\n")

    def close(self):
        self._flush()


# --------------------------------------------------------------------------
# Topology generators. Each yields (a, b) edge tuples and decides node count.
# --------------------------------------------------------------------------


def gen_tree(w: MmdWriter, nodes: int, branching: int, rng: random.Random):
    """k-ary tree: node i's parent is (i-1)//branching."""
    for i in range(1, nodes):
        parent = (i - 1) // branching
        w.edge(parent, i, label=f"e{i}" if w.labels else None)


def gen_dag(w: MmdWriter, nodes: int, edges: int, layers: int, rng: random.Random):
    """Layered DAG: nodes split into layers; edges go forward only."""
    layers = max(2, min(layers, nodes))
    per = max(1, nodes // layers)
    layer_of = [min(i // per, layers - 1) for i in range(nodes)]
    # bucket node ids by layer for forward sampling
    buckets: list[list[int]] = [[] for _ in range(layers)]
    for i in range(nodes):
        buckets[layer_of[i]].append(i)
    produced = 0
    # guarantee connectivity: each non-first-layer node gets one back-edge
    for i in range(nodes):
        L = layer_of[i]
        if L == 0:
            continue
        src = rng.choice(buckets[L - 1])
        w.edge(src, i)
        produced += 1
    # fill remaining budget with random forward edges
    while produced < edges:
        L = rng.randint(0, layers - 2)
        a = rng.choice(buckets[L])
        b = rng.choice(buckets[L + 1])
        w.edge(a, b, label=f"e{produced}" if w.labels else None)
        produced += 1


def gen_scale_free(w: MmdWriter, nodes: int, m: int, rng: random.Random):
    """Barabasi-Albert preferential attachment. `m` edges per new node."""
    m = max(1, min(m, nodes - 1))
    # `repeated` is a multiset of node ids; sampling from it gives degree-bias.
    repeated: list[int] = []
    # seed clique
    for i in range(m):
        for j in range(i + 1, m):
            w.edge(i, j)
            repeated.extend((i, j))
    for new in range(m, nodes):
        targets: set[int] = set()
        while len(targets) < m:
            if repeated:
                targets.add(rng.choice(repeated))
            else:
                targets.add(rng.randrange(new))
        for t in targets:
            w.edge(new, t)
            repeated.extend((new, t))


def gen_random(w: MmdWriter, nodes: int, edges: int, rng: random.Random):
    """Erdos-Renyi-style: sample random directed edges (may repeat rarely)."""
    # spanning path first so the graph is reachable
    for i in range(1, min(nodes, edges + 1)):
        w.edge(i - 1, i)
    produced = max(0, min(nodes, edges + 1) - 1)
    while produced < edges:
        a = rng.randrange(nodes)
        b = rng.randrange(nodes)
        if a != b:
            w.edge(a, b)
            produced += 1


def gen_grid(w: MmdWriter, nodes: int, rng: random.Random):
    """2D grid/mesh: each cell links right and down."""
    side = max(2, int(nodes**0.5))
    n = side * side
    for r in range(side):
        for c in range(side):
            i = r * side + c
            if c + 1 < side:
                w.edge(i, i + 1)
            if r + 1 < side:
                w.edge(i, i + side)
    return n


def gen_clustered(
    w: MmdWriter, nodes: int, edges: int, communities: int, rng: random.Random
):
    """Communities of densely-linked nodes with sparse inter-community links.
    When subgraphs are enabled each community is wrapped in a subgraph block."""
    communities = max(2, min(communities, nodes // 2))
    size = max(2, nodes // communities)
    inter_ratio = 0.05
    produced = 0
    target_inter = int(edges * inter_ratio)
    bounds = []
    for c in range(communities):
        start = c * size
        end = min(nodes, start + size)
        if start >= nodes:
            break
        bounds.append((start, end))
        if w.subgraphs:
            w.subgraph_open(f"cluster_{c}")
        # dense intra-community ring + chords
        for i in range(start, end):
            nxt = start + ((i - start + 1) % (end - start))
            w.edge(i, nxt)
            produced += 1
        if w.subgraphs:
            w.subgraph_close()
    # sparse inter-community links
    while produced < edges and len(bounds) > 1:
        ca, cb = rng.sample(range(len(bounds)), 2)
        a = rng.randrange(*bounds[ca])
        b = rng.randrange(*bounds[cb])
        w.edge(a, b, label="x" if w.labels else None)
        produced += 1
        if produced >= target_inter and produced >= edges:
            break


def build(args) -> tuple[int, int, float]:
    rng = random.Random(args.seed)
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    t0 = time.perf_counter()
    with out.open("w", encoding="utf-8", newline="\n") as fh:
        w = MmdWriter(fh, args.direction, args.labels, args.subgraphs)
        if args.labels and args.topology in ("tree", "dag", "random"):
            # declare a sample of nodes with shapes (avoid declaring millions)
            for i in range(0, min(args.nodes, 2000)):
                w.node_decl(i)
        if args.topology == "tree":
            gen_tree(w, args.nodes, args.branching, rng)
        elif args.topology == "dag":
            gen_dag(w, args.nodes, args.edges, args.layers, rng)
        elif args.topology == "scale-free":
            gen_scale_free(w, args.nodes, args.m, rng)
        elif args.topology == "random":
            gen_random(w, args.nodes, args.edges, rng)
        elif args.topology == "grid":
            gen_grid(w, args.nodes, rng)
        elif args.topology == "clustered":
            gen_clustered(w, args.nodes, args.edges, args.communities, rng)
        else:
            raise SystemExit(f"unknown topology: {args.topology}")
        w.close()
        node_count, edge_count = w.node_count, w.edge_count
    return node_count, edge_count, time.perf_counter() - t0


def main(argv=None):
    p = argparse.ArgumentParser(description="Generate large Mermaid .mmd files.")
    p.add_argument(
        "--preset",
        choices=PRESETS.keys(),
        help="convenience size preset (sets --nodes/--edges)",
    )
    p.add_argument(
        "--topology",
        default="scale-free",
        choices=["tree", "dag", "scale-free", "random", "grid", "clustered"],
    )
    p.add_argument("--nodes", type=int, help="number of nodes")
    p.add_argument("--edges", type=int, help="target number of edges")
    p.add_argument("--out", required=True, help="output .mmd path")
    p.add_argument("--direction", default="TD", choices=DIRECTIONS)
    p.add_argument(
        "--labels", action="store_true", help="emit edge/node labels & shapes"
    )
    p.add_argument(
        "--subgraphs",
        type=int,
        default=0,
        help="wrap clustered output in N subgraph blocks (clustered only)",
    )
    p.add_argument("--seed", type=int, default=42)
    # topology-specific knobs
    p.add_argument("--branching", type=int, default=3, help="tree branching factor")
    p.add_argument("--layers", type=int, default=50, help="dag layer count")
    p.add_argument("--m", type=int, default=2, help="scale-free edges per new node")
    p.add_argument(
        "--communities", type=int, default=100, help="clustered community count"
    )
    args = p.parse_args(argv)

    if args.preset:
        pn, pe = PRESETS[args.preset]
        if args.nodes is None:
            args.nodes = pn
        if args.edges is None:
            args.edges = pe
    if args.nodes is None:
        args.nodes = 10_000
    if args.edges is None:
        args.edges = args.nodes

    node_count, edge_count, secs = build(args)
    size_mb = Path(args.out).stat().st_size / (1024 * 1024)
    print(f"wrote {args.out}")
    print(f"  topology : {args.topology}")
    print(f"  nodes    : {node_count:,} ({human(node_count)})")
    print(f"  edges    : {edge_count:,} ({human(edge_count)})")
    print(f"  size     : {size_mb:.1f} MB")
    print(f"  gen time : {secs:.2f}s")


if __name__ == "__main__":
    main()

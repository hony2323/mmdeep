#!/usr/bin/env python3
"""Benchmark harness for mmdeep.

Generates a matrix of graphs (if not already present), drives the headless
`mmdeep-core` binary's `bench` command on each, and writes a JSON + a readable
table to benchmarks/. This measures the engine without any GUI in the timing
path: parse, roots/first-render, expand latency, overview layout, and viewport
query.

    python scripts/bench.py                 # default matrix
    python scripts/bench.py --quick         # skip the 1M preset
    python scripts/bench.py --iterations 40 # overview layout iterations
"""

from __future__ import annotations

import argparse
import json
import platform
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
GEN = ROOT / "scripts" / "generate.py"
EXAMPLES = ROOT / "examples"
OUT = ROOT / "benchmarks"

# (label, preset, topology)
MATRIX = [
    ("tiny-scalefree", "tiny", "scale-free"),
    ("small-scalefree", "small", "scale-free"),
    ("small-tree", "small", "tree"),
    ("medium-scalefree", "medium", "scale-free"),
    ("medium-dag", "medium", "dag"),
    ("large-scalefree", "large", "scale-free"),
]


def find_cli() -> Path:
    candidates = [
        ROOT / "target" / "release" / "mmdeep-core.exe",
        ROOT / "target" / "release" / "mmdeep-core",
        ROOT / "crates" / "mmdeep-core" / "target" / "release" / "mmdeep-core.exe",
        ROOT / "crates" / "mmdeep-core" / "target" / "release" / "mmdeep-core",
    ]
    for c in candidates:
        if c.exists():
            return c
    sys.exit(
        "mmdeep-core release binary not found. Build it first:\n"
        "  cargo build --release -p mmdeep-core"
    )


def ensure_file(label: str, preset: str, topology: str) -> Path:
    out = EXAMPLES / f"bench-{label}.mmd"
    if out.exists():
        return out
    print(f"  generating {out.name} ({preset}/{topology}) ...")
    subprocess.run(
        [
            sys.executable,
            str(GEN),
            "--preset",
            preset,
            "--topology",
            topology,
            "--out",
            str(out),
        ],
        check=True,
        stdout=subprocess.DEVNULL,
    )
    return out


def run_bench(cli: Path, file: Path, iterations: int) -> dict:
    # remove any layout cache so overview_ms reflects a true first-time compute
    cache = Path(str(file) + ".mmdlayout")
    if cache.exists():
        cache.unlink()
    proc = subprocess.run(
        [
            str(cli),
            "bench",
            str(file),
            "--iterations",
            str(iterations),
            "--limit",
            "300",
        ],
        check=True,
        capture_output=True,
        text=True,
    )
    return json.loads(proc.stdout.strip())


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--quick", action="store_true", help="skip the 1M (large) preset")
    ap.add_argument("--iterations", type=int, default=40)
    args = ap.parse_args()

    cli = find_cli()
    OUT.mkdir(exist_ok=True)
    matrix = [m for m in MATRIX if not (args.quick and m[1] == "large")]

    results = []
    print(f"benchmarking with {cli.name} ({len(matrix)} graphs)\n")
    for label, preset, topology in matrix:
        f = ensure_file(label, preset, topology)
        print(f"  running {label} ...", flush=True)
        t0 = time.perf_counter()
        r = run_bench(cli, f, args.iterations)
        r["label"] = label
        r["topology"] = topology
        r["wall_s"] = round(time.perf_counter() - t0, 2)
        results.append(r)

    payload = {
        "machine": {
            "platform": platform.platform(),
            "python": platform.python_version(),
            "processor": platform.processor(),
        },
        "iterations": args.iterations,
        "results": results,
    }
    out_json = OUT / "results.json"
    out_json.write_text(json.dumps(payload, indent=2))

    # readable table
    print("\n" + "=" * 104)
    hdr = (
        f"{'graph':22}{'nodes':>10}{'edges':>11}{'mem MB':>9}{'parse ms':>10}"
        f"{'roots ms':>10}{'expand ms':>11}{'overview ms':>13}{'vp ms':>8}"
    )
    print(hdr)
    print("-" * 104)
    for r in results:
        print(
            f"{r['label']:22}{r['nodes']:>10,}{r['edges']:>11,}{r['approx_mb']:>9.1f}"
            f"{r['parse_ms']:>10.0f}{r['roots_ms']:>10.2f}{r['expand_avg_ms']:>11.2f}"
            f"{r['overview_ms']:>13.0f}{r['viewport_ms']:>8.2f}"
        )
    print("=" * 104)
    print(f"\nwrote {out_json}")


if __name__ == "__main__":
    main()

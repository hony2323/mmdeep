# Examples

Sample Mermaid flowcharts for testing and benchmarking mmdeep. Small/medium
files are committed; the 1M-edge `large.mmd` is generated on demand (it is
git-ignored because of its size).

Generate any of them with `scripts/generate.py`:

```bash
# committed samples
python scripts/generate.py --preset tiny    --topology scale-free --out examples/tiny.mmd
python scripts/generate.py --preset small   --topology scale-free --out examples/small.mmd
python scripts/generate.py --preset medium  --topology scale-free --out examples/medium.mmd
python scripts/generate.py --topology tree      --nodes 20000              --out examples/tree.mmd
python scripts/generate.py --topology clustered --nodes 5000 --communities 50 --subgraphs 50 --labels --out examples/clustered.mmd

# the big one (≈1.4M edges, ~28 MB — not committed)
python scripts/generate.py --preset large --topology scale-free --out examples/large.mmd
```

| file             | topology    | nodes  | edges  | notes                         |
| ---------------- | ----------- | ------ | ------ | ----------------------------- |
| `tiny.mmd`       | scale-free  | 1k     | ~2k    | smoke test                    |
| `small.mmd`      | scale-free  | 8k     | ~10k   | already beyond most tools     |
| `medium.mmd`     | scale-free  | 80k    | ~160k  | exercises discovery + overview|
| `tree.mmd`       | tree        | 20k    | ~20k   | clean layered layout          |
| `clustered.mmd`  | clustered   | 5k     | ~5k    | subgraphs + labels            |
| `large.mmd`      | scale-free  | 700k   | ~1.4M  | the headline 1M-edge target   |

Opening a file the first time in Overview mode computes a force layout and
writes a `<file>.mmdlayout` cache next to it; subsequent opens load instantly.

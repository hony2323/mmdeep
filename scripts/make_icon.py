#!/usr/bin/env python3
"""Generate a source PNG icon for mmdeep using only the standard library.

Draws a dark rounded tile with a small node-graph motif. Feed the output to
`npm run tauri icon scripts/app-icon.png` to produce all platform icon sizes.
"""

from __future__ import annotations

import math
import struct
import zlib
from pathlib import Path

SIZE = 512
BG = (13, 16, 23)
TILE = (27, 34, 48)
NODE_COLORS = [(110, 168, 254), (99, 230, 190), (247, 131, 172), (255, 212, 59)]
EDGE = (58, 66, 84)


def make_canvas():
    return [[BG[0], BG[1], BG[2], 255] for _ in range(SIZE * SIZE)]


def idx(x, y):
    return y * SIZE + x


def rounded_tile(buf, radius=90, margin=40):
    lo, hi = margin, SIZE - margin
    for y in range(SIZE):
        for x in range(SIZE):
            if x < lo or x > hi or y < lo or y > hi:
                continue
            # rounded corners
            cx = min(max(x, lo + radius), hi - radius)
            cy = min(max(y, lo + radius), hi - radius)
            if (x - cx) ** 2 + (y - cy) ** 2 <= radius**2:
                buf[idx(x, y)] = [TILE[0], TILE[1], TILE[2], 255]


def blend(buf, x, y, color, a):
    if x < 0 or y < 0 or x >= SIZE or y >= SIZE:
        return
    p = buf[idx(x, y)]
    for c in range(3):
        p[c] = int(p[c] * (1 - a) + color[c] * a)


def draw_disc(buf, cx, cy, r, color):
    for y in range(int(cy - r - 1), int(cy + r + 2)):
        for x in range(int(cx - r - 1), int(cx + r + 2)):
            d = math.hypot(x - cx, y - cy)
            if d <= r:
                blend(buf, x, y, color, 1.0)
            elif d <= r + 1.5:
                blend(buf, x, y, color, max(0.0, (r + 1.5 - d) / 1.5))


def draw_edge(buf, a, b, color, w=7):
    (x0, y0), (x1, y1) = a, b
    steps = int(math.hypot(x1 - x0, y1 - y0)) + 1
    for i in range(steps + 1):
        t = i / steps
        x = x0 + (x1 - x0) * t
        y = y0 + (y1 - y0) * t
        draw_disc(buf, x, y, w / 2, color)


def write_png(path, buf):
    raw = bytearray()
    for y in range(SIZE):
        raw.append(0)  # filter type 0
        for x in range(SIZE):
            raw.extend(buf[idx(x, y)])
    compressed = zlib.compress(bytes(raw), 9)

    def chunk(tag, data):
        return (
            struct.pack(">I", len(data))
            + tag
            + data
            + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
        )

    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", SIZE, SIZE, 8, 6, 0, 0, 0))
    png += chunk(b"IDAT", compressed)
    png += chunk(b"IEND", b"")
    Path(path).write_bytes(png)


def main():
    buf = make_canvas()
    rounded_tile(buf)
    nodes = [(180, 150), (350, 200), (200, 340), (360, 360)]
    edges = [(0, 1), (0, 2), (1, 3), (2, 3), (1, 2)]
    for a, b in edges:
        draw_edge(buf, nodes[a], nodes[b], EDGE)
    for i, (x, y) in enumerate(nodes):
        draw_disc(buf, x, y, 30, NODE_COLORS[i % len(NODE_COLORS)])
    out = Path(__file__).parent / "app-icon.png"
    write_png(out, buf)
    print(f"wrote {out} ({out.stat().st_size} bytes)")


if __name__ == "__main__":
    main()

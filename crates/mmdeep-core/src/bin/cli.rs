//! mmdeep-core — headless CLI over the core library.
//!
//! Used for testing and for clean benchmarking (no GUI in the timing path).
//!
//! Usage:
//!   mmdeep-core stats <file.mmd> [--json]
//!   mmdeep-core roots <file.mmd> [--limit N]
//!   mmdeep-core expand <file.mmd> --seed NAME [--depth D] [--limit L] [--dir out|in|both] [--json]
//!   mmdeep-core search <file.mmd> --query Q [--limit N]
//!   mmdeep-core overview <file.mmd> [--iterations N]
//!   mmdeep-core bench <file.mmd> [--iterations N] [--limit L]

use mmdeep_core::Document;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: mmdeep-core <stats|roots|expand|search|overview|bench> <file.mmd> [opts]");
        std::process::exit(2);
    }
    let cmd = args[1].as_str();
    let file = args[2].as_str();
    let opts = Opts::parse(&args[3..]);

    match cmd {
        "stats" => cmd_stats(file, &opts),
        "roots" => cmd_roots(file, &opts),
        "expand" => cmd_expand(file, &opts),
        "search" => cmd_search(file, &opts),
        "overview" => cmd_overview(file, &opts),
        "bench" => cmd_bench(file, &opts),
        other => {
            eprintln!("unknown command: {other}");
            std::process::exit(2);
        }
    }
}

#[derive(Default)]
struct Opts {
    json: bool,
    seed: Option<String>,
    query: Option<String>,
    depth: u32,
    limit: usize,
    iterations: usize,
    dir: String,
}

impl Opts {
    fn parse(args: &[String]) -> Opts {
        let mut o = Opts {
            depth: 2,
            limit: 200,
            iterations: 80,
            dir: "both".into(),
            ..Default::default()
        };
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--json" => o.json = true,
                "--seed" => {
                    o.seed = args.get(i + 1).cloned();
                    i += 1;
                }
                "--query" => {
                    o.query = args.get(i + 1).cloned();
                    i += 1;
                }
                "--depth" => {
                    o.depth = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(2);
                    i += 1;
                }
                "--limit" => {
                    o.limit = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(200);
                    i += 1;
                }
                "--iterations" => {
                    o.iterations = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(80);
                    i += 1;
                }
                "--dir" => {
                    o.dir = args.get(i + 1).cloned().unwrap_or_else(|| "both".into());
                    i += 1;
                }
                _ => {}
            }
            i += 1;
        }
        o
    }
}

fn open(file: &str) -> Document {
    match Document::open(file) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("failed to open {file}: {e}");
            std::process::exit(1);
        }
    }
}

fn cmd_stats(file: &str, o: &Opts) {
    let doc = open(file);
    let s = doc.stats();
    if o.json {
        println!("{}", serde_json::to_string_pretty(&s).unwrap());
    } else {
        println!("path       : {}", s.path);
        println!("nodes      : {}", s.node_count);
        println!("edges      : {}", s.edge_count);
        println!("direction  : {}", s.direction);
        println!("roots      : {}", s.root_count);
        println!("parse time : {:.1} ms", s.parse_ms);
        println!("approx mem : {:.1} MB", s.approx_bytes as f64 / 1e6);
    }
}

fn cmd_roots(file: &str, o: &Opts) {
    let doc = open(file);
    let p = doc.roots(o.limit);
    if o.json {
        println!("{}", serde_json::to_string(&p).unwrap());
    } else {
        for n in &p.nodes {
            println!("{}  (out {}, in {}, +{} more)", n.name, n.out_degree, n.in_degree, n.truncated);
        }
    }
}

fn cmd_expand(file: &str, o: &Opts) {
    let doc = open(file);
    let seed = o.seed.as_deref().unwrap_or_else(|| {
        eprintln!("--seed NAME is required for expand");
        std::process::exit(2);
    });
    let Some(id) = doc.graph.id_of(seed) else {
        eprintln!("no such node: {seed}");
        std::process::exit(1);
    };
    let t0 = Instant::now();
    let p = doc.expand(&[id], o.depth, o.limit, &o.dir);
    let ms = t0.elapsed().as_secs_f64() * 1000.0;
    if o.json {
        println!("{}", serde_json::to_string(&p).unwrap());
    } else {
        println!("expanded {} -> {} nodes, {} edges in {:.2} ms (truncated={})",
            seed, p.nodes.len(), p.edges.len(), ms, p.truncated);
    }
}

fn cmd_search(file: &str, o: &Opts) {
    let doc = open(file);
    let q = o.query.as_deref().unwrap_or_else(|| {
        eprintln!("--query Q is required for search");
        std::process::exit(2);
    });
    let res = doc.search(q, o.limit);
    if o.json {
        println!("{}", serde_json::to_string(&res).unwrap());
    } else {
        for n in &res {
            println!("{}  ({})", n.name, n.label);
        }
        println!("{} matches", res.len());
    }
}

fn cmd_overview(file: &str, o: &Opts) {
    let mut doc = open(file);
    let t0 = Instant::now();
    doc.ensure_overview(o.iterations).unwrap();
    let ms = t0.elapsed().as_secs_f64() * 1000.0;
    let (minx, miny, maxx, maxy) = doc.overview_bounds().unwrap();
    println!("overview computed in {:.1} ms ({} iterations)", ms, o.iterations);
    println!("bounds: [{:.0}, {:.0}] .. [{:.0}, {:.0}]", minx, miny, maxx, maxy);
}

fn cmd_bench(file: &str, o: &Opts) {
    // parse
    let t0 = Instant::now();
    let mut doc = open(file);
    let open_ms = t0.elapsed().as_secs_f64() * 1000.0;
    let stats = doc.stats();

    // roots / first render
    let t = Instant::now();
    let _roots = doc.roots(o.limit);
    let roots_ms = t.elapsed().as_secs_f64() * 1000.0;

    // expand a high-degree node a few times
    let mut expand_ms = 0.0;
    let mut samples = 0;
    let seeds = mmdeep_core::discovery::roots(&doc.graph, 5);
    for &s in &seeds {
        let t = Instant::now();
        let _ = doc.expand(&[s], o.depth, o.limit, "both");
        expand_ms += t.elapsed().as_secs_f64() * 1000.0;
        samples += 1;
    }
    let expand_avg = if samples > 0 { expand_ms / samples as f64 } else { 0.0 };

    // overview layout + viewport
    let t = Instant::now();
    doc.ensure_overview(o.iterations).unwrap();
    let overview_ms = t.elapsed().as_secs_f64() * 1000.0;

    let (minx, miny, maxx, maxy) = doc.overview_bounds().unwrap();
    // sample a central viewport spanning ~25% of each axis
    let (cx, cy) = ((minx + maxx) / 2.0, (miny + maxy) / 2.0);
    let (wx, wy) = ((maxx - minx) * 0.25, (maxy - miny) * 0.25);
    let t = Instant::now();
    let vp = doc.viewport(cx - wx, cy - wy, cx + wx, cy + wy, 5000);
    let viewport_ms = t.elapsed().as_secs_f64() * 1000.0;

    let result = serde_json::json!({
        "file": file,
        "nodes": stats.node_count,
        "edges": stats.edge_count,
        "approx_mb": stats.approx_bytes as f64 / 1e6,
        "open_ms": open_ms,
        "parse_ms": stats.parse_ms,
        "roots_ms": roots_ms,
        "expand_avg_ms": expand_avg,
        "overview_ms": overview_ms,
        "overview_iterations": o.iterations,
        "viewport_ms": viewport_ms,
        "viewport_nodes": vp.nodes.len(),
        "viewport_edges": vp.edges.len(),
    });
    println!("{}", serde_json::to_string(&result).unwrap());
}

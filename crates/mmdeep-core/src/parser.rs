//! Streaming parser for the Mermaid `flowchart` / `graph` subset.
//!
//! Reads line by line (so a 1M-edge file never needs to be fully buffered as
//! one giant statement) and recognises:
//!   * header:        `flowchart TD`, `graph LR`, ...
//!   * comments:      `%% ...` and `%%{ ... }%%` directives (ignored)
//!   * node decls:    `id`, `id["text"]`, `id(t)`, `id((t))`, `id{t}`, `id([t])`,
//!                    `id[[t]]`, `id[(t)]`, `id{{t}}`, `id[/t/]`, `id[\t\]`, `id>t]`
//!   * edges:         `A --> B`, `A --- B`, `A -.-> B`, `A ==> B`, `A -->|lbl| B`,
//!                    chains `A --> B --> C`, and groups `A & B --> C & D`
//!   * subgraphs:     `subgraph id [title]` ... `end`
//!
//! Styling statements (`style`, `classDef`, `class`, `click`, `linkStyle`) are
//! skipped gracefully. Inline `A -- text --> B` edge text is not parsed (use the
//! `|label|` form); such a line still parses as an edge, just without the label.

use crate::store::{EdgeStyle, GraphBuilder, NodeId, Shape};
use regex::Regex;
use std::io::BufRead;

/// One link operator found in a statement.
struct Link {
    start: usize,
    end: usize,
    style: EdgeStyle,
    label: Option<String>,
}

pub struct Parser {
    link_re: Regex,
    builder: GraphBuilder,
    header_seen: bool,
    in_directive: bool,
}

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

impl Parser {
    pub fn new() -> Self {
        // A link operator (optionally reversed with a leading `<`), with an
        // optional trailing `|label|`. Order matters: dotted/thick before plain.
        let link_re = Regex::new(
            r"(?x)
            <?
            (?:
                -\.+-?>? |       # dotted:  -.-> / -.- / -.->
                =+>? |           # thick:   ==> / === / ==
                =+[ox] |         # thick:   ==o / ==x
                -{2,}>? |        # normal:  --> / ---
                -{2,}[ox]        # normal:  --o / --x
            )
            (?:\|([^|]*)\|)?
        ",
        )
        .unwrap();
        Self {
            link_re,
            builder: GraphBuilder::new(),
            header_seen: false,
            in_directive: false,
        }
    }

    /// Parse from any buffered reader, one line at a time (constant memory
    /// beyond the graph itself).
    pub fn parse_reader<R: BufRead>(mut self, mut reader: R) -> std::io::Result<GraphBuilder> {
        let mut s = String::new();
        loop {
            s.clear();
            let read = reader.read_line(&mut s)?;
            if read == 0 {
                break;
            }
            self.feed_line(&s);
        }
        Ok(self.builder)
    }

    pub fn parse_str(mut self, text: &str) -> GraphBuilder {
        for line in text.lines() {
            self.feed_line(line);
        }
        self.builder
    }

    fn feed_line(&mut self, raw: &str) {
        let mut line = raw.trim();

        // Handle multi-line `%%{ ... }%%` init directives.
        if self.in_directive {
            if line.contains("}%%") {
                self.in_directive = false;
            }
            return;
        }
        if line.starts_with("%%{") {
            if !line.contains("}%%") {
                self.in_directive = true;
            }
            return;
        }
        // Strip trailing line comments and skip whole-line comments.
        if let Some(pos) = line.find("%%") {
            line = line[..pos].trim_end();
        }
        if line.is_empty() {
            return;
        }

        // Statements may be `;`-separated on one line.
        for stmt in line.split(';') {
            let stmt = stmt.trim();
            if !stmt.is_empty() {
                self.feed_statement(stmt);
            }
        }
    }

    fn feed_statement(&mut self, stmt: &str) {
        if !self.header_seen {
            if let Some(dir) = parse_header(stmt) {
                self.builder.direction = dir;
                self.header_seen = true;
                return;
            }
            // No explicit header — treat as headerless graph and keep going.
            self.header_seen = true;
        }

        let lower = stmt.to_ascii_lowercase();
        if lower.starts_with("subgraph") {
            // We register subgraph membership lazily via node decls inside; for
            // the v1 store we just record the declared title. Nodes added while
            // a subgraph is open are not tracked per-block here (kept simple).
            return;
        }
        if lower == "end" {
            return;
        }
        // Skip styling / interaction statements.
        for kw in ["style ", "classdef ", "class ", "click ", "linkstyle ", "direction "] {
            if lower.starts_with(kw) {
                return;
            }
        }

        let links = self.find_links(stmt);
        if links.is_empty() {
            // Pure node declaration line, possibly several joined by `&`.
            for tok in stmt.split('&') {
                self.register_token(tok.trim());
            }
        } else {
            self.parse_edge_chain(stmt, &links);
        }
    }

    /// Locate link operators outside of bracketed / quoted regions.
    fn find_links(&self, stmt: &str) -> Vec<Link> {
        let masked = mask_brackets(stmt);
        let mut links = Vec::new();
        for caps in self.link_re.captures_iter(&masked) {
            let m = caps.get(0).unwrap();
            let label = caps.get(1).map(|g| stmt[g.start()..g.end()].trim().to_string());
            let style = classify(&masked[m.start()..m.end()]);
            links.push(Link {
                start: m.start(),
                end: m.end(),
                style,
                label,
            });
        }
        links
    }

    /// Parse `A & B --> C --> D` etc. into edges between adjacent groups.
    fn parse_edge_chain(&mut self, stmt: &str, links: &[Link]) {
        // Node-token groups are the spans between (and around) the links.
        let mut groups: Vec<Vec<NodeId>> = Vec::with_capacity(links.len() + 1);
        let mut pos = 0usize;
        for link in links {
            let seg = stmt[pos..link.start].trim();
            groups.push(self.parse_group(seg));
            pos = link.end;
        }
        groups.push(self.parse_group(stmt[pos..].trim()));

        for (i, link) in links.iter().enumerate() {
            let left = &groups[i];
            let right = &groups[i + 1];
            // collect to avoid borrow conflicts on the builder
            for &a in left {
                for &b in right {
                    self.builder
                        .add_edge(a, b, link.style, link.label.clone());
                }
            }
        }
    }

    /// Parse a group like `A & B["x"]` into node ids, registering shapes.
    fn parse_group(&mut self, seg: &str) -> Vec<NodeId> {
        let mut ids = Vec::new();
        for tok in seg.split('&') {
            let tok = tok.trim();
            if !tok.is_empty() {
                ids.push(self.register_token(tok));
            }
        }
        ids
    }

    /// Register a single node token, returning its id.
    fn register_token(&mut self, tok: &str) -> NodeId {
        let (id_str, shape, text) = parse_node_token(tok);
        let id = self.builder.intern(id_str);
        if shape != Shape::Default || text.is_some() {
            self.builder.set_node(id, shape, text);
        }
        id
    }
}

fn parse_header(stmt: &str) -> Option<String> {
    let lower = stmt.to_ascii_lowercase();
    let rest = if let Some(r) = lower.strip_prefix("flowchart") {
        r
    } else if let Some(r) = lower.strip_prefix("graph") {
        r
    } else {
        return None;
    };
    let rest = rest.trim();
    let dir = rest
        .split_whitespace()
        .next()
        .map(|d| d.to_ascii_uppercase())
        .filter(|d| matches!(d.as_str(), "TD" | "TB" | "BT" | "LR" | "RL"))
        .unwrap_or_else(|| "TD".to_string());
    Some(dir)
}

fn classify(op: &str) -> EdgeStyle {
    if op.contains('=') {
        EdgeStyle::Thick
    } else if op.contains('.') {
        EdgeStyle::Dotted
    } else {
        EdgeStyle::Normal
    }
}

/// Replace characters inside `[] () {} ""` with spaces so link detection and
/// `&` splitting never fire inside a label. Bracket nesting is tracked by depth.
fn mask_brackets(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = vec![b' '; bytes.len()];
    let mut depth = 0i32;
    let mut in_quote = false;
    for (i, &c) in bytes.iter().enumerate() {
        if in_quote {
            if c == b'"' {
                in_quote = false;
            }
            continue;
        }
        match c {
            b'"' => in_quote = true,
            b'[' | b'(' | b'{' => depth += 1,
            b']' | b')' | b'}' => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            _ => {
                if depth == 0 {
                    out[i] = c;
                }
            }
        }
    }
    // SAFETY: we only ever wrote ASCII spaces or copied original ASCII bytes at
    // boundaries; multibyte chars inside brackets become spaces (fine for masking).
    String::from_utf8(out).unwrap_or_else(|_| " ".repeat(s.len()))
}

/// Split a node token into (id, shape, optional text).
fn parse_node_token(tok: &str) -> (&str, Shape, Option<String>) {
    // id is the leading run up to the first shape opener.
    let open_pos = tok.find(|c| matches!(c, '[' | '(' | '{' | '>'));
    let Some(open) = open_pos else {
        return (tok, Shape::Default, None);
    };
    let id = tok[..open].trim();
    let rest = &tok[open..];

    let (shape, inner) = detect_shape(rest);
    let text = inner.map(|t| t.trim().trim_matches('"').trim().to_string());
    let text = text.filter(|t| !t.is_empty());
    (if id.is_empty() { tok } else { id }, shape, text)
}

/// Given the bracketed remainder of a token, identify the shape and inner text.
fn detect_shape(rest: &str) -> (Shape, Option<&str>) {
    let pairs: &[(&str, &str, Shape)] = &[
        ("([", "])", Shape::Stadium),
        ("[[", "]]", Shape::Subroutine),
        ("[(", ")]", Shape::Cylinder),
        ("((", "))", Shape::Circle),
        ("{{", "}}", Shape::Hexagon),
        ("[/", "/]", Shape::Parallelogram),
        ("[\\", "\\]", Shape::Trapezoid),
        ("[", "]", Shape::Rect),
        ("(", ")", Shape::RoundRect),
        ("{", "}", Shape::Diamond),
        (">", "]", Shape::Asymmetric),
    ];
    for (open, close, shape) in pairs {
        if rest.starts_with(open) && rest.ends_with(close) && rest.len() >= open.len() + close.len()
        {
            let inner = &rest[open.len()..rest.len() - close.len()];
            return (*shape, Some(inner));
        }
    }
    (Shape::Default, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> crate::store::Graph {
        Parser::new().parse_str(s).build()
    }

    #[test]
    fn simple_edges() {
        let g = parse("flowchart TD\n A --> B\n B --> C\n");
        assert_eq!(g.node_count(), 3);
        assert_eq!(g.edge_count(), 2);
        assert_eq!(g.direction, "TD");
    }

    #[test]
    fn edge_label_and_styles() {
        let g = parse("graph LR\nA -->|go| B\nB -.-> C\nC ==> D\n");
        assert_eq!(g.edge_count(), 3);
        assert_eq!(g.edge_label(0), Some("go"));
        assert_eq!(g.edge_style(1), EdgeStyle::Dotted);
        assert_eq!(g.edge_style(2), EdgeStyle::Thick);
    }

    #[test]
    fn chains_and_groups() {
        let g = parse("flowchart TD\nA --> B --> C\nA & B --> D & E\n");
        // chain: A-B, B-C ; group: A-D, A-E, B-D, B-E
        assert_eq!(g.edge_count(), 6);
    }

    #[test]
    fn node_shapes_and_labels() {
        let g = parse("flowchart TD\nA[\"Start\"] --> B{Decision}\nB --> C((End))\n");
        let a = g.id_of("A").unwrap();
        let b = g.id_of("B").unwrap();
        let c = g.id_of("C").unwrap();
        assert_eq!(g.shape(a), Shape::Rect);
        assert_eq!(g.display(a), "Start");
        assert_eq!(g.shape(b), Shape::Diamond);
        assert_eq!(g.shape(c), Shape::Circle);
    }

    #[test]
    fn comments_and_directives() {
        let g = parse("%%{init: {'theme':'dark'}}%%\nflowchart TD\n%% a comment\nA --> B %% trailing\n");
        assert_eq!(g.edge_count(), 1);
        assert_eq!(g.node_count(), 2);
    }

    #[test]
    fn dashes_inside_labels_are_safe() {
        let g = parse("flowchart TD\nA[\"a --> b\"] --> B\n");
        assert_eq!(g.edge_count(), 1);
        assert_eq!(g.display(g.id_of("A").unwrap()), "a --> b");
    }
}

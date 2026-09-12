//! `--map`: rank the definitions across a set of files by how much the code
//! references them, then print the top ones within a token budget.
//!
//! The graph is language-agnostic. Definitions come from each profile's
//! `def_kinds`, references from identifier-shaped node kinds, and PageRank
//! (biased toward the files named with `--focus`) orders them. There is no type
//! or scope resolution: the map is a relevance heuristic for orientation, not a
//! correctness oracle — the same trade every syntax-only repo map makes.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;
use tree_sitter::{Node, Tree};

use crate::discover;
use crate::lang::Lang;
use crate::render;

/// Tunables for a map. `budget_tokens` is an approximate ceiling on the printed
/// source (about four bytes per token).
#[derive(Debug, Clone)]
pub struct MapOptions {
    pub force: Option<Lang>,
    pub docs: bool,
    pub budget_tokens: usize,
}

impl Default for MapOptions {
    fn default() -> Self {
        MapOptions {
            force: None,
            docs: false,
            budget_tokens: 1024,
        }
    }
}

#[derive(Serialize, Debug)]
pub struct MapEntry {
    pub file: String,
    pub name: String,
    pub kind: String,
    pub line: usize,
    pub score: f64,
}

pub struct Map {
    /// The grouped, budgeted skeleton text.
    pub text: String,
    /// The selected definitions, in rank order.
    pub entries: Vec<MapEntry>,
}

struct Def {
    file: usize,
    name: String,
    kind: String,
    line: usize,
    depth: usize,
    start: usize,
    parent: Option<usize>,
    /// The declaration's own skeleton (including nested members).
    rendered: String,
    /// The first line of `rendered`, used as an ancestor breadcrumb.
    header: String,
    edges: Vec<(usize, f64)>,
    score: f64,
}

/// Build a ranked map over `paths`, biasing relevance toward `context` (empty
/// means a uniform, globally-important ranking).
pub fn run(paths: &[PathBuf], context: &[PathBuf], opts: &MapOptions) -> Result<Map> {
    let files = collect(paths, opts.force);
    let ctx: Vec<PathBuf> = context.iter().map(|p| canon(p)).collect();

    let mut defs: Vec<Def> = Vec::new();
    let mut by_name: HashMap<String, Vec<usize>> = HashMap::new();
    let mut sources: Vec<String> = Vec::with_capacity(files.len());
    let mut node_def: Vec<HashMap<usize, usize>> = Vec::with_capacity(files.len());
    let mut trees: Vec<Tree> = Vec::with_capacity(files.len());

    for (fi, path) in files.iter().enumerate() {
        let src =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let lang = opts.force.unwrap_or_else(|| Lang::from_path(path));
        let tree = render::parse(&src, lang)?;
        let mut index = Vec::new();
        let mut nmap = HashMap::new();
        walk_defs(
            tree.root_node(),
            &src,
            lang,
            fi,
            None,
            opts.docs,
            &mut defs,
            &mut by_name,
            &mut index,
            &mut nmap,
        );
        sources.push(src);
        node_def.push(nmap);
        trees.push(tree);
    }

    let mut weights: HashMap<(usize, usize), f64> = HashMap::new();
    for (fi, tree) in trees.iter().enumerate() {
        collect_refs(
            tree.root_node(),
            &sources[fi],
            &node_def[fi],
            &by_name,
            &defs,
            fi,
            &mut weights,
            None,
        );
    }
    for ((a, b), w) in weights {
        defs[a].edges.push((b, w));
    }

    let scores = pagerank(&defs, &files, &ctx);
    for (i, s) in scores.into_iter().enumerate() {
        defs[i].score = s;
    }

    let mut order: Vec<usize> = (0..defs.len()).collect();
    order.sort_by(|&a, &b| {
        defs[b]
            .score
            .partial_cmp(&defs[a].score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let limit = opts.budget_tokens.saturating_mul(4).max(1);
    let mut selected: Vec<usize> = Vec::new();
    let mut selected_set: HashSet<usize> = HashSet::new();
    let mut used = 0usize;
    for &i in &order {
        if defs[i].rendered.trim().is_empty() || has_ancestor(i, &defs, &selected_set) {
            continue;
        }
        let cost = defs[i].rendered.len() + defs[i].header.len() + 1;
        if !selected.is_empty() && used + cost > limit {
            continue;
        }
        used += cost;
        selected.push(i);
        selected_set.insert(i);
        if used >= limit {
            break;
        }
    }

    let entries: Vec<MapEntry> = selected
        .iter()
        .map(|&i| MapEntry {
            file: rel(&files[defs[i].file]),
            name: defs[i].name.clone(),
            kind: defs[i].kind.clone(),
            line: defs[i].line,
            score: defs[i].score,
        })
        .collect();

    let mut file_order: Vec<usize> = Vec::new();
    for &i in &selected {
        if !file_order.contains(&defs[i].file) {
            file_order.push(defs[i].file);
        }
    }
    let mut text = String::new();
    for &fi in &file_order {
        let mut rows: Vec<usize> = selected
            .iter()
            .copied()
            .filter(|&i| defs[i].file == fi)
            .collect();
        rows.sort_by_key(|&i| defs[i].start);
        text.push_str(&rel(&files[fi]));
        text.push_str(":\n");
        let mut shown: HashSet<usize> = HashSet::new();
        for i in rows {
            let mut chain = Vec::new();
            let mut p = defs[i].parent;
            while let Some(a) = p {
                chain.push(a);
                p = defs[a].parent;
            }
            for &a in chain.iter().rev() {
                if shown.insert(a) {
                    text.push_str(&"    ".repeat(defs[a].depth));
                    text.push_str(&defs[a].header);
                    text.push('\n');
                }
            }
            text.push_str(&defs[i].rendered);
        }
        text.push('\n');
    }

    Ok(Map { text, entries })
}

fn collect(paths: &[PathBuf], force: Option<Lang>) -> Vec<PathBuf> {
    discover::expand(paths)
        .into_iter()
        .filter(|p| {
            let lang = force.unwrap_or_else(|| Lang::from_path(p));
            lang.is_parseable() && !lang.is_doc()
        })
        .collect()
}

fn canon(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

fn rel(path: &Path) -> String {
    let s = path.display().to_string();
    s.strip_prefix("./").unwrap_or(&s).to_string()
}

/// A def whose ancestor is already selected is already printed inside that
/// ancestor's skeleton, so it is dropped.
fn has_ancestor(i: usize, defs: &[Def], selected: &HashSet<usize>) -> bool {
    let mut p = defs[i].parent;
    while let Some(a) = p {
        if selected.contains(&a) {
            return true;
        }
        p = defs[a].parent;
    }
    false
}

#[allow(clippy::too_many_arguments)]
fn walk_defs(
    node: Node,
    src: &str,
    lang: Lang,
    file: usize,
    parent: Option<usize>,
    docs: bool,
    defs: &mut Vec<Def>,
    by_name: &mut HashMap<String, Vec<usize>>,
    out: &mut Vec<usize>,
    node_def: &mut HashMap<usize, usize>,
) {
    if is_map_node(lang, node.kind(), parent) {
        if let Some(name) = def_name(node, src) {
            let depth = parent.map(|p| defs[p].depth + 1).unwrap_or(0);
            let idx = defs.len();
            let rendered = render::render_def(node, src, lang, depth, docs);
            let header = rendered
                .lines()
                .find(|l| !l.trim().is_empty())
                .map(|l| l.trim_start().to_string())
                .unwrap_or_default();
            defs.push(Def {
                file,
                name: name.to_string(),
                kind: node.kind().to_string(),
                line: node.start_position().row + 1,
                depth,
                start: node.start_byte(),
                parent,
                rendered,
                header,
                edges: Vec::new(),
                score: 0.0,
            });
            by_name.entry(name.to_string()).or_default().push(idx);
            out.push(idx);
            node_def.insert(node.id(), idx);
            let this = Some(idx);
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                walk_defs(
                    child, src, lang, file, this, docs, defs, by_name, out, node_def,
                );
            }
            return;
        }
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk_defs(
            child, src, lang, file, parent, docs, defs, by_name, out, node_def,
        );
    }
}

/// A node in the map graph: a type (class/interface/enum/…), or a top-level
/// callable (a free function). Methods nested in a type are not separate map
/// nodes — the type's skeleton already contains them, and their references
/// attribute to the type, so a class ranks by its whole API's usage instead of
/// each accessor competing on its own. Data fields are likewise not map nodes.
fn is_map_node(lang: Lang, kind: &str, parent: Option<usize>) -> bool {
    if !lang.def_kinds().contains(&kind) {
        return false;
    }
    if lang.type_kinds().contains(&kind) {
        return true;
    }
    is_callable(kind) && parent.is_none()
}

fn is_callable(kind: &str) -> bool {
    kind.contains("function") || kind.contains("method") || kind.contains("constructor")
}

#[allow(clippy::too_many_arguments)]
fn collect_refs(
    node: Node,
    src: &str,
    node_def: &HashMap<usize, usize>,
    by_name: &HashMap<String, Vec<usize>>,
    defs: &[Def],
    file: usize,
    weights: &mut HashMap<(usize, usize), f64>,
    current: Option<usize>,
) {
    let current = node_def.get(&node.id()).copied().or(current);
    if is_reference_kind(node.kind()) {
        if let Some(cur) = current {
            let text = &src[node.start_byte()..node.end_byte()];
            if let Some(targets) = by_name.get(text) {
                // Loose lexical scoping: a name defined in this file shadows a
                // same-named definition elsewhere. Without this, every `x`/`y`
                // method in every app merges into one hot node.
                let local: Vec<usize> = targets
                    .iter()
                    .copied()
                    .filter(|&t| defs[t].file == file)
                    .collect();
                let (chosen, w): (&[usize], f64) = if local.is_empty() {
                    (targets, 1.0)
                } else {
                    (&local, 0.15)
                };
                for &t in chosen {
                    if t != cur {
                        *weights.entry((cur, t)).or_insert(0.0) += w;
                    }
                }
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_refs(child, src, node_def, by_name, defs, file, weights, current);
    }
}

fn pagerank(defs: &[Def], files: &[PathBuf], ctx: &[PathBuf]) -> Vec<f64> {
    let n = defs.len();
    if n == 0 {
        return Vec::new();
    }
    let in_ctx = |fi: usize| -> bool {
        if ctx.is_empty() {
            return true;
        }
        let f = canon(&files[fi]);
        ctx.iter().any(|c| &f == c || f.starts_with(c))
    };
    let mut personal = vec![0.0f64; n];
    for i in 0..n {
        personal[i] = if in_ctx(defs[i].file) { 1.0 } else { 0.0 };
    }
    let sum: f64 = personal.iter().sum();
    if sum > 0.0 {
        for v in personal.iter_mut() {
            *v /= sum;
        }
    } else {
        for v in personal.iter_mut() {
            *v = 1.0 / n as f64;
        }
    }

    let mut incoming: Vec<Vec<(usize, f64)>> = vec![Vec::new(); n];
    for (a, d) in defs.iter().enumerate() {
        for &(b, w) in &d.edges {
            incoming[b].push((a, w));
        }
    }
    let out_sum: Vec<f64> = defs
        .iter()
        .map(|d| d.edges.iter().map(|(_, w)| w).sum())
        .collect();

    let damping = 0.85;
    let mut score = personal.clone();
    for _ in 0..40 {
        let dangling: f64 = (0..n)
            .filter(|&a| out_sum[a] == 0.0)
            .map(|a| score[a])
            .sum();
        let mut next = vec![0.0f64; n];
        for i in 0..n {
            let mut acc = 0.0;
            for &(a, w) in &incoming[i] {
                acc += score[a] * w / out_sum[a];
            }
            next[i] = (1.0 - damping) * personal[i] + damping * (acc + dangling * personal[i]);
        }
        score = next;
    }
    score
}

/// Identifier-shaped kinds. Nearly every grammar uses `identifier` or a
/// `*_identifier` variant; PHP names nodes `name` and the shells use
/// `command_name`/`variable_name`/`word`.
fn is_reference_kind(kind: &str) -> bool {
    kind.ends_with("identifier")
        || matches!(kind, "name" | "word" | "command_name" | "variable_name")
}

fn ident_text<'a>(node: Node, src: &'a str) -> Option<&'a str> {
    if is_reference_kind(node.kind()) {
        node.utf8_text(src.as_bytes()).ok()
    } else {
        None
    }
}

fn first_ident<'a>(node: Node, src: &'a str) -> Option<&'a str> {
    if let Some(t) = ident_text(node, src) {
        return Some(t);
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if let Some(t) = first_ident(child, src) {
            return Some(t);
        }
    }
    None
}

/// A descendant's `name` field (JS `variable_declarator`, Go `type_spec`, …).
fn name_field<'a>(node: Node, src: &'a str, depth: usize) -> Option<&'a str> {
    if depth == 0 {
        return None;
    }
    if let Some(n) = node.child_by_field_name("name") {
        if let Some(t) = ident_text(n, src).or_else(|| first_ident(n, src)) {
            return Some(t);
        }
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if let Some(t) = name_field(child, src, depth - 1) {
            return Some(t);
        }
    }
    None
}

/// The name a declaration binds. Tries the `name` field, then the declarator
/// (C-family), then a descendant `name` field, then the first identifier.
fn def_name<'a>(node: Node, src: &'a str) -> Option<&'a str> {
    if let Some(n) = node.child_by_field_name("name") {
        if let Some(t) = ident_text(n, src).or_else(|| first_ident(n, src)) {
            return Some(t);
        }
    }
    for field in ["declarator", "field"] {
        if let Some(n) = node.child_by_field_name(field) {
            if let Some(t) = ident_text(n, src).or_else(|| first_ident(n, src)) {
                return Some(t);
            }
        }
    }
    name_field(node, src, 3).or_else(|| first_ident(node, src))
}

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
    /// `--referrers` also matches comments and string literals.
    pub text: bool,
    /// `--referrers` folds ASCII case.
    pub ignore_case: bool,
}

impl Default for MapOptions {
    fn default() -> Self {
        MapOptions {
            force: None,
            docs: false,
            budget_tokens: 4096,
            text: false,
            ignore_case: false,
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
    /// How many definitions the budget printed; always `entries.len()`.
    pub selected: usize,
    /// Definitions the budget dropped. A def printed inside a selected
    /// ancestor's body is not dropped, so it is not counted here.
    pub omitted: usize,
}

/// One place a name occurs, with the definition that encloses it.
#[derive(Serialize, Debug)]
pub struct Occurrence {
    pub file: String,
    pub line: usize,
    /// `Type.method`, `Type`, or empty at file scope.
    pub enclosing: String,
    /// True when this occurrence is the declaration itself rather than a use.
    pub definition: bool,
}

/// Every occurrence of a name across the scanned files. Unlike the map graph,
/// which resolves references only to whole types and top-level functions, this
/// matches any identifier, so it finds method, field and constant uses too.
/// The resolution is lexical, exactly like `grep`, but each hit names the
/// definition that contains it.
#[derive(Serialize, Debug)]
pub struct ReferrerReport {
    pub symbol: String,
    pub occurrences: usize,
    pub files: usize,
    /// Source files searched, so a zero can be told from an empty scan set.
    pub scanned: usize,
    /// Whether comments and string literals were included in the scan.
    pub text: bool,
    pub entries: Vec<Occurrence>,
}

impl ReferrerReport {
    pub fn text(&self) -> String {
        let mut out = String::new();
        let mut last: Option<&str> = None;
        for e in &self.entries {
            if last != Some(e.file.as_str()) {
                out.push_str(&e.file);
                out.push_str(":\n");
                last = Some(&e.file);
            }
            let enc = if e.enclosing.is_empty() {
                "(file scope)"
            } else {
                &e.enclosing
            };
            let tail = if e.definition { "  [definition]" } else { "" };
            out.push_str(&format!("  {:<6} {}{}\n", e.line, enc, tail));
        }
        let note = if self.text {
            ""
        } else {
            "; code identifiers only, pass --text to include comments and string literals"
        };
        out.push_str(&format!(
            "\n{} occurrence(s) of {:?} in {} file(s) (scanned {}{note})\n",
            self.occurrences, self.symbol, self.files, self.scanned
        ));
        out
    }
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

    let selected_count = selected.len();
    let omitted = (0..defs.len())
        .filter(|&i| {
            !defs[i].rendered.trim().is_empty()
                && !selected_set.contains(&i)
                && !has_ancestor(i, &defs, &selected_set)
        })
        .count();

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
    text.push_str(&format!(
        "# map: {} of {} definitions, budget {} tokens\n",
        selected_count,
        selected_count + omitted,
        opts.budget_tokens
    ));
    for &fi in &file_order {
        text.push_str(&rel(&files[fi]));
        text.push_str(":\n");
        let lang = opts.force.unwrap_or_else(|| Lang::from_path(&files[fi]));
        let close = lang.braces().1;

        // The printed tree is the selected defs plus whatever type ancestors
        // they need to stay balanced. A selected type already renders its own
        // body (and closing brace); an ancestor that is only context prints its
        // header, its selected descendants, then its closing brace.
        let here: Vec<usize> = selected
            .iter()
            .copied()
            .filter(|&i| defs[i].file == fi)
            .collect();
        let selected_here: HashSet<usize> = here.iter().copied().collect();
        let mut nodes: HashSet<usize> = selected_here.clone();
        for &i in &here {
            let mut p = defs[i].parent;
            while let Some(a) = p {
                if lang.type_kinds().contains(&defs[a].kind.as_str()) {
                    nodes.insert(a);
                }
                p = defs[a].parent;
            }
        }
        let mut children: HashMap<usize, Vec<usize>> = HashMap::new();
        let mut roots: Vec<usize> = Vec::new();
        for &n in &nodes {
            let mut p = defs[n].parent;
            let mut tree_parent = None;
            while let Some(a) = p {
                if nodes.contains(&a) {
                    tree_parent = Some(a);
                    break;
                }
                p = defs[a].parent;
            }
            match tree_parent {
                Some(a) => children.entry(a).or_default().push(n),
                None => roots.push(n),
            }
        }
        roots.sort_by_key(|&i| defs[i].start);
        for kids in children.values_mut() {
            kids.sort_by_key(|&i| defs[i].start);
        }
        for r in roots {
            emit_tree(r, &defs, &selected_here, close, &children, &mut text);
        }
        text.push('\n');
    }
    if omitted > 0 {
        text.push_str(&format!(
            "# omitted: {} - raise --map-budget to include them\n",
            omitted
        ));
    }

    Ok(Map {
        text,
        entries,
        selected: selected_count,
        omitted,
    })
}

/// Find every occurrence of `symbol` under `paths`, each labelled with its
/// enclosing definition. This is the "who uses this" half of the map: the
/// reference graph answers what a file depends on, this answers where a name is
/// touched, ahead of a rename or a signature change.
///
/// The match is lexical — the same thing `grep` does — so a same-named local is
/// reported too. What it adds over `grep` is the enclosing `Type.method`, so the
/// result is the list of scopes to change rather than a pile of line numbers.
pub fn referrers(paths: &[PathBuf], symbol: &str, opts: &MapOptions) -> Result<ReferrerReport> {
    let files = collect(paths, opts.force);
    let scanned = files.len();
    let mut entries: Vec<Occurrence> = Vec::new();
    for path in &files {
        let src =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let lang = opts.force.unwrap_or_else(|| Lang::from_path(path));
        let tree = render::parse(&src, lang)?;
        let mut hits: Vec<(usize, String, bool)> = Vec::new();
        scan_hits(
            tree.root_node(),
            &src,
            lang,
            symbol,
            opts.text,
            opts.ignore_case,
            None,
            None,
            &mut hits,
        );
        let file = rel(path);
        for (line, enclosing, definition) in hits {
            entries.push(Occurrence {
                file: file.clone(),
                line,
                enclosing,
                definition,
            });
        }
    }
    entries.sort_by(|a, b| {
        a.file
            .cmp(&b.file)
            .then(a.line.cmp(&b.line))
            .then(a.enclosing.cmp(&b.enclosing))
    });
    let files = {
        let mut seen: Vec<&str> = entries.iter().map(|e| e.file.as_str()).collect();
        seen.dedup();
        seen.len()
    };
    Ok(ReferrerReport {
        symbol: symbol.to_string(),
        occurrences: entries.len(),
        files,
        scanned,
        text: opts.text,
        entries,
    })
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

/// Print one node of the pruned map tree. A selected node prints its own
/// skeleton; a context-only ancestor prints its header, its selected
/// descendants, and its closing brace.
fn emit_tree(
    n: usize,
    defs: &[Def],
    selected: &HashSet<usize>,
    close: &str,
    children: &HashMap<usize, Vec<usize>>,
    out: &mut String,
) {
    if selected.contains(&n) {
        out.push_str(&defs[n].rendered);
        return;
    }
    let pad = "    ".repeat(defs[n].depth);
    out.push_str(&pad);
    out.push_str(&defs[n].header);
    out.push('\n');
    if let Some(kids) = children.get(&n) {
        for &k in kids {
            emit_tree(k, defs, selected, close, children, out);
        }
    }
    if !close.is_empty() {
        out.push_str(&pad);
        out.push_str(close);
        out.push('\n');
    }
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

/// Walk `node` recording every `symbol` occurrence, labelled with the nearest
/// enclosing definition (`Type.method`). `enc` carries that label down, and
/// `def_span` carries the byte range of the enclosing declaration's name node
/// so the declaration can be distinguished from a use of the same name. With
/// `text`, comments and string literals are matched too (as whole words), which
/// is what a rename needs to catch a label like `"Found"` and its javadoc; the
/// default scan is code identifiers only.
#[allow(clippy::too_many_arguments)]
fn scan_hits(
    node: Node,
    src: &str,
    lang: Lang,
    symbol: &str,
    text: bool,
    ignore_case: bool,
    enc: Option<&str>,
    def_span: Option<(usize, usize)>,
    hits: &mut Vec<(usize, String, bool)>,
) {
    let name_node = if lang.def_kinds().contains(&node.kind()) {
        def_name_node(node, src)
    } else {
        None
    };
    let label = name_node.map(|n| {
        let n = &src[n.start_byte()..n.end_byte()];
        match enc {
            Some(e) => format!("{e}.{n}"),
            None => n.to_string(),
        }
    });
    let enc = label.as_deref().or(enc);
    let def_span = name_node
        .map(|n| (n.start_byte(), n.end_byte()))
        .or(def_span);
    let kind = node.kind();
    if is_reference_kind(kind) {
        let found = &src[node.start_byte()..node.end_byte()];
        let matches = if ignore_case {
            found.eq_ignore_ascii_case(symbol)
        } else {
            found == symbol
        };
        if matches {
            let definition = def_span == Some((node.start_byte(), node.end_byte()));
            hits.push((
                node.start_position().row + 1,
                enc.unwrap_or("").to_string(),
                definition,
            ));
            return;
        }
    } else if text && (lang.comment_kinds().contains(&kind) || is_string_kind(kind)) {
        let raw = &src[node.start_byte()..node.end_byte()];
        if let Some(off) = find_text(raw, symbol, ignore_case) {
            let line = node.start_position().row + 1 + raw[..off].matches('\n').count();
            hits.push((line, enc.unwrap_or("").to_string(), false));
        }
        // A comment or string literal contains no nested declarations, and its
        // child `string_fragment` would otherwise double-count the same hit.
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        scan_hits(
            child,
            src,
            lang,
            symbol,
            text,
            ignore_case,
            enc,
            def_span,
            hits,
        );
    }
}

/// Whether a node kind is a string/char literal whose text `--text` should scan.
fn is_string_kind(kind: &str) -> bool {
    kind.contains("string") || kind.contains("char") || kind == "quoted_attribute_value"
}

/// For each 0-based source line, the innermost enclosing definition's label
/// (`Type.method`, `Type`, or empty at file scope). A `--grep --body` hit uses
/// this to say which scope it belongs to, the way `--referrers` does.
pub(crate) fn enclosing_by_line(root: Node, src: &str, lang: Lang) -> Vec<String> {
    let mut spans: Vec<(usize, usize, String)> = Vec::new();
    collect_def_spans(root, src, lang, None, &mut spans);
    let lines = src.lines().count();
    let mut out = vec![String::new(); lines];
    // Widest-first, so a nested definition overwrites the ancestors that
    // contain it and the innermost scope wins.
    spans.sort_by_key(|(start, end, _)| std::cmp::Reverse(end.saturating_sub(*start)));
    for (start, end, label) in spans {
        let stop = end.min(lines.saturating_sub(1));
        for slot in out.iter_mut().take(stop + 1).skip(start) {
            *slot = label.clone();
        }
    }
    out
}

fn collect_def_spans(
    node: Node,
    src: &str,
    lang: Lang,
    enc: Option<&str>,
    out: &mut Vec<(usize, usize, String)>,
) {
    let name_node = if lang.def_kinds().contains(&node.kind()) {
        def_name_node(node, src)
    } else {
        None
    };
    let label = name_node.map(|n| {
        let n = &src[n.start_byte()..n.end_byte()];
        match enc {
            Some(e) => format!("{e}.{n}"),
            None => n.to_string(),
        }
    });
    if let Some(label) = &label {
        out.push((
            node.start_position().row,
            node.end_position().row,
            label.clone(),
        ));
    }
    let enc = label.as_deref().or(enc);
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_def_spans(child, src, lang, enc, out);
    }
}

/// The byte offset of `symbol` in `text` as a whole word, optionally ASCII
/// case-insensitively. Whole-word matching keeps `--text` a rename tool rather
/// than a substring search: `Found` hits `"Found"` and `Found/Best`, not
/// `Foundations` or `Found by looking`.
fn find_text(text: &str, symbol: &str, ignore_case: bool) -> Option<usize> {
    if symbol.is_empty() {
        return None;
    }
    let hay = if ignore_case {
        text.to_ascii_lowercase()
    } else {
        text.to_string()
    };
    let needle = if ignore_case {
        symbol.to_ascii_lowercase()
    } else {
        symbol.to_string()
    };
    let bytes = hay.as_bytes();
    for (start, _) in hay.match_indices(&needle) {
        let end = start + needle.len();
        let before_ok = start == 0 || !is_word_byte(bytes[start - 1]);
        let after_ok = end == bytes.len() || !is_word_byte(bytes[end]);
        if before_ok && after_ok {
            return Some(start);
        }
    }
    None
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
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

fn ident_node<'a>(node: Node<'a>, src: &'a str) -> Option<Node<'a>> {
    if is_reference_kind(node.kind()) && node.utf8_text(src.as_bytes()).is_ok() {
        Some(node)
    } else {
        None
    }
}

fn first_ident_node<'a>(node: Node<'a>, src: &'a str) -> Option<Node<'a>> {
    if let Some(n) = ident_node(node, src) {
        return Some(n);
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if let Some(n) = first_ident_node(child, src) {
            return Some(n);
        }
    }
    None
}

/// A descendant's `name` field (JS `variable_declarator`, Go `type_spec`, …).
fn name_field_node<'a>(node: Node<'a>, src: &'a str, depth: usize) -> Option<Node<'a>> {
    if depth == 0 {
        return None;
    }
    if let Some(n) = node.child_by_field_name("name") {
        if let Some(id) = ident_node(n, src).or_else(|| first_ident_node(n, src)) {
            return Some(id);
        }
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if let Some(id) = name_field_node(child, src, depth - 1) {
            return Some(id);
        }
    }
    None
}

/// The node a declaration's name is written at. Tries the `name` field, then
/// the declarator (C-family) and `type` (a Rust `impl` block), then a
/// descendant `name` field, then the first identifier. Returning the node, not
/// just its text, lets a caller tell the declaration site from a use of the
/// same name.
fn def_name_node<'a>(node: Node<'a>, src: &'a str) -> Option<Node<'a>> {
    if let Some(n) = node.child_by_field_name("name") {
        if let Some(id) = ident_node(n, src).or_else(|| first_ident_node(n, src)) {
            return Some(id);
        }
    }
    for field in ["declarator", "field", "type"] {
        if let Some(n) = node.child_by_field_name(field) {
            if let Some(id) = ident_node(n, src).or_else(|| first_ident_node(n, src)) {
                return Some(id);
            }
        }
    }
    name_field_node(node, src, 3).or_else(|| first_ident_node(node, src))
}

/// The name a declaration binds.
fn def_name<'a>(node: Node, src: &'a str) -> Option<&'a str> {
    let n = def_name_node(node, src)?;
    n.utf8_text(src.as_bytes()).ok()
}

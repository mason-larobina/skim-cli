//! Tree-sitter parsing helpers and the two renderers: source → signature
//! skeleton, markdown → its own heading/quote skeleton.

use std::collections::BTreeMap;

use anyhow::{anyhow, Result};
use tree_sitter::{Node, Parser, Tree};

use crate::lang::Lang;

pub fn parse(src: &str, lang: Lang) -> Result<Tree> {
    let language = lang
        .language()
        .ok_or_else(|| anyhow!("no grammar for {}", lang.name()))?;
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .map_err(|e| anyhow!("failed to load {} grammar: {e}", lang.name()))?;
    parser
        .parse(src, None)
        .ok_or_else(|| anyhow!("tree-sitter returned no tree for {} input", lang.name()))
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn header_text<'a>(src: &'a str, node: Node, body: Option<Node>, lang: Lang) -> &'a str {
    let mut end = body
        .map(|b| b.start_byte())
        .unwrap_or_else(|| node.end_byte());
    // `type A struct {` (Go): the grammar nests the keyword inside the body
    // node, so extend the header to the first `{` inside it.
    if let Some(b) = body {
        if lang.braces().0.contains('{') {
            let text = &src[b.start_byte()..b.end_byte()];
            if let Some(i) = text.find('{') {
                end = b.start_byte() + i;
            }
        }
    }
    &src[node.start_byte()..end]
}

/// The elidable body of a declaration: the `body` field when the grammar
/// exposes one, otherwise (for type declarations and scan-friendly grammars) the
/// shallowest descendant whose kind is a body kind. Shallowest keeps members
/// inside their own declaration instead of hoisting a nested body up.
fn find_body(node: Node, lang: Lang) -> Option<Node> {
    if let Some(body) = node.child_by_field_name("body") {
        return Some(body);
    }
    if !(lang.type_kinds().contains(&node.kind()) || lang.scan_body()) {
        return None;
    }
    let kinds = lang.body_kinds();
    let mut frontier = vec![node];
    for _ in 0..5 {
        let mut next = Vec::new();
        let mut found: Option<Node> = None;
        for n in frontier {
            let mut cursor = n.walk();
            for child in n.named_children(&mut cursor) {
                if kinds.contains(&child.kind())
                    && found
                        .map(|f| child.start_byte() < f.start_byte())
                        .unwrap_or(true)
                {
                    found = Some(child);
                }
                next.push(child);
            }
        }
        if found.is_some() {
            return found;
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }
    None
}

/// Width of the `--lines` prefix: a five-digit line number and two spaces.
const GUTTER: usize = 7;

pub(crate) fn emit(
    node: Node,
    src: &str,
    lang: Lang,
    indent: usize,
    docs: bool,
    lines: bool,
    out: &mut String,
) {
    let kind = node.kind();
    if lang.def_kinds().contains(&kind) {
        if docs {
            out.push_str(&leading_comments(src, node, lang, indent, lines));
        }
        let body = find_body(node, lang);
        let pad = "    ".repeat(indent);
        let num = if lines {
            format!("{:>w$}  ", node.start_position().row + 1, w = GUTTER - 2)
        } else {
            String::new()
        };
        if lang.type_kinds().contains(&kind) && body.is_some() {
            let head = collapse(header_text(src, node, body, lang));
            let (open, close) = lang.braces();
            out.push_str(&num);
            out.push_str(&pad);
            out.push_str(&head);
            out.push_str(open);
            out.push('\n');
            if let Some(body) = body {
                let mut cursor = body.walk();
                for child in body.named_children(&mut cursor) {
                    emit(child, src, lang, indent + 1, docs, lines, out);
                }
            }
            if !close.is_empty() {
                if lines {
                    out.push_str(&" ".repeat(GUTTER));
                }
                out.push_str(&pad);
                out.push_str(close);
                out.push('\n');
            }
        } else if body.is_some() {
            let head = collapse(header_text(src, node, body, lang));
            out.push_str(&num);
            out.push_str(&pad);
            out.push_str(&head);
            out.push_str(lang.elide());
            out.push('\n');
        } else {
            let text = collapse(header_text(src, node, None, lang));
            if !text.is_empty() {
                out.push_str(&num);
                out.push_str(&pad);
                out.push_str(&text);
                out.push('\n');
            }
        }
        return;
    }
    // Not a declaration: descend through wrappers (program, class_body, …)
    // without changing indentation. Executable bodies never reach here because
    // their owning declaration returned above.
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        emit(child, src, lang, indent, docs, lines, out);
    }
}

/// Render a source file as a declaration skeleton. Returns an empty string for
/// an unparseable/plain language.
pub fn render_source(src: &str, lang: Lang) -> Result<String> {
    render_source_opts(src, lang, false, false)
}

/// Like [`render_source`], but when `docs` is set each declaration keeps the
/// comment block immediately above it (its doc comment).
pub fn render_source_with(src: &str, lang: Lang, docs: bool) -> Result<String> {
    render_source_opts(src, lang, docs, false)
}

/// Like [`render_source_with`], but when `lines` is set each declaration line is
/// prefixed with its 1-based source line, so a reader can jump straight to the
/// body `read` has to open.
pub fn render_source_opts(src: &str, lang: Lang, docs: bool, lines: bool) -> Result<String> {
    if !lang.is_parseable() || lang.is_doc() {
        return Ok(String::new());
    }
    let tree = parse(src, lang)?;
    let mut out = String::new();
    emit(tree.root_node(), src, lang, 0, docs, lines, &mut out);
    Ok(out)
}

/// Render a single declaration node (and any nested members) at `indent`.
pub(crate) fn render_def(node: Node, src: &str, lang: Lang, indent: usize, docs: bool) -> String {
    let mut out = String::new();
    emit(node, src, lang, indent, docs, false, &mut out);
    out
}

/// The contiguous run of comment siblings immediately above `node`, re-indented
/// to `indent`. A comment that shares a line with the statement before it is a
/// trailing comment, not a doc comment, and is ignored. Returns an empty string
/// when there is no adjacent leading comment. With `lines`, each comment line
/// gets a blank gutter as wide as a declaration's line number, so the comment
/// stays aligned with the declaration it documents.
fn leading_comments(src: &str, node: Node, lang: Lang, indent: usize, lines: bool) -> String {
    let kinds = lang.comment_kinds();
    if kinds.is_empty() {
        return String::new();
    }
    let mut block: Vec<Node> = Vec::new();
    let mut cursor = node;
    while let Some(prev) = cursor.prev_sibling() {
        if !kinds.contains(&prev.kind()) {
            break;
        }
        // A comment on the same line as the code before it is a trailing
        // comment of that code, not a doc comment for `node`.
        if prev
            .prev_sibling()
            .is_some_and(|b| b.end_position().row >= prev.start_position().row)
        {
            break;
        }
        // A blank line between the comment and what follows breaks the run. A
        // line comment's span may already include its trailing newline, so test
        // the whitespace gap rather than the row numbers.
        if src[prev.end_byte()..cursor.start_byte()]
            .matches('\n')
            .count()
            > 1
        {
            break;
        }
        block.push(prev);
        cursor = prev;
    }
    if block.is_empty() {
        return String::new();
    }
    let gutter = if lines {
        " ".repeat(GUTTER)
    } else {
        String::new()
    };
    let pad = gutter + &"    ".repeat(indent);
    let mut out = String::new();
    for c in block.into_iter().rev() {
        let base = c.start_position().column;
        // A line comment's span may include its trailing newline; drop it so the
        // emitted block has no spurious blank line.
        let raw = &src[c.start_byte()..c.end_byte()];
        let text = raw.strip_suffix('\n').unwrap_or(raw);
        for (i, line) in text.split('\n').enumerate() {
            let line = line.trim_end();
            let line = if i == 0 {
                line
            } else {
                strip_indent(line, base)
            };
            if !line.is_empty() {
                out.push_str(&pad);
            }
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// Strip up to `cols` leading whitespace characters.
fn strip_indent(line: &str, cols: usize) -> &str {
    for (n, (i, ch)) in line.char_indices().enumerate() {
        if n >= cols || !ch.is_whitespace() {
            return &line[i..];
        }
    }
    ""
}

/// Skeleton of a markdown document: the heading lines and every block-quote
/// line, keyed by 0-based line number. This is exactly the "index" that
/// `todo.py` computes with a line regex, but computed from the parse.
pub fn markdown_skeleton(src: &str, lang: Lang) -> Result<BTreeMap<usize, String>> {
    let tree = parse(src, lang)?;
    let lines: Vec<&str> = src.lines().collect();
    let mut skeleton = BTreeMap::new();
    collect_markdown(tree.root_node(), &lines, &mut skeleton);
    Ok(skeleton)
}

fn collect_markdown(node: Node, lines: &[&str], out: &mut BTreeMap<usize, String>) {
    match node.kind() {
        "atx_heading" | "setext_heading" => {
            let row = node.start_position().row;
            if let Some(line) = lines.get(row) {
                out.entry(row)
                    .or_insert_with(|| line.trim_end().to_string());
            }
            return;
        }
        "block_quote" => {
            for row in node.start_position().row..=node.end_position().row {
                if let Some(line) = lines.get(row) {
                    // A block_quote node can absorb the blank line after it and
                    // lazy continuation lines; only literal `>` lines are
                    // skeleton, matching the `todo.py` contract.
                    if line.trim_start().starts_with('>') {
                        out.entry(row)
                            .or_insert_with(|| line.trim_end().to_string());
                    }
                }
            }
            return;
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_markdown(child, lines, out);
    }
}

pub fn render_markdown(src: &str, lang: Lang) -> Result<String> {
    render_markdown_opts(src, lang, false)
}

/// Like [`render_markdown`], but when `lines` is set each skeleton line is
/// prefixed with its 1-based source line.
pub fn render_markdown_opts(src: &str, lang: Lang, lines: bool) -> Result<String> {
    let skeleton = markdown_skeleton(src, lang)?;
    let mut out = String::new();
    for (row, line) in &skeleton {
        if lines {
            out.push_str(&format!("{:>5}  ", row + 1));
        }
        out.push_str(line);
        out.push('\n');
    }
    Ok(out)
}

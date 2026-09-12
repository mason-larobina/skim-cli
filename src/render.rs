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

fn header_text<'a>(src: &'a str, node: Node, body: Option<Node>) -> &'a str {
    let end = body
        .map(|b| b.start_byte())
        .unwrap_or_else(|| node.end_byte());
    &src[node.start_byte()..end]
}

fn emit(node: Node, src: &str, lang: Lang, indent: usize, out: &mut String) {
    let kind = node.kind();
    if lang.def_kinds().contains(&kind) {
        let body = node.child_by_field_name("body");
        let pad = "    ".repeat(indent);
        if lang.type_kinds().contains(&kind) && body.is_some() {
            let head = collapse(header_text(src, node, body));
            let (open, close) = lang.braces();
            out.push_str(&pad);
            out.push_str(&head);
            out.push_str(open);
            out.push('\n');
            if let Some(body) = body {
                for child in body.named_children(&mut node.walk()) {
                    emit(child, src, lang, indent + 1, out);
                }
            }
            out.push_str(&pad);
            out.push_str(close);
            out.push('\n');
        } else if body.is_some() {
            let head = collapse(header_text(src, node, body));
            out.push_str(&pad);
            out.push_str(&head);
            out.push_str(" { … }\n");
        } else {
            let text = collapse(header_text(src, node, None));
            if !text.is_empty() {
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
        emit(child, src, lang, indent, out);
    }
}

/// Render a source file as a declaration skeleton. Returns an empty string for
/// an unparseable/plain language.
pub fn render_source(src: &str, lang: Lang) -> Result<String> {
    if !lang.is_parseable() || lang.is_doc() {
        return Ok(String::new());
    }
    let tree = parse(src, lang)?;
    let mut out = String::new();
    emit(tree.root_node(), src, lang, 0, &mut out);
    Ok(out)
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
    let skeleton = markdown_skeleton(src, lang)?;
    let mut out = String::new();
    for line in skeleton.values() {
        out.push_str(line);
        out.push('\n');
    }
    Ok(out)
}

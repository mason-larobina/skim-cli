//! Syntax-aware line accounting: how much of a file is code, comment, or blank,
//! and how many declarations it exposes.

use anyhow::Result;
use tree_sitter::{Node, Tree};

use crate::lang::Lang;
use crate::render;

#[derive(Clone, Copy, Debug, Default)]
pub struct FileMetrics {
    pub total: usize,
    pub nonblank: usize,
    pub comment: usize,
    pub code: usize,
    pub decls: usize,
}

impl FileMetrics {
    /// Share of non-blank lines that are comment-only, 0.0 when the file is
    /// empty.
    pub fn comment_ratio(&self) -> f64 {
        if self.nonblank == 0 {
            0.0
        } else {
            self.comment as f64 / self.nonblank as f64
        }
    }

    /// The value a sqrt band is measured against.
    pub fn metric(&self, metric: Metric) -> usize {
        match metric {
            Metric::Nonblank => self.nonblank,
            Metric::Code => self.code,
            Metric::Decls => self.decls,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Metric {
    #[default]
    Nonblank,
    Code,
    Decls,
}

impl Metric {
    pub fn from_name(name: &str) -> Option<Metric> {
        match name.to_ascii_lowercase().as_str() {
            "nonblank" | "lines" => Some(Metric::Nonblank),
            "code" => Some(Metric::Code),
            "decls" | "declarations" => Some(Metric::Decls),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Metric::Nonblank => "nonblank",
            Metric::Code => "code",
            Metric::Decls => "decls",
        }
    }
}

/// Byte ranges of every comment node, sorted by start.
fn comment_ranges(tree: &Tree, lang: Lang) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    collect_comments(tree.root_node(), lang, &mut ranges);
    ranges.sort_unstable();
    ranges
}

fn collect_comments(node: Node, lang: Lang, out: &mut Vec<(usize, usize)>) {
    if lang.comment_kinds().contains(&node.kind()) {
        out.push((node.start_byte(), node.end_byte()));
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_comments(child, lang, out);
    }
}

fn covered(pos: usize, ranges: &[(usize, usize)]) -> bool {
    let mut lo = 0usize;
    let mut hi = ranges.len();
    while lo < hi {
        let mid = (lo + hi) / 2;
        let (start, end) = ranges[mid];
        if pos < start {
            hi = mid;
        } else if pos >= end {
            lo = mid + 1;
        } else {
            return true;
        }
    }
    false
}

fn count_decls(node: Node, lang: Lang) -> usize {
    let mut n = if lang.def_kinds().contains(&node.kind()) {
        1
    } else {
        0
    };
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        n += count_decls(child, lang);
    }
    n
}

/// Measure a parseable source file. Comments are detected from the parse, so a
/// `//` inside a string literal is code, and a block comment is one comment
/// region rather than one per line.
pub fn measure_source(src: &str, lang: Lang) -> Result<FileMetrics> {
    if !lang.is_parseable() || lang.is_doc() {
        return Ok(measure_lines_only(src));
    }
    let tree = render::parse(src, lang)?;
    let ranges = comment_ranges(&tree, lang);
    let mut m = FileMetrics {
        decls: count_decls(tree.root_node(), lang),
        ..Default::default()
    };
    let mut offset = 0usize;
    for line in src.lines() {
        m.total += 1;
        let start = offset;
        offset += line.len() + 1; // account for '\n'
        if line.trim().is_empty() {
            continue;
        }
        m.nonblank += 1;
        let mut comment_only = true;
        for (i, ch) in line.char_indices() {
            if ch.is_whitespace() {
                continue;
            }
            if !covered(start + i, &ranges) {
                comment_only = false;
                break;
            }
        }
        if comment_only {
            m.comment += 1;
        } else {
            m.code += 1;
        }
    }
    Ok(m)
}

fn measure_lines_only(src: &str) -> FileMetrics {
    let mut m = FileMetrics::default();
    for line in src.lines() {
        m.total += 1;
        if !line.trim().is_empty() {
            m.nonblank += 1;
            m.code += 1;
        }
    }
    m
}

/// A markdown document is measured the same way (non-blank lines) with no
/// comment classification, plus its skeleton line count.
pub fn measure_doc(src: &str, lang: Lang) -> Result<(FileMetrics, usize)> {
    let m = measure_lines_only(src);
    let skeleton = render::markdown_skeleton(src, lang)?.len();
    Ok((m, skeleton))
}

/// Non-blank line count only, for any file.
pub fn nonblank(src: &str) -> usize {
    src.lines().filter(|l| !l.trim().is_empty()).count()
}

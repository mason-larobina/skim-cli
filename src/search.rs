//! `--grep`: literal search over the rendered skeletons. Unlike `--referrers`
//! (identifiers labelled with their enclosing scope) this matches the *index* —
//! markdown headings and quote lines, code declaration signatures (and doc
//! comments with `docs`) — so a hit names a concept or a definition rather than
//! an arbitrary line of prose. It is the scouting pass for finding what a task
//! touches before reading anything: which design docs carry a concept, which
//! module declares a tool, where a rule is written down.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::discover;
use crate::lang::Lang;
use crate::render;

/// One matching skeleton line.
#[derive(Serialize, Debug)]
pub struct Match {
    pub file: String,
    /// The 1-based source line the skeleton line maps to.
    pub line: usize,
    /// The skeleton line itself (heading, quote, or declaration signature).
    pub skeleton: String,
}

/// Every skeleton line under `paths` that contains `pattern`.
pub fn grep(
    paths: &[PathBuf],
    pattern: &str,
    force: Option<Lang>,
    docs: bool,
    ignore_case: bool,
) -> Result<Vec<Match>> {
    let files: Vec<PathBuf> = discover::expand(paths)
        .into_iter()
        .filter(|p| force.is_some() || Lang::from_path(p).is_parseable())
        .collect();
    let needle = if ignore_case {
        pattern.to_lowercase()
    } else {
        pattern.to_string()
    };
    let mut out = Vec::new();
    for p in &files {
        let text =
            std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?;
        let lang = force.unwrap_or_else(|| Lang::from_path(p));
        // Always number the skeleton so a hit opens straight in `read`; split
        // the prefix back out so the JSON carries the number too.
        let rendered = if lang.is_doc() {
            render::render_markdown_opts(&text, lang, true)?
        } else {
            render::render_source_opts(&text, lang, docs, true)?
        };
        for raw in rendered.lines() {
            let hay = if ignore_case {
                raw.to_lowercase()
            } else {
                raw.to_string()
            };
            if hay.contains(&needle) {
                let (line, skeleton) = split_line(raw);
                out.push(Match {
                    file: p.display().to_string(),
                    line,
                    skeleton,
                });
            }
        }
    }
    Ok(out)
}

/// Split the `   13  text` prefix `--lines` writes back into `(13, "text")`.
/// A line with no numeric prefix (a doc-comment continuation) reports line 0.
fn split_line(raw: &str) -> (usize, String) {
    let trimmed = raw.trim_start();
    if let Some(rest) = trimmed.split_once("  ") {
        let (num, text) = rest;
        if !num.is_empty() && num.chars().all(|c| c.is_ascii_digit()) {
            return (num.parse().unwrap_or(0), text.to_string());
        }
    }
    (0, raw.trim_end().to_string())
}
